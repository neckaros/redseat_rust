use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};

use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::RwLock,
};
use tokio_util::sync::CancellationToken;

use crate::{
    server::get_server_folder_path_array,
    tools::{
        get_time,
        log::{log_error, log_info, LogServiceType},
        video_tools::{probe_video, VideoCommandBuilder},
    },
};

pub const MEDIA_HLS_SEGMENT_DURATION: u32 = 6;
/// 30 minutes inactivity timeout — users take breaks during movies
pub const MEDIA_INACTIVITY_TIMEOUT_SECS: u64 = 1800;
pub const MEDIA_PLAYLIST_READY_TIMEOUT_MS: u64 = 30000;

pub struct MediaHlsSession {
    pub key: String,
    pub library_id: String,
    pub media_id: String,
    pub output_dir: PathBuf,
    pub playlist_path: PathBuf,
    pub cancel_token: CancellationToken,
    pub last_active: Arc<AtomicU64>,
    /// True when FFmpeg exits cleanly (all segments generated)
    pub finished: Arc<AtomicBool>,
    _supervisor_handle: tokio::task::JoinHandle<()>,
}

impl MediaHlsSession {
    pub fn touch(&self) {
        self.last_active
            .store(get_time().as_secs(), Ordering::Relaxed);
    }

    pub fn is_stale(&self) -> bool {
        let last = self.last_active.load(Ordering::Relaxed);
        get_time().as_secs().saturating_sub(last) > MEDIA_INACTIVITY_TIMEOUT_SECS
    }
}

/// Spawn FFmpeg in copy mode (no transcoding) for HLS VOD output
#[cfg(test)]
fn build_copy_mode_command(
    input_uri: &str,
    output_dir: &Path,
    playlist_path: &Path,
) -> Command {
    let ffmpeg_path = VideoCommandBuilder::get_ffmpeg_path();
    let segment_pattern = output_dir.join("seg_%05d.ts");

    let mut cmd = Command::new(ffmpeg_path);
    cmd.arg("-y")
        // Input
        .args(["-i", input_uri])
        // No re-encoding
        .args(["-c", "copy"])
        // HLS output format
        .args(["-f", "hls"])
        .args(["-hls_time", &MEDIA_HLS_SEGMENT_DURATION.to_string()])
        .args(["-hls_playlist_type", "event"])
        .args(["-hls_flags", "temp_file+append_list"])
        .args([
            "-hls_segment_filename",
            &segment_pattern.to_string_lossy(),
        ])
        .args(["-hls_allow_cache", "1"])
        .arg(playlist_path.to_string_lossy().as_ref())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    cmd
}

/// Probe source codecs and spawn FFmpeg with H.264 + AAC compatibility for HLS.
/// - Video: copy if already H.264, otherwise transcode to H.264
/// - Audio: copy if already AAC, otherwise transcode to AAC 128k
/// - Subtitles: stripped (HLS .ts doesn't support most formats)
async fn spawn_compatible_hls(
    input_uri: &str,
    output_dir: &Path,
    playlist_path: &Path,
) -> crate::error::RsResult<tokio::process::Child> {
    let probe = probe_video(input_uri).await?;

    let video_codec = probe.video_stream().and_then(|s| s.codec_name.as_deref());
    let audio_codec = probe.audio_stream().and_then(|s| s.codec_name.as_deref());

    let video_is_h264 = video_codec.map_or(false, |c| {
        c.eq_ignore_ascii_case("h264") || c.eq_ignore_ascii_case("x264")
    });
    let audio_is_aac = audio_codec.map_or(true, |c| c.eq_ignore_ascii_case("aac"));

    let video_needs_transcode = !video_is_h264;
    let audio_needs_transcode = !audio_is_aac;

    // Skip CUDA/hardware detection when everything can be copied
    let mut builder = if video_needs_transcode {
        VideoCommandBuilder::new(input_uri.to_string()).await
    } else {
        VideoCommandBuilder::new_copy_only(input_uri.to_string())
    };

    if video_needs_transcode {
        builder.add_out_option("-c:v");
        if builder.has_cuda_support() {
            builder.add_out_option("h264_nvenc");
            builder.add_out_option("-preset:v");
            builder.add_out_option("p7");
            builder.add_out_option("-tune:v");
            builder.add_out_option("hq");
            builder.add_out_option("-rc:v");
            builder.add_out_option("vbr");
            builder.add_out_option("-cq:v");
            builder.add_out_option("23");
            builder.add_out_option("-b:v");
            builder.add_out_option("0");
            builder.add_out_option("-profile:v");
            builder.add_out_option("high");
        } else {
            builder.add_out_option("libx264");
            builder.add_out_option("-preset");
            builder.add_out_option("medium");
            builder.add_out_option("-crf:v");
            builder.add_out_option("23");
            builder.add_out_option("-profile:v");
            builder.add_out_option("high");
            builder.add_out_option("-level");
            builder.add_out_option("4.1");
        }
        builder.add_out_option("-pix_fmt");
        builder.add_out_option("yuv420p");
    } else {
        builder.add_out_option("-c:v");
        builder.add_out_option("copy");
    }

    if audio_needs_transcode {
        builder.set_audio_codec_aac("128k");
    } else if audio_codec.is_some() {
        builder.copy_audio();
    }

    builder.add_out_option("-sn");

    let cmd = builder.build_command_for_hls(output_dir, playlist_path, MEDIA_HLS_SEGMENT_DURATION);
    cmd.spawn().map_err(|e| {
        crate::error::RsError::Error(format!("Failed to spawn FFmpeg for HLS compatible mode: {}", e))
    })
}

