use std::{
    collections::{HashMap, HashSet},
    io::Cursor,
    time::Duration,
};

use nanoid::nanoid;
use rs_plugin_common_interfaces::ImageType;
use serde::{Deserialize, Serialize};

use crate::{
    domain::{
        channel::{
            Channel, ChannelForUpdate, ChannelMessage, ChannelVariant, ChannelWithAction,
            M3uImportResult,
        },
        library::{LibraryRole, LibraryType},
        ElementAction,
    },
    error::RsResult,
    plugins::sources::{AsyncReadPinBox, FileStreamResult},
    routes::sse::SseEvent,
    tools::{
        image_tools::{convert_image_reader, ImageSize},
        log::{log_error, log_info, LogServiceType},
        m3u_parser::{self, M3uContentType, M3uEntry, QUALITY_ORDER},
        stream_slots::ProxySlot,
    },
};

use super::{
    entity_images::EntityImageConfig, store::sql::library::SqliteLibraryStore, tags::TagForAdd,
    users::ConnectedUser, ModelController,
};

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChannelQuery {
    #[serde(alias = "groupTag")]
    pub tag: Option<String>,
    pub name: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequest {
    pub url: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StreamQuery {
    pub quality: Option<String>,
}

/// How long a request waits for a stream slot: closing the previous stream of
/// a viewer who changes channel takes a moment.
const STREAM_SLOT_WAIT: Duration = Duration::from_secs(3);
const STREAM_SLOT_POLL: Duration = Duration::from_millis(100);

impl ModelController {
    pub async fn get_channels(
        &self,
        library_id: &str,
        query: ChannelQuery,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Vec<Channel>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id)?;
        let mut channels = store
            .get_channels(query.tag, query.name, query.limit, query.offset)
            .await?;

        let channel_ids = channels.iter().map(|channel| channel.id.clone()).collect();
        let mut variants_by_channel: HashMap<String, Vec<ChannelVariant>> = HashMap::new();
        for variant in store.get_channel_variants_for_channels(channel_ids).await? {
            variants_by_channel
                .entry(variant.channel_ref.clone())
                .or_default()
                .push(variant);
        }
        for channel in &mut channels {
            channel.variants = variants_by_channel.remove(&channel.id);
        }

        Ok(channels)
    }

    pub async fn get_channel(
        &self,
        library_id: &str,
        channel_id: &str,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Channel> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id)?;
        let mut channel = store
            .get_channel(channel_id)
            .await?
            .ok_or(crate::Error::NotFound(format!(
                "Channel {} not found",
                channel_id
            )))?;
        let variants = store.get_channel_variants(&channel.id).await?;
        if !variants.is_empty() {
            channel.variants = Some(variants);
        }
        Ok(channel)
    }

    pub async fn remove_channel(
        &self,
        library_id: &str,
        channel_id: &str,
        requesting_user: &ConnectedUser,
    ) -> RsResult<()> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        let store = self.store.get_library_store(library_id)?;
        let existing = store.get_channel(channel_id).await?;
        store.remove_channel(channel_id.to_string()).await?;
        if let Some(channel) = existing {
            self.broadcast_sse(SseEvent::Channels(ChannelMessage {
                library: library_id.to_string(),
                channels: vec![ChannelWithAction {
                    action: ElementAction::Deleted,
                    channel,
                }],
            }));
        }
        Ok(())
    }

    pub async fn get_channel_stream_url(
        &self,
        library_id: &str,
        channel_id: &str,
        quality: Option<String>,
        requesting_user: &ConnectedUser,
    ) -> RsResult<String> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id)?;
        let variants = store.get_channel_variants(channel_id).await?;

        if variants.is_empty() {
            return Err(crate::Error::NotFound(format!(
                "No variants for channel {}",
                channel_id
            )));
        }

        // If quality requested, try to find that variant
        if let Some(ref q) = quality {
            if let Some(variant) = variants
                .iter()
                .find(|v| v.quality.as_deref() == Some(q.as_str()))
            {
                return Ok(variant.stream_url.clone());
            }
        }

        // Otherwise pick best quality: 4K > FHD > HD > SD > first
        for q in QUALITY_ORDER {
            if let Some(variant) = variants.iter().find(|v| v.quality.as_deref() == Some(*q)) {
                return Ok(variant.stream_url.clone());
            }
        }

        // Fallback to first variant
        Ok(variants[0].stream_url.clone())
    }

    pub async fn import_m3u(
        &self,
        library_id: &str,
        url_override: Option<String>,
        requesting_user: &ConnectedUser,
    ) -> RsResult<M3uImportResult> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;

        let library =
            self.get_library(library_id, requesting_user)
                .await?
                .ok_or(crate::Error::NotFound(format!(
                    "Library {} not found",
                    library_id
                )))?;
        if library.kind != LibraryType::Iptv {
            return Err(crate::Error::Error(
                "M3U import is only available for IPTV libraries".to_string(),
            ));
        }

        // The library's `root` field stores the M3U URL for IPTV libraries
        let m3u_url = url_override
            .or(library.root.clone())
            .ok_or(crate::Error::Error(
            "No M3U URL configured for this library. Set the library root to the M3U playlist URL."
                .to_string(),
        ))?;

        // Send status
        self.broadcast_sse(SseEvent::LibraryStatus(
            crate::domain::library::LibraryStatusMessage {
                message: "Fetching M3U playlist...".to_string(),
                library: library_id.to_string(),
            },
        ));

        log_info(
            LogServiceType::Source,
            format!("Fetching M3U from: {}", m3u_url),
        );

        let store = self.store.get_library_store(library_id)?;

        // Fetch M3U content. The errors leave out the URL: it holds the
        // credentials of the provider account.
        let response = reqwest::get(&m3u_url).await.map_err(|e| {
            self.refuse_m3u_refresh(
                library_id,
                format!("Failed to fetch M3U: {}", e.without_url()),
                "",
            )
        })?;
        let status = response.status();
        // The body of an error answer is not read: the status refuses the refresh.
        let content = if status.is_success() {
            response.text().await.map_err(|e| {
                self.refuse_m3u_refresh(
                    library_id,
                    format!("Failed to read M3U content: {}", e.without_url()),
                    "",
                )
            })?
        } else {
            String::new()
        };

        // Refuse before any change: an error page, an empty answer or a playlist
        // without channels would otherwise delete every channel of the library.
        let existing_channels = store.get_all_channel_ids().await?.len();
        let playlist = check_m3u_playlist(status, &content, existing_channels)
            .map_err(|reason| self.refuse_m3u_refresh(library_id, reason, &content))?;

        log_info(
            LogServiceType::Source,
            format!("Parsed {} M3U entries", playlist.total_parsed),
        );
        log_info(
            LogServiceType::Source,
            format!(
                "Classified: {} live, {} VOD, {} series",
                playlist.live_entries.len(),
                playlist.vod_count,
                playlist.series_count
            ),
        );

        self.broadcast_sse(SseEvent::LibraryStatus(
            crate::domain::library::LibraryStatusMessage {
                message: format!("Parsed {} entries, importing...", playlist.total_parsed),
                library: library_id.to_string(),
            },
        ));

        // Pre-create tags for all unique group-titles
        let mut tag_cache: HashMap<String, String> = HashMap::new(); // group_title -> tag_id
        let mut unique_groups: Vec<String> = playlist
            .live_entries
            .iter()
            .filter_map(|e| e.group_title.clone())
            .collect();
        unique_groups.sort();
        unique_groups.dedup();

        for group_name in &unique_groups {
            let tag = self
                .get_or_create_path(
                    library_id,
                    vec!["iptv", group_name],
                    TagForAdd {
                        name: group_name.clone(),
                        generated: true,
                        ..Default::default()
                    },
                    requesting_user,
                )
                .await?;
            tag_cache.insert(group_name.clone(), tag.id);
        }

        let mut result = M3uImportResult {
            total_parsed: playlist.total_parsed,
            groups_created: unique_groups.len(),
            ..Default::default()
        };

        // --- Import Live Channels ---
        let (channel_actions, channel_count) =
            sync_m3u_channels(&store, &playlist.live_entries, &tag_cache, &mut result).await?;

        // Emit SSE for all channel changes
        if !channel_actions.is_empty() {
            self.broadcast_sse(SseEvent::Channels(ChannelMessage {
                library: library_id.to_string(),
                channels: channel_actions,
            }));
        }

        // --- Import VOD Movies ---
        // TODO: Import VOD movies using existing movies + medias tables

        // --- Import Series ---
        // TODO: Import series using existing series + episodes + medias tables

        self.broadcast_sse(SseEvent::LibraryStatus(
            crate::domain::library::LibraryStatusMessage {
                message: format!(
                    "Import complete: {} channels ({} new, {} updated, {} removed), {} groups",
                    channel_count,
                    result.channels_added,
                    result.channels_updated,
                    result.channels_removed,
                    result.groups_created
                ),
                library: library_id.to_string(),
            },
        ));

        log_info(
            LogServiceType::Source,
            format!("IPTV import for library {}: {:?}", library_id, result),
        );

        Ok(result)
    }

    /// Logs and broadcasts why a playlist refresh changed nothing, and gives the
    /// error to return.
    fn refuse_m3u_refresh(&self, library_id: &str, reason: String, content: &str) -> crate::Error {
        let message = format!("M3U refresh refused, channels kept: {}", reason);
        log_error(
            LogServiceType::Source,
            format!("IPTV library {}: {}", library_id, message),
        );
        if !content.trim().is_empty() {
            log_info(
                LogServiceType::Source,
                format!(
                    "Refused M3U content starts with: {}",
                    content_excerpt(content)
                ),
            );
        }
        self.broadcast_sse(SseEvent::LibraryStatus(
            crate::domain::library::LibraryStatusMessage {
                message: message.clone(),
                library: library_id.to_string(),
            },
        ));
        crate::Error::Error(message)
    }

    // -- Channel tag management --

    pub async fn add_channel_tag(
        &self,
        library_id: &str,
        channel_id: &str,
        tag_id: &str,
        requesting_user: &ConnectedUser,
    ) -> RsResult<()> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let store = self.store.get_library_store(library_id)?;
        // confidence = NULL means user-assigned
        store.add_channel_tag(channel_id, tag_id, None).await?;

        let channel = self
            .get_channel(library_id, channel_id, requesting_user)
            .await?;
        self.broadcast_sse(SseEvent::Channels(ChannelMessage {
            library: library_id.to_string(),
            channels: vec![ChannelWithAction {
                action: ElementAction::Updated,
                channel,
            }],
        }));
        Ok(())
    }

    pub async fn remove_channel_tag(
        &self,
        library_id: &str,
        channel_id: &str,
        tag_id: &str,
        requesting_user: &ConnectedUser,
    ) -> RsResult<()> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let store = self.store.get_library_store(library_id)?;
        store.remove_channel_tag(channel_id, tag_id).await?;

        let channel = self
            .get_channel(library_id, channel_id, requesting_user)
            .await?;
        self.broadcast_sse(SseEvent::Channels(ChannelMessage {
            library: library_id.to_string(),
            channels: vec![ChannelWithAction {
                action: ElementAction::Updated,
                channel,
            }],
        }));
        Ok(())
    }

    pub async fn channel_image(
        &self,
        library_id: &str,
        channel_id: &str,
        kind: Option<ImageType>,
        size: Option<ImageSize>,
        requesting_user: &ConnectedUser,
    ) -> crate::Result<FileStreamResult<AsyncReadPinBox>> {
        let kind = kind.unwrap_or(ImageType::Poster);
        let config = EntityImageConfig {
            folder: ".channels",
            cache_prefix: "channel",
        };
        self.serve_local_entity_image(
            library_id,
            channel_id,
            &kind,
            size,
            &config,
            requesting_user,
            self.refresh_channel_image(library_id, channel_id, &kind, requesting_user),
        )
        .await
    }

    pub async fn update_channel_image(
        &self,
        library_id: &str,
        channel_id: &str,
        kind: &ImageType,
        reader: AsyncReadPinBox,
        requesting_user: &ConnectedUser,
    ) -> RsResult<()> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;

        let converted =
            convert_image_reader(reader, image::ImageFormat::Avif, Some(60), false).await?;
        let converted_reader = Cursor::new(converted);

        self.update_library_image(
            library_id,
            ".channels",
            channel_id,
            &Some(kind.clone()),
            &None,
            converted_reader,
            requesting_user,
        )
        .await?;

        let store = self.store.get_library_store(library_id)?;
        store
            .update_channel_image(channel_id.to_string(), kind.clone())
            .await;

        let channel = self
            .get_channel(library_id, channel_id, requesting_user)
            .await?;
        self.broadcast_sse(SseEvent::Channels(ChannelMessage {
            library: library_id.to_string(),
            channels: vec![ChannelWithAction {
                action: ElementAction::Updated,
                channel,
            }],
        }));

        Ok(())
    }

    pub async fn refresh_channel_image(
        &self,
        library_id: &str,
        channel_id: &str,
        kind: &ImageType,
        requesting_user: &ConnectedUser,
    ) -> RsResult<()> {
        let channel = self
            .get_channel(library_id, channel_id, requesting_user)
            .await?;
        let logo_url = channel.logo.ok_or(crate::Error::NotFound(format!(
            "Channel {} has no logo URL",
            channel_id
        )))?;

        let response = reqwest::get(&logo_url)
            .await
            .map_err(|e| crate::Error::Error(format!("Failed to fetch channel logo: {}", e)))?;

        if !response.status().is_success() {
            return Err(crate::Error::Error(format!(
                "Failed to fetch channel logo for {}: HTTP {}",
                channel_id,
                response.status()
            )));
        }

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !content_type.is_empty() && !content_type.starts_with("image/") {
            return Err(crate::Error::Error(format!(
                "Channel logo for {} is not an image: content-type {}",
                channel_id, content_type
            )));
        }

        let bytes = response.bytes().await.map_err(|e| {
            crate::Error::Error(format!("Failed to read channel logo bytes: {}", e))
        })?;

        if bytes.is_empty() {
            return Err(crate::Error::Error(format!(
                "Channel logo for {} returned empty response",
                channel_id
            )));
        }

        let reader: AsyncReadPinBox = Box::pin(Cursor::new(bytes.to_vec()));
        self.update_channel_image(
            library_id,
            channel_id,
            kind,
            reader,
            &ConnectedUser::ServerAdmin,
        )
        .await?;

        Ok(())
    }

    // -- Stream slot management (concurrent stream limiting) --
    //
    // A library's slots are shared by its HLS sessions (`active_streams`) and its
    // MPEG2-TS proxy streams (`proxy_streams`). When they are all taken, the
    // newest request wins: a viewer changing channel or quality asks for the new
    // stream before the server notices that the previous one was closed, and the
    // provider usually accepts a single connection.

    async fn library_max_streams(&self, library_id: &str) -> RsResult<usize> {
        let library = self
            .cache_get_library(library_id)
            .await
            .ok_or(crate::Error::NotFound(format!(
                "Library {} not found",
                library_id
            )))?;
        Ok(library.settings.max_streams.unwrap_or(1) as usize)
    }

    fn stream_limit_reached(max_streams: usize) -> crate::Error {
        crate::Error::Error(format!(
            "Maximum concurrent streams reached for this library ({}). Stop another stream first.",
            max_streams
        ))
    }

    /// Takes one of the library's stream slots for an HLS session.
    pub async fn acquire_stream_slot(&self, library_id: &str, channel_id: &str) -> RsResult<()> {
        let max_streams = self.library_max_streams(library_id).await?;
        let deadline = tokio::time::Instant::now() + STREAM_SLOT_WAIT;
        loop {
            if self
                .try_acquire_stream_slot(library_id, channel_id, max_streams)
                .await
            {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(Self::stream_limit_reached(max_streams));
            }
            tokio::time::sleep(STREAM_SLOT_POLL).await;
        }
    }

    /// Returns false while the slots are taken by MPEG2-TS proxy streams: the
    /// oldest is asked to close.
    async fn try_acquire_stream_slot(
        &self,
        library_id: &str,
        channel_id: &str,
        max_streams: usize,
    ) -> bool {
        let _reserving = self.proxy_streams.reserving().await;
        {
            let mut streams = self.active_streams.write().await;
            let channels = streams.entry(library_id.to_string()).or_default();

            // Same channel is allowed (doesn't count as extra)
            if channels.contains(channel_id) {
                return true;
            }

            if channels.len() + self.proxy_streams.count(library_id) < max_streams {
                channels.insert(channel_id.to_string());
                return true;
            }
            if channels.is_empty() {
                streams.remove(library_id);
            }
        }

        // A stream already closing frees the slot: wait for it.
        if self.proxy_streams.has_closing(library_id) {
            return false;
        }
        if self.evict_oldest_hls_session(library_id).await {
            let mut streams = self.active_streams.write().await;
            let channels = streams.entry(library_id.to_string()).or_default();
            channels.insert(channel_id.to_string());
            return true;
        }
        self.cancel_oldest_proxy_stream(library_id);
        false
    }

    /// Takes one of the library's stream slots for an MPEG2-TS proxy stream,
    /// held until the returned slot is dropped. A full library closes its
    /// oldest stream for it. Returns whether a stream was closed: the provider
    /// may take a moment to accept the new connection.
    ///
    /// Check the requester's access to the channel first: this may stop
    /// another viewer's stream.
    pub async fn acquire_proxy_stream_slot(
        &self,
        library_id: &str,
        channel_id: &str,
    ) -> RsResult<(ProxySlot, bool)> {
        let max_streams = self.library_max_streams(library_id).await?;
        let deadline = tokio::time::Instant::now() + STREAM_SLOT_WAIT;
        let mut replaced = false;
        loop {
            let released = self.proxy_streams.released();
            {
                // HLS and proxy requests check and take the slots one at a time.
                let _reserving = self.proxy_streams.reserving().await;
                let hls_streams = self
                    .active_streams
                    .read()
                    .await
                    .get(library_id)
                    .map_or(0, HashSet::len);
                if let Some(slot) = self.proxy_streams.try_register(
                    library_id,
                    channel_id,
                    max_streams.saturating_sub(hls_streams),
                ) {
                    return Ok((slot, replaced));
                }

                // A stream already closing frees the slot: wait for it
                // rather than closing another one.
                if !self.proxy_streams.has_closing(library_id) {
                    if self.cancel_oldest_proxy_stream(library_id) {
                        replaced = true;
                    } else if self.evict_oldest_hls_session(library_id).await {
                        replaced = true;
                        continue;
                    }
                }
            }

            if tokio::time::Instant::now() >= deadline {
                return Err(Self::stream_limit_reached(max_streams));
            }
            // A closing proxy stream wakes us when it is gone; an HLS session doesn't.
            tokio::select! {
                _ = released => {}
                _ = tokio::time::sleep(STREAM_SLOT_POLL) => {}
            }
        }
    }

    fn cancel_oldest_proxy_stream(&self, library_id: &str) -> bool {
        let Some(channel_id) = self.proxy_streams.cancel_oldest(library_id) else {
            return false;
        };
        log_info(
            LogServiceType::Source,
            format!(
                "IPTV stream limit reached for library {}, closing the oldest stream (channel {})",
                library_id, channel_id
            ),
        );
        true
    }

    /// Stops the library's oldest HLS session and frees its slot.
    async fn evict_oldest_hls_session(&self, library_id: &str) -> bool {
        let oldest = {
            let sessions = self.hls_sessions.read().await;
            sessions
                .values()
                .filter(|s| s.library_id == library_id)
                .min_by_key(|s| s.last_active.load(std::sync::atomic::Ordering::Relaxed))
                .map(|s| (s.key.clone(), s.channel_id.clone()))
        };

        let Some((key, old_channel_id)) = oldest else {
            return false;
        };

        log_info(
            LogServiceType::Source,
            format!(
                "IPTV stream limit reached for library {}, evicting oldest session (channel {})",
                library_id, old_channel_id
            ),
        );
        crate::tools::hls_session::stop_session(&key, &self.hls_sessions).await;
        let mut streams = self.active_streams.write().await;
        if let Some(channels) = streams.get_mut(library_id) {
            channels.remove(&old_channel_id);
            if channels.is_empty() {
                streams.remove(library_id);
            }
        }
        true
    }

    pub async fn release_stream_slot(&self, library_id: &str, channel_id: &str) {
        let mut streams = self.active_streams.write().await;
        if let Some(channels) = streams.get_mut(library_id) {
            channels.remove(channel_id);
            if channels.is_empty() {
                streams.remove(library_id);
            }
        }
    }

    // -- HLS session management --

    pub async fn get_or_create_hls_session(
        &self,
        library_id: &str,
        channel_id: &str,
        quality: Option<String>,
        requesting_user: &ConnectedUser,
    ) -> RsResult<(std::path::PathBuf, std::path::PathBuf)> {
        let quality_key = quality.as_deref().unwrap_or("best");
        let key = format!("{}:{}:{}", library_id, channel_id, quality_key);

        // Check if session already exists and is alive
        // Note: we intentionally do NOT touch() here — only segment requests
        // should keep the session alive. Otherwise, a client polling the playlist
        // while FFmpeg is stalled (reconnecting) would prevent cleanup forever.
        {
            let sessions = self.hls_sessions.read().await;
            if let Some(session) = sessions.get(&key) {
                return Ok((session.output_dir.clone(), session.playlist_path.clone()));
            }
        }

        // Resolve the stream URL first: it checks the requester's access, and
        // taking a slot may stop another viewer's stream.
        let stream_url = self
            .get_channel_stream_url(library_id, channel_id, quality, requesting_user)
            .await?;

        // Acquire stream slot (enforces max_streams)
        self.acquire_stream_slot(library_id, channel_id).await?;

        // Create the session
        match crate::tools::hls_session::create_session(
            key.clone(),
            library_id.to_string(),
            channel_id.to_string(),
            stream_url,
            self.hls_sessions.clone(),
            self.clone(),
        )
        .await
        {
            Ok(result) => Ok(result),
            Err(e) => {
                // Release slot on failure
                self.release_stream_slot(library_id, channel_id).await;
                Err(e)
            }
        }
    }

    pub async fn stop_hls_session(&self, library_id: &str, channel_id: &str) -> RsResult<()> {
        let keys_to_stop: Vec<String> = {
            let sessions = self.hls_sessions.read().await;
            sessions
                .keys()
                .filter(|k| k.starts_with(&format!("{}:{}:", library_id, channel_id)))
                .cloned()
                .collect()
        };

        for key in &keys_to_stop {
            crate::tools::hls_session::stop_session(key, &self.hls_sessions).await;
        }

        if !keys_to_stop.is_empty() {
            self.release_stream_slot(library_id, channel_id).await;
        }

        Ok(())
    }
}

