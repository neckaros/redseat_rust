use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
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
        video_tools::VideoCommandBuilder,
    },
};

pub const HLS_SEGMENT_DURATION: u32 = 6;
pub const HLS_LIST_SIZE: u32 = 10;
pub const INACTIVITY_TIMEOUT_SECS: u64 = 30;
const MAX_FFMPEG_RESTARTS: u32 = 5;
pub const PLAYLIST_READY_TIMEOUT_MS: u64 = 15000;
const FFMPEG_RESTART_DELAY_MS: u64 = 2000;
/// If FFmpeg runs longer than this, consider it a successful run and reset the restart counter
const STABLE_RUN_SECS: u64 = 60;
/// If no new segment is produced for this long, consider FFmpeg stalled and kill it
const OUTPUT_STALL_TIMEOUT_SECS: u64 = 90;
/// If no segment is produced at all within this time after spawn, consider startup failed
const STARTUP_STALL_TIMEOUT_SECS: u64 = 60;
/// How often to check for new segments in the stall detector
const STALL_CHECK_INTERVAL_SECS: u64 = 15;
/// Timeout for child.kill() to prevent supervisor from hanging
const KILL_TIMEOUT_SECS: u64 = 5;

pub struct HlsSession {
    pub key: String,
    pub library_id: String,
    pub channel_id: String,
    pub output_dir: PathBuf,
    pub playlist_path: PathBuf,
    pub cancel_token: CancellationToken,
    pub last_active: Arc<AtomicU64>,
    _supervisor_handle: tokio::task::JoinHandle<()>,
}

impl HlsSession {
    pub fn touch(&self) {
        self.last_active.store(get_time().as_secs(), Ordering::Relaxed);
    }

    pub fn is_stale(&self) -> bool {
        let last = self.last_active.load(Ordering::Relaxed);
        get_time().as_secs().saturating_sub(last) > INACTIVITY_TIMEOUT_SECS
    }
}

/// Find the highest segment number in the output directory to continue numbering
async fn find_next_segment_number(output_dir: &PathBuf) -> u32 {
    let mut max_num: u32 = 0;
    if let Ok(mut entries) = tokio::fs::read_dir(output_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(num_str) = name.strip_prefix("seg_").and_then(|s| s.strip_suffix(".ts")) {
                if let Ok(num) = num_str.parse::<u32>() {
                    max_num = max_num.max(num + 1);
                }
            }
        }
    }
    max_num
}

/// Spawn the FFmpeg HLS process
async fn spawn_ffmpeg(
    stream_url: &str,
    output_dir: &PathBuf,
    playlist_path: &PathBuf,
    start_number: u32,
    is_restart: bool,
) -> std::io::Result<tokio::process::Child> {
    let ffmpeg_path = VideoCommandBuilder::get_ffmpeg_path();
    let segment_pattern = output_dir.join("seg_%05d.ts");

    let mut hls_flags = String::from("delete_segments+append_list+temp_file+program_date_time");
    if is_restart {
        // Signal discontinuity so players reset their decoder state on restart
        hls_flags.push_str("+discont_start");
    }

    let mut cmd = Command::new(ffmpeg_path);
    cmd.arg("-y")
        // Reconnection flags for live stream resilience
        .args(["-reconnect", "1"])
        .args(["-reconnect_streamed", "1"])
        .args(["-reconnect_delay_max", "30"])
        .args(["-reconnect_on_network_error", "1"])
        // I/O timeout: 15 seconds
        .args(["-rw_timeout", "15000000"])
        // Regenerate PTS from source and discard corrupt frames
        .args(["-fflags", "+genpts+discardcorrupt"])
        // Handle timestamp overflows from unstable sources
        .args(["-correct_ts_overflow", "1"])
        // Normalize negative timestamps
        .args(["-avoid_negative_ts", "make_zero"])
        // Input
        .args(["-i", stream_url])
        // No re-encoding
        .args(["-c", "copy"])
        // Allow larger muxing queue to prevent A/V desync under jitter
        .args(["-max_muxing_queue_size", "2048"])
        // HLS output format
        .args(["-f", "hls"])
        .args(["-hls_time", &HLS_SEGMENT_DURATION.to_string()])
        .args(["-hls_list_size", &HLS_LIST_SIZE.to_string()])
        .args(["-hls_flags", &hls_flags])
        .args([
            "-hls_segment_filename",
            &segment_pattern.to_string_lossy(),
        ])
        .args(["-hls_allow_cache", "1"])
        .args(["-start_number", &start_number.to_string()])
        .arg(playlist_path.to_string_lossy().as_ref())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    cmd.spawn()
}

