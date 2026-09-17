use bytes::Bytes;
use futures::{AsyncRead, Stream};
use serde_json::Value;
use std::{
    any::Any,
    io,
    path::{Path, PathBuf},
    pin::Pin,
    process::Stdio,
    str::from_utf8,
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};
use stream_map_any::StreamMapAny;

use lazy_static::lazy_static;
use nanoid::nanoid;
use rs_plugin_common_interfaces::request::{RsCookie, RsRequest};
use tokio::{
    fs::{self, remove_file, File},
    io::{AsyncWrite, AsyncWriteExt, BufReader},
    process::{Child, ChildStderr, ChildStdout, Command},
    sync::{Mutex, RwLock},
    time::timeout,
};
use tokio_stream::StreamExt;
use tokio_util::io::{ReaderStream, StreamReader};
use youtube_dl::{download_yt_dlp, YoutubeDl};

pub mod ytdl_model;

use crate::{
    domain::progress::{self, RsProgress, RsProgressCallback, RsProgressType},
    error::RsResult,
    plugins::sources::{
        error::SourcesError, AsyncReadPinBox, CleanupFiles, FileStreamResult, SourceRead,
    },
    server::{get_server_file_path_array, get_server_folder_path_array, get_server_temp_file_path},
    tools::{
        file_tools::get_mime_from_filename,
        log::{log_error, log_info},
        video_tools::ytdl::ytdl_model::{Playlist, SingleVideo},
    },
    Error,
};

use self::ytdl_model::YoutubeDlOutput;

const FILE_NAME: &str = if cfg!(target_os = "windows") {
    "yt-dlp.exe"
} else {
    "yt-dlp"
};

const UPDATE_INTERVAL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const UPDATE_RETRY_INTERVAL: Duration = Duration::from_secs(15 * 60);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(2 * 60);

lazy_static! {
    static ref YTDL_EXECUTION_LOCK: Arc<RwLock<()>> = Arc::new(RwLock::new(()));
    static ref YTDL_LAST_UPDATE_ATTEMPT: Mutex<Option<Instant>> = Mutex::new(None);
}

#[derive(Debug, Clone)]
pub struct YydlContext {
    binary_path: PathBuf,
}

impl YydlContext {
    /// Prepare the managed yt-dlp binary without constructing a request context.
    /// Used at server startup so normal requests rarely need to wait for an update.
    pub async fn initialize() -> RsResult<()> {
        Self::ensure_binary().await.map(|_| ())
    }

    pub async fn new() -> RsResult<Self> {
        Ok(Self {
            binary_path: Self::ensure_binary().await?,
        })
    }

    async fn managed_binary_path() -> RsResult<PathBuf> {
        get_server_file_path_array(vec!["tools", FILE_NAME]).await
    }

    fn should_update(modified: Option<SystemTime>, now: SystemTime) -> bool {
        match modified {
            Some(modified) => now
                .duration_since(modified)
                .map(|age| age >= UPDATE_INTERVAL)
                // A future timestamp can happen after a clock correction. Treat it as fresh.
                .unwrap_or(false),
            None => true,
        }
    }

    async fn binary_needs_update(path: &Path) -> bool {
        let modified = match fs::metadata(path).await {
            Ok(metadata) => metadata.modified().ok(),
            Err(_) => None,
        };
        Self::should_update(modified, SystemTime::now())
    }