/// A downloaded M3U playlist accepted for import.
#[derive(Debug)]
struct M3uPlaylist {
    total_parsed: usize,
    /// Live entries with a name: the channels of the library.
    live_entries: Vec<M3uEntry>,
    vod_count: usize,
    series_count: usize,
}

/// Accepts the answer of the provider, or gives why it must not replace the
/// channels of the library. Providers answer an error page, an empty body or an
/// empty playlist when an account expired, a request is rate-limited or another
/// connection is open: importing it would delete every channel.
fn check_m3u_playlist(
    status: reqwest::StatusCode,
    content: &str,
    existing_channels: usize,
) -> Result<M3uPlaylist, String> {
    if !status.is_success() {
        return Err(format!("the provider answered HTTP {}", status));
    }
    let parse_result = m3u_parser::parse_m3u(content);
    if !parse_result.header.has_header && parse_result.entries.is_empty() {
        return Err(if content.trim().is_empty() {
            "the provider sent an empty answer".to_string()
        } else {
            "the answer is not an M3U playlist".to_string()
        });
    }

    let total_parsed = parse_result.entries.len();
    let mut live_entries: Vec<M3uEntry> = Vec::new();
    let mut vod_count = 0;
    let mut series_count = 0;
    for entry in parse_result.entries {
        match entry.content_type() {
            // Many entries have no tvg-id: it only links the EPG.
            M3uContentType::Live => {
                if entry.import_key().is_some() {
                    live_entries.push(entry);
                }
            }
            M3uContentType::Vod => vod_count += 1,
            M3uContentType::Series => series_count += 1,
        }
    }

    if live_entries.is_empty() && existing_channels > 0 {
        return Err(if total_parsed == 0 {
            format!(
                "the playlist has no entries while the library has {} channels",
                existing_channels
            )
        } else {
            format!(
                "the playlist has no live channels ({} entries) while the library has {} channels",
                total_parsed, existing_channels
            )
        });
    }

    Ok(M3uPlaylist {
        total_parsed,
        live_entries,
        vod_count,
        series_count,
    })
}

