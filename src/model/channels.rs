use std::{collections::HashMap, io::Cursor};

use nanoid::nanoid;
use rs_plugin_common_interfaces::ImageType;
use serde::{Deserialize, Serialize};

use crate::{
    domain::{
        channel::{Channel, ChannelForUpdate, ChannelVariant, ChannelMessage, ChannelWithAction, M3uImportResult},
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
    },
};

use super::{entity_images::EntityImageConfig, tags::TagForAdd, users::ConnectedUser, ModelController};

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChannelQuery {
    pub group_tag: Option<String>,
    pub name: Option<String>,
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

impl ModelController {
    pub async fn get_channels(
        &self,
        library_id: &str,
        query: ChannelQuery,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Vec<Channel>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id)?;
        let mut channels = store.get_channels(query.group_tag, query.name).await?;

        // Attach variants to each channel
        for channel in &mut channels {
            let variants = store.get_channel_variants(&channel.id).await?;
            if !variants.is_empty() {
                channel.variants = Some(variants);
            }
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
            .ok_or(crate::Error::NotFound(format!("Channel {} not found", channel_id)))?;
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
            if let Some(variant) = variants.iter().find(|v| v.quality.as_deref() == Some(q.as_str())) {
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

        let library = self
            .get_library(library_id, requesting_user)
            .await?
            .ok_or(crate::Error::NotFound(format!("Library {} not found", library_id)))?;
        if library.kind != LibraryType::Iptv {
            return Err(crate::Error::Error(
                "M3U import is only available for IPTV libraries".to_string(),
            ));
        }

        // The library's `root` field stores the M3U URL for IPTV libraries
        let m3u_url = url_override
            .or(library.root.clone())
            .ok_or(crate::Error::Error(
                "No M3U URL configured for this library. Set the library root to the M3U playlist URL.".to_string(),
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

        // Fetch M3U content
        let response = reqwest::get(&m3u_url)
            .await
            .map_err(|e| crate::Error::Error(format!("Failed to fetch M3U: {}", e)))?;
        let content = response
            .text()
            .await
            .map_err(|e| crate::Error::Error(format!("Failed to read M3U content: {}", e)))?;

        // Parse
        let parse_result = m3u_parser::parse_m3u(&content);
        let total_parsed = parse_result.entries.len();

        log_info(
            LogServiceType::Source,
            format!("Parsed {} M3U entries", total_parsed),
        );

        self.broadcast_sse(SseEvent::LibraryStatus(
            crate::domain::library::LibraryStatusMessage {
                message: format!("Parsed {} entries, importing...", total_parsed),
                library: library_id.to_string(),
            },
        ));

        // Classify entries
        let mut live_entries: Vec<M3uEntry> = Vec::new();
        let mut _vod_entries: Vec<M3uEntry> = Vec::new();
        let mut _series_entries: Vec<M3uEntry> = Vec::new();

        for entry in parse_result.entries {
            if entry.is_separator() {
                continue;
            }
            match entry.content_type() {
                M3uContentType::Live => live_entries.push(entry),
                M3uContentType::Vod => _vod_entries.push(entry),
                M3uContentType::Series => _series_entries.push(entry),
            }
        }

        log_info(
            LogServiceType::Source,
            format!(
                "Classified: {} live, {} VOD, {} series",
                live_entries.len(),
                _vod_entries.len(),
                _series_entries.len()
            ),
        );

        let mut result = M3uImportResult {
            total_parsed,
            ..Default::default()
        };

        // --- Import Live Channels ---
        let store = self.store.get_library_store(library_id)?;

        // Group live entries by channel identity
        let mut channel_groups: HashMap<String, Vec<M3uEntry>> = HashMap::new();
        for entry in &live_entries {
            let key = if let Some(ref tvg_id) = entry.tvg_id {
                tvg_id.clone()
            } else {
                entry.channel_key()
            };
            channel_groups.entry(key).or_default().push(entry.clone());
        }

        // Pre-create tags for all unique group-titles
        let mut tag_cache: HashMap<String, String> = HashMap::new(); // group_title -> tag_id
        let mut unique_groups: Vec<String> = live_entries
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
        result.groups_created = unique_groups.len();

        // Track existing channels for removal detection
        let existing_channel_ids: Vec<String> = store.get_all_channel_ids().await?;
        let mut seen_channel_ids: Vec<String> = Vec::new();
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
            let group_tag_id = rep.group_title.as_ref().and_then(|gt| tag_cache.get(gt)).cloned();

            // Try to find existing channel
            let existing = if let Some(ref tvg_id) = rep.tvg_id {
                store.get_channel_by_tvg_id(tvg_id).await?
            } else {
                store.get_channel_by_name(&channel_name).await?
            };

            let channel_id = if let Some(existing) = existing {
                // Update if needed
                store
                    .update_channel(
                        &existing.id,
                        ChannelForUpdate {
                            name: Some(channel_name.clone()),
                            logo: rep.tvg_logo.clone(),
                            group_tag: group_tag_id,
                            tvg_id: rep.tvg_id.clone(),
                            ..Default::default()
                        },
                    )
                    .await?;
                result.channels_updated += 1;
                if let Some(updated) = store.get_channel(&existing.id).await? {
                    channel_actions.push(ChannelWithAction {
                        action: ElementAction::Updated,
                        channel: updated,
                    });
                }
                existing.id
            } else {
                // Create new channel
                let id = nanoid!();
                store
                    .add_channel(Channel {
                        id: id.clone(),
                        name: channel_name.clone(),
                        tvg_id: rep.tvg_id.clone(),
                        logo: rep.tvg_logo.clone(),
                        group_tag: group_tag_id,
                        channel_number: None,
                        posterv: None,
                        modified: None,
                        added: None,
                        variants: None,
                    })
                    .await?;
                result.channels_added += 1;
                if let Some(added) = store.get_channel(&id).await? {
                    channel_actions.push(ChannelWithAction {
                        action: ElementAction::Added,
                        channel: added,
                    });
                }
                id
            };

            seen_channel_ids.push(channel_id.clone());

            // Upsert variants
            for entry in entries {
                let quality = entry.quality().unwrap_or_else(|| "default".to_string());

                let existing_variant = store
                    .get_channel_variant_by_quality(&channel_id, &quality)
                    .await?;

                if let Some(existing_variant) = existing_variant {
                    // Update URL if changed
                    if existing_variant.stream_url != entry.url {
                        store
                            .add_channel_variant(ChannelVariant {
                                id: existing_variant.id,
                                channel_ref: channel_id.clone(),
                                quality: Some(quality),
                                stream_url: entry.url.clone(),
                                modified: None,
                                added: None,
                            })
                            .await?;
                    }
                } else {
                    store
                        .add_channel_variant(ChannelVariant {
                            id: nanoid!(),
                            channel_ref: channel_id.clone(),
                            quality: Some(quality),
                            stream_url: entry.url.clone(),
                            modified: None,
                            added: None,
                        })
                        .await?;
                }
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
                    seen_channel_ids.len(),
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
            format!(
                "IPTV import for library {}: {:?}",
                library_id, result
            ),
        );

        Ok(result)
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

        let converted = convert_image_reader(reader, image::ImageFormat::Avif, Some(60), false).await?;
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

        let channel = self.get_channel(library_id, channel_id, requesting_user).await?;
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
        _kind: &ImageType,
        requesting_user: &ConnectedUser,
    ) -> RsResult<()> {
        let channel = self.get_channel(library_id, channel_id, requesting_user).await?;
        let logo_url = channel.logo.ok_or(crate::Error::NotFound(
            format!("Channel {} has no logo URL", channel_id),
        ))?;

        let response = reqwest::get(&logo_url)
            .await
            .map_err(|e| crate::Error::Error(format!("Failed to fetch channel logo: {}", e)))?;
        let bytes = response
            .bytes()
            .await
            .map_err(|e| crate::Error::Error(format!("Failed to read channel logo bytes: {}", e)))?;

        let reader: AsyncReadPinBox = Box::pin(Cursor::new(bytes.to_vec()));
        self.update_channel_image(
            library_id,
            channel_id,
            &ImageType::Poster,
            reader,
            &ConnectedUser::ServerAdmin,
        )
        .await?;

        Ok(())
    }
}