    async fn ensure_binary() -> RsResult<PathBuf> {
        let target = Self::managed_binary_path().await?;
        let target_exists = fs::metadata(&target).await.is_ok();
        let mut last_attempt = YTDL_LAST_UPDATE_ATTEMPT.lock().await;

        if target_exists
            && last_attempt
                .as_ref()
                .is_some_and(|attempt| attempt.elapsed() < UPDATE_RETRY_INTERVAL)
        {
            return Ok(target);
        }
        if !Self::binary_needs_update(&target).await {
            return Ok(target);
        }

        // Wait for active yt-dlp processes before replacing the executable.
        let _execution_lock = YTDL_EXECUTION_LOCK.write().await;
        if !Self::binary_needs_update(&target).await {
            return Ok(target);
        }

        *last_attempt = Some(Instant::now());
        log_info(
            crate::tools::log::LogServiceType::Other,
            format!("Updating YT-DLP at {:?}", target),
        );

        match Self::download_binary(&target).await {
            Ok(()) => {
                log_info(
                    crate::tools::log::LogServiceType::Other,
                    format!("Updated YT-DLP at {:?}", target),
                );
                Ok(target)
            }
            Err(error) => {
                if fs::metadata(&target).await.is_ok() {
                    log_error(
                        crate::tools::log::LogServiceType::Other,
                        format!(
                            "Unable to update YT-DLP; continuing with {:?}: {}",
                            target, error
                        ),
                    );
                    Ok(target)
                } else {
                    Err(error)
                }
            }
        }
    }

    async fn download_binary(target: &Path) -> RsResult<()> {
        let temporary_name = format!(".{}.{}.download", FILE_NAME, nanoid!());
        let temporary_path = target.with_file_name(temporary_name);
        File::create(&temporary_path).await?;

        let result: RsResult<()> = async {
            match timeout(DOWNLOAD_TIMEOUT, download_yt_dlp(&temporary_path)).await {
                Ok(result) => {
                    result?;
                }
                Err(_) => {
                    return Err(Error::Error(format!(
                        "YT-DLP download timed out after {} seconds",
                        DOWNLOAD_TIMEOUT.as_secs()
                    )));
                }
            }

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;

                let mut permissions = fs::metadata(&temporary_path).await?.permissions();
                permissions.set_mode(0o755);
                fs::set_permissions(&temporary_path, permissions).await?;
            }

            // Windows cannot atomically replace an existing executable with rename.
            // The write lock above guarantees no process managed here is using it.
            #[cfg(windows)]
            if fs::metadata(target).await.is_ok() {
                remove_file(target).await?;
            }

            fs::rename(&temporary_path, target).await?;
            Ok(())
        }
        .await;

        if result.is_err() {
            let _ = remove_file(&temporary_path).await;
        }
        result
    }

    pub async fn request(
        &self,
        request: &RsRequest,
        progress: RsProgressCallback,
    ) -> RsResult<SourceRead> {
        let mut command = YtDlCommandBuilder::new(&request.url, &self.binary_path);
        //let mut process = YoutubeDl::new(request.url.to_owned());
        //process.socket_timeout("15");
        command.set_request(request).await?;
        let output = command.run_with_cache(progress).await?;

        let read = SourceRead::Stream(output);
        Ok(read)
    }

    pub async fn request_infos(&self, request: &RsRequest) -> RsResult<Option<SingleVideo>> {
        let mut command = YtDlCommandBuilder::new(&request.url, &self.binary_path);
        //let mut process = YoutubeDl::new(request.url.to_owned());
        //process.socket_timeout("15");

        if let Some(cookies) = &request.cookies {
            command.set_cookies(cookies).await?;
        }
        if let Some(headers) = &request.headers {
            for header in headers {
                command.add_header(&header.0, &header.1);
            }
        }

        if let Some(referer) = &request.referer {
            command.add_referer(&referer);
        }

        let output = command.infos().await?;

        Ok(output)
    }

    pub async fn download_to(&self, request: &RsRequest) -> RsResult<PathBuf> {
        let _execution_lock = YTDL_EXECUTION_LOCK.read().await;
        let mut process = YoutubeDl::new(request.url.to_owned());
        process.youtube_dl_path(&self.binary_path);
        process.socket_timeout("15");

        let download_path = get_server_folder_path_array(vec![".cache"]).await?;
        let filename = format!("{}.mp4", nanoid!());
        let path = if let Some(cookies) = &request.cookies {
            let p = get_server_temp_file_path().await?;
            let mut file = File::create(&p).await?;
            file.write_all("# Netscape HTTP Cookie File\n".as_bytes())
                .await?;
            for cookie in cookies {
                file.write_all(format!("{}\n", cookie.netscape()).as_bytes())
                    .await?;
            }
            file.flush().await?;
            process.cookies(
                &p.as_os_str()
                    .to_str()
                    .ok_or(Error::Error("unable to parse cookies path".to_owned()))?
                    .to_owned(),
            );
            Some(p)
        } else {
            None
        };

        //args.f = 'bestvideo[ext=mp4]+bestaudio[ext=m4a]/best[ext=mp4]/best';
        process.extra_arg("--merge-output-format");
        process.extra_arg("mp4");
        //process.extra_arg("--postprocessorArgs");
        //process.extra_arg("'-c copy'");
        println!("path: {:?}", path);
        process
            .output_template(&filename)
            .download_to_async(&download_path)
            .await?;
        if let Some(p) = path {
            remove_file(p).await?;
        }

        Ok(download_path)
    }
}