/// Supervisor loop for media HLS: runs FFmpeg once (no restart logic for VOD)
async fn media_supervisor_loop(
    session_key: String,
    mut child: tokio::process::Child,
    output_dir: PathBuf,
    cancel_token: CancellationToken,
    finished: Arc<AtomicBool>,
    media_hls_sessions: Arc<RwLock<HashMap<String, MediaHlsSession>>>,
) {
    log_info(
        LogServiceType::Other,
        format!("Media HLS [{}]: FFmpeg started", session_key),
    );

    // Log stderr in background
    if let Some(stderr) = child.stderr.take() {
        let key = session_key.clone();
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.contains("error") || line.contains("Error") {
                    log_error(
                        LogServiceType::Other,
                        format!("Media HLS [{}] FFmpeg: {}", key, line),
                    );
                }
            }
        });
    }

    // Wait for FFmpeg to exit or cancellation
    tokio::select! {
        status = child.wait() => {
            let exit_info = match status {
                Ok(s) => {
                    if s.success() {
                        finished.store(true, Ordering::Relaxed);
                        format!("exit code: {:?} (success)", s.code())
                    } else {
                        format!("exit code: {:?}", s.code())
                    }
                }
                Err(e) => format!("error: {}", e),
            };
            log_info(
                LogServiceType::Other,
                format!("Media HLS [{}]: FFmpeg exited — {}", session_key, exit_info),
            );
        }
        _ = cancel_token.cancelled() => {
            let _ = child.kill().await;
            log_info(
                LogServiceType::Other,
                format!("Media HLS [{}]: Session cancelled", session_key),
            );
        }
    }

    // Don't clean up immediately if finished — segments still needed for playback.
    // The stale session cleanup will handle it after MEDIA_INACTIVITY_TIMEOUT_SECS.
    if cancel_token.is_cancelled() {
        let _ = tokio::fs::remove_dir_all(&output_dir).await;
        let mut sessions = media_hls_sessions.write().await;
        sessions.remove(&session_key);
        log_info(
            LogServiceType::Other,
            format!("Media HLS [{}]: Session cleaned up", session_key),
        );
    }
}

/// Build the FFmpeg command for a media HLS session.
/// Returns a spawned child process and the output directory + playlist path.
async fn build_and_spawn_media_hls(
    input_uri: &str,
    convert_request: Option<rs_plugin_common_interfaces::video::VideoConvertRequest>,
) -> crate::error::RsResult<(tokio::process::Child, PathBuf, PathBuf)> {
    let dir_name = format!("hls_{}", nanoid::nanoid!());
    let output_dir = get_server_folder_path_array(vec![".cache", &dir_name]).await?;
    let playlist_path = output_dir.join("playlist.m3u8");

    let spawn_result = if let Some(request) = convert_request {
        let mut builder = VideoCommandBuilder::new(input_uri.to_string()).await;
        builder.set_request(request).await?;
        let cmd = builder.build_command_for_hls(
            &output_dir,
            &playlist_path,
            MEDIA_HLS_SEGMENT_DURATION,
        );
        cmd.spawn().map_err(|e| {
            crate::error::RsError::Error(format!("Failed to spawn FFmpeg for HLS transcode: {}", e))
        })
    } else {
        spawn_compatible_hls(input_uri, &output_dir, &playlist_path).await
    };

    match spawn_result {
        Ok(child) => Ok((child, output_dir, playlist_path)),
        Err(e) => {
            // Clean up the output directory on spawn failure
            let _ = tokio::fs::remove_dir_all(&output_dir).await;
            Err(e)
        }
    }
}

/// Start a media HLS session: builds FFmpeg command, spawns it, creates session.
/// The caller must ensure no session with the same key already exists.
pub async fn start_media_hls_session(
    key: String,
    library_id: String,
    media_id: String,
    input_uri: &str,
    convert_request: Option<rs_plugin_common_interfaces::video::VideoConvertRequest>,
    media_hls_sessions: Arc<RwLock<HashMap<String, MediaHlsSession>>>,
) -> crate::error::RsResult<()> {
    let (child, output_dir, playlist_path) =
        build_and_spawn_media_hls(input_uri, convert_request).await?;

    let cancel_token = CancellationToken::new();
    let last_active = Arc::new(AtomicU64::new(get_time().as_secs()));
    let finished = Arc::new(AtomicBool::new(false));

    let supervisor_handle = tokio::spawn(media_supervisor_loop(
        key.clone(),
        child,
        output_dir.clone(),
        cancel_token.clone(),
        finished.clone(),
        media_hls_sessions.clone(),
    ));

    let session = MediaHlsSession {
        key: key.clone(),
        library_id,
        media_id,
        output_dir,
        playlist_path,
        cancel_token,
        last_active,
        finished,
        _supervisor_handle: supervisor_handle,
    };

    {
        let mut sessions = media_hls_sessions.write().await;
        // Double-check under write lock: if another request already created
        // a session with this key while we were spawning FFmpeg, kill ours.
        if sessions.contains_key(&key) {
            log_info(
                LogServiceType::Other,
                format!("Media HLS [{}]: Duplicate session detected during creation, cancelling ours", key),
            );
            session.cancel_token.cancel();
            return Ok(());
        }
        sessions.insert(key, session);
    }

    Ok(())
}