/// The start of a refused answer, on one line, to tell from the logs what the
/// provider sent.
fn content_excerpt(content: &str) -> String {
    const MAX_CHARS: usize = 200;
    let flat = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut excerpt: String = flat.chars().take(MAX_CHARS).collect();
    if flat.chars().count() > MAX_CHARS {
        excerpt.push('…');
    }
    excerpt
}

/// Makes the channels of the library match the live entries of an accepted
/// playlist: adds, updates and removes channels, their variants and their
/// imported tags. `tag_cache` maps each group-title to its tag id. Returns the
/// changes to broadcast and the number of channels in the playlist.
async fn sync_m3u_channels(
    store: &SqliteLibraryStore,
    live_entries: &[M3uEntry],
    tag_cache: &HashMap<String, String>,
    result: &mut M3uImportResult,
) -> RsResult<(Vec<ChannelWithAction>, usize)> {
    // Group the qualities of each channel. Not by tvg-id: providers leave it
    // empty or give the same one to different channels.
    let mut channel_groups: HashMap<String, Vec<M3uEntry>> = HashMap::new();
    for entry in live_entries {
        if let Some(import_key) = entry.import_key() {
            channel_groups
                .entry(import_key)
                .or_default()
                .push(entry.clone());
        }
    }

    // Track existing channels for removal detection
    let existing_channel_ids: Vec<String> = store.get_all_channel_ids().await?;
    let mut seen_channel_ids: HashSet<String> = HashSet::new();
    let mut channel_actions: Vec<ChannelWithAction> = Vec::new();

    // Upsert channels and variants
    for (channel_key, entries) in &channel_groups {
        // Find representative entry (prefer one with tvg_id and logo)
        let rep = entries
            .iter()
            .find(|e| e.tvg_id.is_some() && e.tvg_logo.is_some())
            .or_else(|| entries.iter().find(|e| e.tvg_logo.is_some()))
            .unwrap_or(&entries[0]);

        let channel_name = rep.channel_key();
        // For the EPG: the first entry of the channel that has one.
        let tvg_id = entries.iter().find_map(|e| e.tvg_id.clone());

        let existing = store.get_channel_by_import_key(channel_key).await?;

        let (channel_id, is_new) = if let Some(existing) = existing {
            // Update if needed
            store
                .update_channel(
                    &existing.id,
                    ChannelForUpdate {
                        name: Some(channel_name.clone()),
                        logo: rep.tvg_logo.clone(),
                        tvg_id: tvg_id.clone(),
                        ..Default::default()
                    },
                )
                .await?;
            result.channels_updated += 1;
            (existing.id, false)
        } else {
            // Create new channel
            let id = nanoid!();
            store
                .add_channel(
                    Channel {
                        id: id.clone(),
                        name: channel_name.clone(),
                        tvg_id: tvg_id.clone(),
                        country: rep.country(),
                        logo: rep.tvg_logo.clone(),
                        tags: None,
                        channel_number: None,
                        posterv: None,
                        modified: None,
                        added: None,
                        variants: None,
                    },
                    Some(channel_key.clone()),
                )
                .await?;
            result.channels_added += 1;
            (id, true)
        };

        seen_channel_ids.insert(channel_id.clone());

        // Diff-based tag sync: collect desired tag IDs from all entries
        let desired_tag_ids: HashSet<String> = entries
            .iter()
            .filter_map(|e| e.group_title.as_ref())
            .filter_map(|gt| tag_cache.get(gt))
            .cloned()
            .collect();

        // Get current auto-imported tags
        let current_auto_tags: HashSet<String> = store
            .get_channel_auto_tag_ids(&channel_id)
            .await?
            .into_iter()
            .collect();

        // Add new auto tags
        for tag_id in desired_tag_ids.difference(&current_auto_tags) {
            store.add_channel_tag(&channel_id, tag_id, Some(0)).await?;
        }

        // Remove stale auto tags
        for tag_id in current_auto_tags.difference(&desired_tag_ids) {
            store.remove_channel_auto_tag(&channel_id, tag_id).await?;
        }

        // Upsert variants by tvg_name
        let mut seen_tvg_names: HashSet<String> = HashSet::new();
        for entry in entries {
            let quality = entry.quality().unwrap_or_else(|| "default".to_string());
            let tvg_name = entry
                .tvg_name
                .as_deref()
                .unwrap_or(&entry.display_name)
                .to_string();
            let variant_name = entry.variant_name();

            let existing_variant = store
                .get_channel_variant_by_tvg_name(&channel_id, &tvg_name)
                .await?;

            let variant_id = existing_variant
                .as_ref()
                .map(|v| v.id.clone())
                .unwrap_or_else(|| nanoid!());
            let needs_update = existing_variant
                .as_ref()
                .map(|v| {
                    v.stream_url != entry.url
                        || v.quality.as_deref() != Some(&quality)
                        || v.name.as_deref() != Some(&variant_name)
                })
                .unwrap_or(true);

            if needs_update {
                store
                    .add_channel_variant(ChannelVariant {
                        id: variant_id,
                        channel_ref: channel_id.clone(),
                        quality: Some(quality),
                        stream_url: entry.url.clone(),
                        name: Some(variant_name),
                        tvg_name: Some(tvg_name.clone()),
                        modified: None,
                        added: None,
                    })
                    .await?;
            }

            seen_tvg_names.insert(tvg_name);
        }

        // Remove stale variants no longer in playlist
        let existing_variants = store.get_channel_variants(&channel_id).await?;
        for variant in &existing_variants {
            if let Some(ref name) = variant.tvg_name {
                if !seen_tvg_names.contains(name) {
                    store.remove_channel_variant(&variant.id).await?;
                }
            }
        }

        // Fetch updated channel for SSE (after tag sync)
        if let Some(updated) = store.get_channel(&channel_id).await? {
            channel_actions.push(ChannelWithAction {
                action: if is_new {
                    ElementAction::Added
                } else {
                    ElementAction::Updated
                },
                channel: updated,
            });
        }
    }

    // Remove channels no longer in M3U
    for old_id in &existing_channel_ids {
        if !seen_channel_ids.contains(old_id) {
            if let Some(removed) = store.get_channel(old_id).await? {
                channel_actions.push(ChannelWithAction {
                    action: ElementAction::Deleted,
                    channel: removed,
                });
            }
            store.remove_channel(old_id.clone()).await?;
            result.channels_removed += 1;
        }
    }

    Ok((channel_actions, seen_channel_ids.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;
    use serde_json::Value;

    const PLAYLIST: &str = r#"#EXTM3U
#EXTINF:-1 tvg-id="TF1.fr" tvg-name="|FR| TF1 HD" tvg-logo="https://logo/tf1.png" group-title="FR TV",|FR| TF1 HD
http://host/user/pass/1
#EXTINF:-1 tvg-id="TF1.fr" tvg-name="|FR| TF1 FHD" tvg-logo="https://logo/tf1.png" group-title="FR TV",|FR| TF1 FHD
http://host/user/pass/2
#EXTINF:-1 tvg-id="France2.fr" tvg-name="|FR| FRANCE 2 HD" tvg-logo="" group-title="FR TV",|FR| FRANCE 2 HD
http://host/user/pass/3
#EXTINF:-1 tvg-id="M6.fr" tvg-name="|FR| M6 HD" tvg-logo="" group-title="FR TV",|FR| M6 HD
http://host/user/pass/4"#;

    async fn library_store() -> SqliteLibraryStore {
        let connection = tokio_rusqlite::Connection::open_in_memory().await.unwrap();
        SqliteLibraryStore::new(connection).await.unwrap()
    }

    /// The steps of `import_m3u` once the playlist is downloaded, with a tag id
    /// per group-title.
    async fn refresh(
        store: &SqliteLibraryStore,
        status: StatusCode,
        content: &str,
    ) -> Result<M3uImportResult, String> {
        let existing_channels = store.get_all_channel_ids().await.unwrap().len();
        let playlist = check_m3u_playlist(status, content, existing_channels)?;
        let tag_cache: HashMap<String, String> = playlist
            .live_entries
            .iter()
            .filter_map(|e| e.group_title.clone())
            .map(|group| (group.clone(), format!("tag-{}", group)))
            .collect();
        let mut result = M3uImportResult::default();
        sync_m3u_channels(store, &playlist.live_entries, &tag_cache, &mut result)
            .await
            .unwrap();
        Ok(result)
    }

    /// Every channel with its variants, ordered by tvg-id.
    async fn snapshot(store: &SqliteLibraryStore) -> Vec<(Value, Value)> {
        let mut channels = store.get_channels(None, None, None, None).await.unwrap();
        channels.sort_by(|a, b| a.tvg_id.cmp(&b.tvg_id));
        let mut snapshot = Vec::new();
        for channel in channels {
            let mut variants = store.get_channel_variants(&channel.id).await.unwrap();
            variants.sort_by(|a, b| a.tvg_name.cmp(&b.tvg_name));
            snapshot.push((
                serde_json::to_value(&channel).unwrap(),
                serde_json::to_value(&variants).unwrap(),
            ));
        }
        snapshot
    }

    async fn tvg_ids(store: &SqliteLibraryStore) -> Vec<String> {
        let mut ids: Vec<String> = store
            .get_channels(None, None, None, None)
            .await
            .unwrap()
            .into_iter()
            .filter_map(|c| c.tvg_id)
            .collect();
        ids.sort();
        ids
    }

    async fn imported_store() -> SqliteLibraryStore {
        let store = library_store().await;
        let result = refresh(&store, StatusCode::OK, PLAYLIST).await.unwrap();
        assert_eq!(result.channels_added, 3);
        store
    }

    #[tokio::test]
    async fn an_http_error_keeps_the_channels() {
        let store = imported_store().await;
        let before = snapshot(&store).await;
        for status in [
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
            StatusCode::NOT_FOUND,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
        ] {
            // Even a valid playlist sent with an error status is refused.
            let reason = refresh(&store, status, PLAYLIST).await.unwrap_err();
            assert!(reason.contains(&status.as_u16().to_string()), "{reason}");
            let reason = refresh(&store, status, "").await.unwrap_err();
            assert!(reason.contains(&status.as_u16().to_string()), "{reason}");
        }
        assert_eq!(snapshot(&store).await, before);
    }

    #[tokio::test]
    async fn an_answer_that_is_not_a_playlist_keeps_the_channels() {
        let store = imported_store().await;
        let before = snapshot(&store).await;
        for content in [
            "",
            "  \n\r\n ",
            "<html><body><h1>Account expired</h1></body></html>",
            r#"{"user_info":{"auth":0,"status":"Expired"}}"#,
            "Max connections reached",
        ] {
            let reason = refresh(&store, StatusCode::OK, content).await.unwrap_err();
            assert!(
                reason.contains("empty answer") || reason.contains("not an M3U playlist"),
                "{reason}"
            );
        }
        assert_eq!(snapshot(&store).await, before);

        // Refused even for an empty library: the answer is not a playlist.
        let empty = library_store().await;
        assert!(refresh(&empty, StatusCode::OK, "<html></html>")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn a_playlist_without_channels_keeps_the_channels() {
        let store = imported_store().await;
        let before = snapshot(&store).await;
        let vod_only = "#EXTM3U\n#EXTINF:-1 tvg-id=\"\" tvg-name=\"Movie FHD 2026\",Movie FHD 2026\nhttp://host/movie/user/pass/9.mkv";
        for content in [
            "#EXTM3U",
            "\u{feff}#EXTM3U url-tvg=\"http://epg\"\n",
            vod_only,
        ] {
            let reason = refresh(&store, StatusCode::OK, content).await.unwrap_err();
            assert!(reason.contains("3 channels"), "{reason}");
        }
        assert_eq!(snapshot(&store).await, before);

        // An empty playlist for an empty library changes nothing and is accepted.
        let empty = library_store().await;
        let result = refresh(&empty, StatusCode::OK, "#EXTM3U\n").await.unwrap();
        assert_eq!(result.channels_added + result.channels_removed, 0);
    }

    #[tokio::test]
    async fn a_valid_playlist_adds_updates_and_removes_channels() {
        let store = imported_store().await;
        assert_eq!(tvg_ids(&store).await, ["France2.fr", "M6.fr", "TF1.fr"]);
        let tf1 = store
            .get_channel_by_tvg_id("TF1.fr")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(store.get_channel_variants(&tf1.id).await.unwrap().len(), 2);

        // M6 left the playlist, Arte joined it, TF1 lost its FHD variant and
        // France 2 got a new stream URL.
        let next = r#"#EXTM3U
#EXTINF:-1 tvg-id="TF1.fr" tvg-name="|FR| TF1 HD" tvg-logo="https://logo/tf1.png" group-title="FR TV",|FR| TF1 HD
http://host/user/pass/1
#EXTINF:-1 tvg-id="France2.fr" tvg-name="|FR| FRANCE 2 HD" tvg-logo="" group-title="FR TV",|FR| FRANCE 2 HD
http://host/user/pass/33
#EXTINF:-1 tvg-id="Arte.fr" tvg-name="|FR| ARTE HD" tvg-logo="" group-title="FR TV",|FR| ARTE HD
http://host/user/pass/5"#;
        let result = refresh(&store, StatusCode::OK, next).await.unwrap();
        assert_eq!(result.channels_added, 1);
        assert_eq!(result.channels_updated, 2);
        assert_eq!(result.channels_removed, 1);
        assert_eq!(tvg_ids(&store).await, ["Arte.fr", "France2.fr", "TF1.fr"]);

        let tf1_after = store
            .get_channel_by_tvg_id("TF1.fr")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(tf1_after.id, tf1.id);
        let tf1_variants = store.get_channel_variants(&tf1.id).await.unwrap();
        assert_eq!(tf1_variants.len(), 1);
        assert_eq!(tf1_variants[0].tvg_name.as_deref(), Some("|FR| TF1 HD"));

        let france2 = store
            .get_channel_by_tvg_id("France2.fr")
            .await
            .unwrap()
            .unwrap();
        let france2_variants = store.get_channel_variants(&france2.id).await.unwrap();
        assert_eq!(france2_variants.len(), 1);
        assert_eq!(france2_variants[0].stream_url, "http://host/user/pass/33");
    }

    /// Providers leave the tvg-id empty or reuse one for different channels:
    /// the channels follow the names of the entries.
    #[tokio::test]
    async fn channels_are_grouped_by_name_not_tvg_id() {
        let playlist = r#"#EXTM3U
#EXTINF:-1 tvg-id="" tvg-name="|FR| DAZN LALIGA 1 HD (LIVE MATCH ONLY)" group-title="FR LALIGA",|FR| DAZN LALIGA 1 HD (LIVE MATCH ONLY)
http://host/user/pass/11
#EXTINF:-1 tvg-id="plutotv_familyclub" tvg-name="|FR| DAZN LALIGA 1 FHD (LIVE MATCH ONLY)" group-title="FR LALIGA",|FR| DAZN LALIGA 1 FHD (LIVE MATCH ONLY)
http://host/user/pass/12
#EXTINF:-1 tvg-id="" tvg-name="|FR| DAZN LALIGA 2 HD (LIVE MATCH ONLY)" group-title="FR LALIGA",|FR| DAZN LALIGA 2 HD (LIVE MATCH ONLY)
http://host/user/pass/21
#EXTINF:-1 tvg-id="" tvg-name="|FR| DAZN LALIGA 2 FHD (LIVE MATCH ONLY)" group-title="FR LALIGA",|FR| DAZN LALIGA 2 FHD (LIVE MATCH ONLY)
http://host/user/pass/22
#EXTINF:-1 tvg-id="PrimeVideoLigue1.fr" tvg-name="|LIGUE 1+| MATCH 1 HD" group-title="LIGUE 1+",|LIGUE 1+| MATCH 1 HD
http://host/user/pass/31
#EXTINF:-1 tvg-id="PrimeVideoLigue1.fr" tvg-name="|LIGUE 1+| MATCH 2 HD" group-title="LIGUE 1+",|LIGUE 1+| MATCH 2 HD
http://host/user/pass/32
#EXTINF:-1 tvg-id="AB3.be" tvg-name="|BE| AB3 FHD" group-title="BE",|BE| AB3 FHD
http://host/user/pass/41
#EXTINF:-1 tvg-id="AB3.be" tvg-name="|FR| AB3 HD" group-title="FR",|FR| AB3 HD
http://host/user/pass/42"#;
        let store = library_store().await;
        let result = refresh(&store, StatusCode::OK, playlist).await.unwrap();
        assert_eq!(result.channels_added, 6);

        let mut channels = store.get_channels(None, None, None, None).await.unwrap();
        channels.sort_by(|a, b| a.name.cmp(&b.name).then(a.country.cmp(&b.country)));
        let names: Vec<&str> = channels.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "AB3",
                "AB3",
                "DAZN LALIGA 1",
                "DAZN LALIGA 2",
                "|LIGUE 1+| MATCH 1",
                "|LIGUE 1+| MATCH 2"
            ]
        );
        for channel in &channels {
            let variants = store.get_channel_variants(&channel.id).await.unwrap();
            let expected = if channel.name.starts_with("DAZN") {
                2
            } else {
                1
            };
            assert_eq!(variants.len(), expected, "{}", channel.name);
        }
        let countries: Vec<Option<&str>> = channels.iter().map(|c| c.country.as_deref()).collect();
        assert_eq!(
            countries,
            [Some("BE"), Some("FR"), Some("FR"), Some("FR"), None, None]
        );
        // The tvg-id of any entry of the channel is kept for the EPG.
        let laliga1 = channels.iter().find(|c| c.name == "DAZN LALIGA 1").unwrap();
        assert_eq!(laliga1.tvg_id.as_deref(), Some("plutotv_familyclub"));

        // A refresh finds the same channels again.
        let before = snapshot(&store).await;
        let result = refresh(&store, StatusCode::OK, playlist).await.unwrap();
        assert_eq!(result.channels_added + result.channels_removed, 0);
        assert_eq!(result.channels_updated, 6);
        let ids = |snapshot: &Vec<(Value, Value)>| -> Vec<String> {
            let mut ids: Vec<String> = snapshot.iter().map(|(c, _)| c["id"].to_string()).collect();
            ids.sort();
            ids
        };
        assert_eq!(ids(&snapshot(&store).await), ids(&before));
    }

    #[test]
    fn refused_content_is_logged_on_one_short_line() {
        assert_eq!(
            content_excerpt("<html>\n  <body>expired</body>\n</html>"),
            "<html> <body>expired</body> </html>"
        );
        let long = "x".repeat(500);
        assert_eq!(content_excerpt(&long).chars().count(), 201);
    }
}