impl RsProgress {
    pub fn from_ytdl(str: &str) -> Option<Self> {
        let mut split = str.split("progress=");
        if let Some(progress_part) = split.nth(1) {
            let mut parts = progress_part.split("-");
            Some(Self {
                id: nanoid!(),
                current: parts.next().and_then(|p| p.parse::<u64>().ok()),
                total: parts
                    .next()
                    .and_then(|p| p.replace("\"", "").parse::<u64>().ok()),
                kind: RsProgressType::Download,
                filename: None,
            })
        } else {
            None
        }
    }
}

pub enum ProgressStreamItem {
    Progress(RsProgress),
    Data(Result<Bytes, io::Error>),
}
pub struct YtDlCommandBuilder {
    cmd: Command,
    cookies_path: Option<PathBuf>,
}

impl YtDlCommandBuilder {
    pub fn new(path: &str, binary_path: &Path) -> Self {
        let mut cmd = Command::new(binary_path);
        cmd.arg(path);
        Self {
            cmd,
            cookies_path: None,
        }
    }

    pub async fn set_request(&mut self, request: &RsRequest) -> RsResult<()> {
        if let Some(cookies) = &request.cookies {
            self.set_cookies(cookies).await?;
        }
        if let Some(headers) = &request.headers {
            for header in headers {
                self.add_header(&header.0, &header.1);
            }
        }

        if let Some(referer) = &request.referer {
            self.add_referer(&referer);
        }
        Ok(())
    }

    pub fn add_referer(&mut self, referer: &str) -> &mut Self {
        self.cmd.arg("--referer").arg(referer);
        //println!("REFERER {}", referer);
        self
    }
    pub fn add_header(&mut self, name: &str, value: &str) -> &mut Self {
        self.cmd
            .arg("--add-headers")
            .arg(format!("{}:{}", name, value));
        self
    }
    /// Ex: Path to cookies file in netscape format
    pub async fn set_cookies(&mut self, cookies: &Vec<RsCookie>) -> RsResult<&mut Self> {
        let p = get_server_temp_file_path().await?;
        let mut file = File::create(&p).await?;
        file.write_all("# Netscape HTTP Cookie File\n".as_bytes())
            .await?;
        for cookie in cookies {
            file.write_all(format!("{}\n", cookie.netscape()).as_bytes())
                .await?;
        }
        file.flush().await?;

        self.cmd.arg("--cookies").arg(&p);
        self.cookies_path = Some(p);
        Ok(self)
    }