/// Kill an FFmpeg child process with a timeout, falling back to start_kill if it hangs
async fn kill_ffmpeg(child: &mut tokio::process::Child, session_key: &str) {
    let kill_result = tokio::time::timeout(
        std::time::Duration::from_secs(KILL_TIMEOUT_SECS),
        child.kill(),
    )
    .await;
    if kill_result.is_err() {
        log_error(
            LogServiceType::Other,
            format!(
                "HLS [{}]: FFmpeg kill timed out after {}s, forcing",
                session_key, KILL_TIMEOUT_SECS
            ),
        );
        let _ = child.start_kill();
    }
}

/// Supervisor loop: manages FFmpeg lifecycle with restart logic
async fn supervisor_loop(
    session_key: String,
    library_id: String,
    channel_id: String,
    stream_url: String,
    output_dir: PathBuf,
    playlist_path: PathBuf,
    cancel_token: CancellationToken,
    hls_sessions: Arc<RwLock<HashMap<String, HlsSession>>>,
    mc: crate::model::ModelController,
) {
    let mut restart_count = 0u32;

    loop {
        let start_number = find_next_segment_number(&output_dir).await;
        let is_restart = restart_count > 0;

        let child = spawn_ffmpeg(&stream_url, &output_dir, &playlist_path, start_number, is_restart).await;

        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                log_error(
                    LogServiceType::Other,
                    format!("HLS [{}]: Failed to spawn FFmpeg: {}", session_key, e),
                );
                break;
            }
        };

        let spawned_at = get_time().as_secs();

        log_info(
            LogServiceType::Other,
            format!(
                "HLS [{}]: FFmpeg started (restart #{})",
                session_key, restart_count
            ),
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
                            format!("HLS [{}] FFmpeg: {}", key, line),
                        );
                    }
                }
            });
        }

        // Stall detector: monitors segment production and fires if FFmpeg stops producing output
        let stall_detector = {
            let output_dir = output_dir.clone();
            async move {
                let mut last_seg_num = start_number;
                let mut last_seg_change = get_time().as_secs();
                let mut first_segment_seen = start_number > 0;

                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(STALL_CHECK_INTERVAL_SECS)).await;
                    let current = find_next_segment_number(&output_dir).await;
                    if current > last_seg_num {
                        last_seg_num = current;
                        last_seg_change = get_time().as_secs();
                        first_segment_seen = true;
                    } else if first_segment_seen
                        && get_time().as_secs().saturating_sub(last_seg_change) > OUTPUT_STALL_TIMEOUT_SECS
                    {
                        break;
                    } else if !first_segment_seen
                        && get_time().as_secs().saturating_sub(last_seg_change) > STARTUP_STALL_TIMEOUT_SECS
                    {
                        break;
                    }
                }
            }
        };

        // Wait for FFmpeg to exit, cancellation, or stall detection
        tokio::select! {
            status = child.wait() => {
                if cancel_token.is_cancelled() {
                    break;
                }
                let exit_info = match status {
                    Ok(s) => format!("exit code: {:?}", s.code()),
                    Err(e) => format!("error: {}", e),
                };

                let ran_for = get_time().as_secs().saturating_sub(spawned_at);

                if ran_for >= STABLE_RUN_SECS {
                    // FFmpeg ran long enough — this is a transient failure, not a crash loop
                    restart_count = 0;
                    log_info(
                        LogServiceType::Other,
                        format!(
                            "HLS [{}]: FFmpeg exited after {}s ({}), restarting (counter reset)",
                            session_key, ran_for, exit_info
                        ),
                    );
                } else {
                    restart_count += 1;
                    log_error(
                        LogServiceType::Other,
                        format!(
                            "HLS [{}]: FFmpeg exited after {}s ({}) — rapid failure {}/{}",
                            session_key, ran_for, exit_info, restart_count, MAX_FFMPEG_RESTARTS
                        ),
                    );
                    if restart_count >= MAX_FFMPEG_RESTARTS {
                        log_error(
                            LogServiceType::Other,
                            format!("HLS [{}]: Max rapid restarts ({}) exceeded, giving up", session_key, MAX_FFMPEG_RESTARTS),
                        );
                        break;
                    }
                }

                tokio::time::sleep(std::time::Duration::from_millis(FFMPEG_RESTART_DELAY_MS)).await;
            }
            _ = cancel_token.cancelled() => {
                kill_ffmpeg(&mut child, &session_key).await;
                break;
            }
            _ = stall_detector => {
                log_info(
                    LogServiceType::Other,
                    format!(
                        "HLS [{}]: No new segments produced, killing stalled FFmpeg",
                        session_key
                    ),
                );
                kill_ffmpeg(&mut child, &session_key).await;
                break;
            }
        }
    }

    // Cleanup: remove temp directory, session from map, and release stream slot
    let _ = tokio::fs::remove_dir_all(&output_dir).await;
    {
        let mut sessions = hls_sessions.write().await;
        sessions.remove(&session_key);
    }
    mc.release_stream_slot(&library_id, &channel_id).await;
    log_info(
        LogServiceType::Other,
        format!("HLS [{}]: Session cleaned up", session_key),
    );
}