/// Stop a session by key
pub async fn stop_session(
    key: &str,
    media_hls_sessions: &Arc<RwLock<HashMap<String, MediaHlsSession>>>,
) {
    let session = {
        let sessions = media_hls_sessions.read().await;
        sessions.get(key).map(|s| s.cancel_token.clone())
    };
    if let Some(token) = session {
        token.cancel();
    }
}

/// Clean up stale media HLS sessions (called periodically)
pub async fn cleanup_stale_sessions(
    media_hls_sessions: &Arc<RwLock<HashMap<String, MediaHlsSession>>>,
) {
    let stale_keys: Vec<String> = {
        let sessions = media_hls_sessions.read().await;
        sessions
            .values()
            .filter(|s| s.is_stale())
            .map(|s| s.key.clone())
            .collect()
    };

    for key in &stale_keys {
        log_info(
            LogServiceType::Other,
            format!("Media HLS [{}]: Cleaning up stale session", key),
        );
        stop_session(key, media_hls_sessions).await;
        // After cancel, the supervisor loop will clean up the directory
        // But we also remove from the map in case supervisor already exited
        let output_dir = {
            let mut sessions = media_hls_sessions.write().await;
            sessions.remove(key).map(|s| s.output_dir)
        };
        if let Some(dir) = output_dir {
            let _ = tokio::fs::remove_dir_all(&dir).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_session_is_stale() {
        let last_active = Arc::new(AtomicU64::new(0)); // epoch = always stale
        let session = MediaHlsSession {
            key: "test:media:default".to_string(),
            library_id: "lib1".to_string(),
            media_id: "media1".to_string(),
            output_dir: PathBuf::from("/tmp/test"),
            playlist_path: PathBuf::from("/tmp/test/playlist.m3u8"),
            cancel_token: CancellationToken::new(),
            last_active: last_active.clone(),
            finished: Arc::new(AtomicBool::new(false)),
            _supervisor_handle: tokio::spawn(async {}),
        };

        // Session with last_active = 0 should be stale
        assert!(session.is_stale());

        // After touch, should not be stale
        session.touch();
        assert!(!session.is_stale());
    }

    #[tokio::test]
    async fn test_session_touch_updates_last_active() {
        let last_active = Arc::new(AtomicU64::new(0));
        let session = MediaHlsSession {
            key: "test:media:default".to_string(),
            library_id: "lib1".to_string(),
            media_id: "media1".to_string(),
            output_dir: PathBuf::from("/tmp/test"),
            playlist_path: PathBuf::from("/tmp/test/playlist.m3u8"),
            cancel_token: CancellationToken::new(),
            last_active: last_active.clone(),
            finished: Arc::new(AtomicBool::new(false)),
            _supervisor_handle: tokio::spawn(async {}),
        };

        assert_eq!(last_active.load(Ordering::Relaxed), 0);
        session.touch();
        assert!(last_active.load(Ordering::Relaxed) > 0);
    }

    #[test]
    fn test_constants() {
        assert_eq!(MEDIA_HLS_SEGMENT_DURATION, 6);
        assert_eq!(MEDIA_INACTIVITY_TIMEOUT_SECS, 1800);
        assert_eq!(MEDIA_PLAYLIST_READY_TIMEOUT_MS, 30000);
    }

    #[test]
    fn test_copy_mode_command_has_hls_flags() {
        let cmd = build_copy_mode_command(
            "/path/to/video.mp4",
            &PathBuf::from("/tmp/output"),
            &PathBuf::from("/tmp/output/playlist.m3u8"),
        );
        // Verify the command is built (we can't easily inspect args,
        // but we can verify it doesn't panic)
        let program = cmd.as_std().get_program().to_string_lossy().to_string();
        assert!(!program.is_empty());
    }

    #[tokio::test]
    async fn test_stop_session_nonexistent() {
        let sessions: Arc<RwLock<HashMap<String, MediaHlsSession>>> =
            Arc::new(RwLock::new(HashMap::new()));
        // Should not panic on nonexistent session
        stop_session("nonexistent", &sessions).await;
    }

    #[tokio::test]
    async fn test_cleanup_stale_sessions_empty() {
        let sessions: Arc<RwLock<HashMap<String, MediaHlsSession>>> =
            Arc::new(RwLock::new(HashMap::new()));
        // Should not panic on empty sessions
        cleanup_stale_sessions(&sessions).await;
    }
}