    pub async fn run_with_cache(
        &mut self,
        progress: RsProgressCallback,
    ) -> RsResult<FileStreamResult<AsyncReadPinBox>> {
        let _execution_lock = YTDL_EXECUTION_LOCK.read().await;
        let temp_path = get_server_temp_file_path().await?;
        let fileroot = nanoid!();
        self.cmd
            .arg("--write-info-json")
            .arg("-f")
            //.arg("best/bestvideo+bestaudio")
            .arg("bestvideo+bestaudio/best")
            .arg("--merge-output-format")
            .arg("mp4")
            .arg("--remux-video")
            .arg("mp4")
            .arg("--progress-template")
            .arg("\"download:progress=%(progress.downloaded_bytes)s-%(progress.total_bytes)s\"")
            .arg("-P")
            .arg(&temp_path)
            .arg("-o")
            .arg(format!("{}.%(ext)s", fileroot))
            .stdout(Stdio::piped());
        //.stderr(Stdio::piped());
        let mut child = self.cmd.spawn()?;

        let mut out = ReaderStream::new(child.stdout.take().unwrap()).filter_map(|f| {
            let r = f
                .ok()
                .and_then(|b| from_utf8(&b).ok().and_then(RsProgress::from_ytdl));
            r
        });
        if let Some(progress) = progress {
            println!("Progress==");
            while let Some(p) = &mut out.next().await {
                progress.send(p.to_owned()).await.unwrap();
            }
        }

        let r = child.wait().await;
        if let Err(error) = r {
            log_error(
                crate::tools::log::LogServiceType::Plugin,
                format!("YTDLP error {:?}", error),
            );
            return Err(error.into());
        }
        if let Some(p) = &self.cookies_path {
            remove_file(p).await?;
        }

        let file = temp_path
            .read_dir()?
            .into_iter()
            .filter_map(|f| if let Ok(file) = f { Some(file) } else { None })
            .find(|f| {
                if let Some(p) = f.path().file_name().and_then(|p| p.to_str()) {
                    p.starts_with(&fileroot) && !p.ends_with(".json")
                } else {
                    false
                }
            });

        let result = file.ok_or(Error::Error("unable to get ytdl output path".to_owned()))?;
        let final_path = result.path();
        let file = File::open(&final_path).await.map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                SourcesError::NotFound(result.path().to_str().map(|a| a.to_string()))
            } else {
                SourcesError::Io(err)
            }
        })?;
        let metadata = file.metadata().await?;
        let mime = final_path.to_str().and_then(get_mime_from_filename);
        let size = metadata.len();

        let filereader = BufReader::new(file);
        let cleanup = CleanupFiles {
            paths: vec![temp_path],
        };
        let fs: FileStreamResult<AsyncReadPinBox> = FileStreamResult {
            stream: Box::pin(filereader),
            size: Some(size),
            accept_range: false,
            range: None,
            mime,
            name: final_path
                .file_name()
                .and_then(|p| p.to_str())
                .map(|f| f.to_owned()),
            cleanup: Some(Box::new(cleanup)),
        };

        Ok(fs)
    }

    pub async fn infos(&mut self) -> RsResult<Option<SingleVideo>> {
        let _execution_lock = YTDL_EXECUTION_LOCK.read().await;
        self.cmd.arg("-J");
        //.stderr(Stdio::piped());
        let output = self.cmd.output().await?;

        let processed = YtDlCommandBuilder::process_json_output(output.stdout)?.into_single_video();
        if let Some(p) = &self.cookies_path {
            remove_file(p).await?;
        }
        Ok(processed)
    }

    fn process_json_output(stdout: Vec<u8>) -> Result<YoutubeDlOutput, Error> {
        use serde_json::json;

        let value: Value = serde_json::from_reader(stdout.as_slice())?;

        let is_playlist = value["_type"] == json!("playlist");
        if is_playlist {
            let playlist: Playlist = serde_json::from_value(value)?;
            Ok(YoutubeDlOutput::Playlist(Box::new(playlist)))
        } else {
            let video: SingleVideo = serde_json::from_value(value)?;
            Ok(YoutubeDlOutput::SingleVideo(Box::new(video)))
        }
    }

    pub async fn run(
        &mut self,
    ) -> Result<Pin<Box<dyn Stream<Item = ProgressStreamItem> + Send>>, Error> {
        let execution_lock = YTDL_EXECUTION_LOCK.clone().read_owned().await;
        self.cmd
            .arg("-f")
            //.arg("best/bestvideo+bestaudio")
            .arg("bestvideo+bestaudio/best")
            .arg("--merge-output-format")
            .arg("mp4")
            .arg("--remux-video")
            .arg("mp4")
            .arg("--recode-video")
            .arg("mp4")
            .arg("--progress-template")
            .arg("\"download:progress=%(progress.downloaded_bytes)s-%(progress.total_bytes)s\"")
            .arg("-o")
            .arg("-")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = self.cmd.spawn()?;

        let stdout =
            ReaderStream::new(child.stdout.take().unwrap()).map(|b| ProgressStreamItem::Data(b));
        let stderr = ReaderStream::new(child.stderr.take().unwrap()).filter_map(|f| {
            let r = f.ok().and_then(|b| {
                from_utf8(&b).ok().and_then(|b| {
                    RsProgress::from_ytdl(b).and_then(|p| Some(ProgressStreamItem::Progress(p)))
                })
            });
            r
        });

        let cookies_path = self.cookies_path.clone();
        tokio::spawn(async move {
            let _execution_lock = execution_lock;
            let r = child.wait().await;
            if let Err(error) = r {
                log_error(
                    crate::tools::log::LogServiceType::Plugin,
                    format!("YTDLP error {:?}", error),
                );
            }
            println!("CLEANING!!!!!");
            if let Some(p) = cookies_path {
                remove_file(p).await.expect("unable to delete file");
            }
        });

        let merged = stdout.merge(stderr);

        Ok(Box::pin(merged))
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use bytes::Bytes;
    use tokio::{io::copy, join, sync::mpsc};
    use tokio_stream::{StreamExt, StreamMap};
    use tokio_util::io::ReaderStream;

    use crate::domain::library::LibraryRole;

    use super::*;

    #[tokio::test]
    async fn test_stream2() -> RsResult<()> {
        let mut reader = YtDlCommandBuilder::new(
            "https://www.youtube.com/watch?v=8kGIlALKO-s",
            Path::new(FILE_NAME),
        )
        .run()
        .await?;
        let mut file: File = File::create(std::env::temp_dir().join("test1.webm")).await?;

        while let Some(data) = reader.next().await {
            match data {
                ProgressStreamItem::Progress(p) => println!("progress: {:?}", p),
                ProgressStreamItem::Data(b) => {
                    file.write(&b?).await?;
                }
            };
        }

        Ok(())
    }

    #[tokio::test]
    #[ignore] // requires network + yt-dlp binary
    async fn test_run_with_cache() -> RsResult<()> {
        let (tx_progress, mut rx_progress) = mpsc::channel::<RsProgress>(100);

        tokio::spawn(async move {
            while let Some(progress) = rx_progress.recv().await {
                println!("PROGRESS {:?}", progress);
            }
            println!("Finished progress");
        });

        let path = YtDlCommandBuilder::new(
            "https://www.youtube.com/watch?v=8kGIlALKO-s",
            Path::new(FILE_NAME),
        )
        .run_with_cache(Some(tx_progress))
        .await?;

        println!("PATH: {:?}", path.mime);

        Ok(())
    }

    #[tokio::test]
    #[ignore] // requires network + yt-dlp binary
    async fn test_run_infos() -> RsResult<()> {
        let path = YtDlCommandBuilder::new(
            "https://www.youtube.com/watch?v=-t7Aa6Dr4pI",
            Path::new(FILE_NAME),
        )
        .infos()
        .await?;

        println!("TAGS: {:?}", path.as_ref().and_then(|r| r.tags.clone()));
        assert!(path.unwrap().tags.unwrap().contains(&"axum".to_owned()));

        Ok(())
    }

    #[test]
    fn refreshes_missing_and_expired_binaries() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(30 * 24 * 60 * 60);

        assert!(YydlContext::should_update(None, now));
        assert!(!YydlContext::should_update(
            Some(now - UPDATE_INTERVAL + Duration::from_secs(1)),
            now
        ));
        assert!(YydlContext::should_update(Some(now - UPDATE_INTERVAL), now));
    }

    #[test]
    fn command_uses_the_managed_binary_path() {
        let binary_path = Path::new("managed-tools").join(FILE_NAME);
        let command = YtDlCommandBuilder::new("https://example.com/video", &binary_path);

        assert_eq!(command.cmd.as_std().get_program(), binary_path.as_os_str());
    }
}