/// Create a new HLS session and start the supervisor
pub async fn create_session(
    key: String,
    library_id: String,
    channel_id: String,
    stream_url: String,
    hls_sessions: Arc<RwLock<HashMap<String, HlsSession>>>,
    mc: crate::model::ModelController,
) -> crate::error::RsResult<(PathBuf, PathBuf)> {
    let dir_name = format!("hls_{}", nanoid::nanoid!());
    let output_dir = get_server_folder_path_array(vec![".cache", &dir_name]).await?;
    let playlist_path = output_dir.join("playlist.m3u8");

    let cancel_token = CancellationToken::new();
    let last_active = Arc::new(AtomicU64::new(get_time().as_secs()));

    let supervisor_handle = tokio::spawn(supervisor_loop(
        key.clone(),
        library_id.clone(),
        channel_id.clone(),
        stream_url.clone(),
        output_dir.clone(),
        playlist_path.clone(),
        cancel_token.clone(),
        hls_sessions.clone(),
        mc,
    ));

    let session = HlsSession {
        key: key.clone(),
        library_id,
        channel_id,
        output_dir: output_dir.clone(),
        playlist_path: playlist_path.clone(),
        cancel_token,
        last_active,
        _supervisor_handle: supervisor_handle,
    };

    {
        let mut sessions = hls_sessions.write().await;
        sessions.insert(key, session);
    }

    Ok((output_dir, playlist_path))
}

/// Stop a session by key
pub async fn stop_session(key: &str, hls_sessions: &Arc<RwLock<HashMap<String, HlsSession>>>) {
    let session = {
        let sessions = hls_sessions.read().await;
        sessions.get(key).map(|s| s.cancel_token.clone())
    };
    if let Some(token) = session {
        token.cancel();
    }
}

/// Clean up stale sessions (called periodically)
pub async fn cleanup_stale_sessions(hls_sessions: &Arc<RwLock<HashMap<String, HlsSession>>>) -> Vec<(String, String)> {
    let stale_keys: Vec<(String, String, String)> = {
        let sessions = hls_sessions.read().await;
        sessions
            .values()
            .filter(|s| s.is_stale())
            .map(|s| (s.key.clone(), s.library_id.clone(), s.channel_id.clone()))
            .collect()
    };

    let mut released = Vec::new();
    for (key, library_id, channel_id) in &stale_keys {
        log_info(
            LogServiceType::Other,
            format!("HLS [{}]: Cleaning up stale session", key),
        );
        stop_session(key, hls_sessions).await;
        released.push((library_id.clone(), channel_id.clone()));
    }
    released
}

/// Clean up orphaned HLS directories from previous crashes
pub async fn cleanup_orphaned_dirs() {
    if let Ok(cache_dir) = get_server_folder_path_array(vec![".cache"]).await {
        if let Ok(mut entries) = tokio::fs::read_dir(&cache_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("hls_") {
                    if let Ok(file_type) = entry.file_type().await {
                        if file_type.is_dir() {
                            log_info(
                                LogServiceType::Other,
                                format!("Cleaning up orphaned HLS directory: {}", name),
                            );
                            let _ = tokio::fs::remove_dir_all(entry.path()).await;
                        }
                    }
                }
            }
        }
    }
}
