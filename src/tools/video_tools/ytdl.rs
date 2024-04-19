use std::{any::Any, io, path::{Path, PathBuf}, pin::Pin, process::Stdio, str::from_utf8, sync::Arc};
use bytes::Bytes;
use serde_json::Value;
use stream_map_any::StreamMapAny;
use futures::{AsyncRead, Stream};

use nanoid::nanoid;
use rs_plugin_common_interfaces::request::{RsCookie, RsRequest};
use tokio::{fs::{remove_file, File}, io::{AsyncWrite, AsyncWriteExt, BufReader}, process::{Child, ChildStderr, ChildStdout, Command}};
use tokio_util::io::{ReaderStream, StreamReader};
use youtube_dl::{download_yt_dlp, YoutubeDl};
use tokio_stream::StreamExt;

pub mod ytdl_model;

use crate::{domain::progress::{self, RsProgress, RsProgressCallback, RsProgressType}, error::RsResult, plugins::sources::{error::SourcesError, AsyncReadPinBox, CleanupFiles, FileStreamResult, SourceRead}, server::{get_server_folder_path_array, get_server_temp_file_path}, tools::{file_tools::get_mime_from_filename, log::{log_error, log_info}, video_tools::ytdl::ytdl_model::{Playlist, SingleVideo}}, Error};

use self::ytdl_model::YoutubeDlOutput;

const FILE_NAME: &str = if cfg!(target_os = "windows") {
    "yt-dlp.exe"
} else {
    "yt-dlp"
};


#[derive(Debug, Clone, Default)]
pub struct YydlContext {
    update_checked: bool,
}

impl YydlContext {
    pub async fn new() -> RsResult<Self> {
        if !Self::has_binary() {
            log_info(crate::tools::log::LogServiceType::Other, "Downloading YT-DLP".to_owned());
            let yt_dlp_path = download_yt_dlp(".").await?;
            
            log_info(crate::tools::log::LogServiceType::Other, format!("downloaded YT-DLP at {:?}", yt_dlp_path));
        }
        Ok(YydlContext { update_checked: false})
    }

    pub async fn update_binary() -> RsResult<()> {
        log_info(crate::tools::log::LogServiceType::Other, "Downloading YT-DLP".to_owned());
        let yt_dlp_path = download_yt_dlp(".").await?;
        
        log_info(crate::tools::log::LogServiceType::Other, format!("downloaded YT-DLP at {:?}", yt_dlp_path));
        Ok(())
    }


    pub fn has_binary() -> bool {
        Path::new(FILE_NAME).exists()
    }

    pub async fn request(&self, request: &RsRequest, progress: RsProgressCallback) -> RsResult<SourceRead> {

        let mut command = YtDlCommandBuilder::new(&request.url);
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
        let output = command.run_with_cache(progress).await?;

        let read = SourceRead::Stream(output);
        Ok(read)
    }

    pub async fn request_infos(&self, request: &RsRequest) -> RsResult<Option<SingleVideo>> {

        let mut command = YtDlCommandBuilder::new(&request.url);
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
        let mut process = YoutubeDl::new(request.url.to_owned());
        process.socket_timeout("15");


        let download_path = get_server_folder_path_array(vec![".cache"]).await?;
        let filename = format!("{}.mp4", nanoid!());
        let path = if let Some(cookies) = &request.cookies {
            let p = get_server_temp_file_path().await?;
            let mut file = File::create(&p).await?;
            file.write_all("# Netscape HTTP Cookie File\n".as_bytes()).await?;
            for cookie in cookies {
                file.write_all(format!("{}\n", cookie.netscape()).as_bytes()).await?;
            }
            file.flush().await?;
            process.cookies(&p.as_os_str().to_str().ok_or(Error::Error("unable to parse cookies path".to_owned()))?.to_owned());
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
                total: parts.next().and_then(|p| p.replace("\"", "").parse::<u64>().ok()),
                kind: RsProgressType::Download,
                filename: None
            })
        } else {
            None
        }
    }
}

pub enum ProgressStreamItem {
    Progress(RsProgress),
    Data(Result<Bytes, io::Error>)
}
pub struct YtDlCommandBuilder {
    cmd: Command,
    cookies_path: Option<PathBuf>
}

impl YtDlCommandBuilder {
    pub fn new(path: &str) -> Self {
        let mut cmd = Command::new("yt-dlp");
        cmd.arg(path);
        Self {
            cmd,
            cookies_path: None   
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
        self
    }
    pub fn add_header(&mut self, name: &str, value: &str) -> &mut Self {
        self.cmd.arg("--add-headers").arg(format!("{}:{}", name, value));
        self
    }
    /// Ex: Path to cookies file in netscape format
    pub async fn set_cookies(&mut self, cookies: &Vec<RsCookie>) -> RsResult<&mut Self> {
        let p = get_server_temp_file_path().await?;
        let mut file = File::create(&p).await?;
        file.write_all("# Netscape HTTP Cookie File\n".as_bytes()).await?;
        for cookie in cookies {
            file.write_all(format!("{}\n", cookie.netscape()).as_bytes()).await?;
        }
        file.flush().await?;

        self.cmd
            .arg("--cookies")
            .arg(&p);
        self.cookies_path = Some(p);
        Ok(self)
    }

    pub async fn run_with_cache(&mut self, progress: RsProgressCallback) -> RsResult<FileStreamResult<AsyncReadPinBox>>
    {
        let temp_path = get_server_temp_file_path().await?;
        let fileroot = nanoid!();
        self.cmd
        .arg("-f")
        //.arg("best/bestvideo+bestaudio")
        .arg("bestvideo+bestaudio/best")
        .arg("--merge-output-format")
        .arg("mp4")
        .arg("--remux-video")
        .arg("mp4")
        .arg("--progress-template")
        .arg("\"download:progress=%(progress.downloaded_bytes)s-%(progress.total_bytes)s\"")
        .arg("-P").arg(&temp_path)
        .arg("-o").arg(format!("{}.%(ext)s", fileroot))
        .stdout(Stdio::piped());
        //.stderr(Stdio::piped());
        let mut child = self.cmd
        .spawn()?;
        
        let mut out = ReaderStream::new(child.stdout.take().unwrap()).filter_map(|f| { 
                let r = f.ok().and_then(|b| from_utf8(&b).ok().and_then(RsProgress::from_ytdl));
                r
            }
        );
        if let Some(progress) = progress {
            tokio::spawn(async move {
                while let Some(p) = &mut out.next().await {
                    progress.send(p.to_owned()).await.unwrap();
                }
            });
        }

        let r = child.wait().await;
        if let Err(error) = r {
            log_error(crate::tools::log::LogServiceType::Plugin, format!("YTDLP error {:?}", error));
            return Err(error.into());
        }
        if let Some(p) = &self.cookies_path {
            remove_file(p).await?;
        }
  
        let file = temp_path.read_dir()?.into_iter().filter_map(|f|{
            if let Ok(file) = f {
                Some(file)
            } else {
                None
            }
        }).find(|f| {
            if let Some(p) = f.path().file_name().and_then(|p| p.to_str()) {
                p.starts_with(&fileroot)
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
            paths: vec![temp_path]
        };
        let fs: FileStreamResult<AsyncReadPinBox> = FileStreamResult {
            stream: Box::pin(filereader),
            size: Some(size),
            accept_range: false,
            range: None,
            mime,
            name: final_path.file_name().and_then(|p| p.to_str()).map(|f| f.to_owned()),
            cleanup: Some(Box::new(cleanup)),
        };
    
        Ok(fs)
    } 

    pub async fn infos(&mut self) -> RsResult<Option<SingleVideo>>
    {
        self.cmd
        .arg("-J");
        //.stderr(Stdio::piped());
        let output = self.cmd
        .output().await?;

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

    pub async fn run(&mut self) -> Result<Pin<Box<dyn Stream<Item = ProgressStreamItem> + Send>>, Error>
    {
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
        .arg("-o").arg("-")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        let mut child = self.cmd
        .spawn()?;
        
        let stdout = ReaderStream::new(child.stdout.take().unwrap()).map(|b| ProgressStreamItem::Data(b));
        let stderr = ReaderStream::new(child.stderr.take().unwrap()).filter_map(|f| { 
                let r = f.ok().and_then(|b| from_utf8(&b).ok().and_then(|b| RsProgress::from_ytdl(b).and_then(|p| Some(ProgressStreamItem::Progress(p)))));
                r
            }
        );

        let cookies_path = self.cookies_path.clone();
        tokio::spawn(async move {
            let r = child.wait().await;
            if let Err(error) = r {
                log_error(crate::tools::log::LogServiceType::Plugin, format!("YTDLP error {:?}", error));
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
        let mut reader = YtDlCommandBuilder::new("https://www.youtube.com/watch?v=8kGIlALKO-s").run().await?;
        let mut file: File = File::create("C:\\Users\\arnau\\AppData\\Local\\redseat\\.cache\\test1.webm").await?;


        while let Some(data) = reader.next().await {
            match data {
                ProgressStreamItem::Progress(p) => println!("progress: {:?}", p),
                ProgressStreamItem::Data(b) => {file.write(&b?).await?;},
            };
        }


        Ok(())
    }

        
    #[tokio::test]
    async fn test_run_with_cache() -> RsResult<()> {
        let (tx_progress, mut rx_progress) = mpsc::channel::<RsProgress>(100);

        tokio::spawn(async move {
            while let Some(progress) = rx_progress.recv().await {
                println!("PROGRESS {:?}", progress);
                
            }
            println!("Finished progress");
        });

        let path = YtDlCommandBuilder::new("https://www.youtube.com/watch?v=8kGIlALKO-s").run_with_cache(Some(tx_progress)).await?;

        println!("PATH: {:?}", path.mime);




        Ok(())
    }

    
        
    #[tokio::test]
    async fn test_run_infos() -> RsResult<()> {

        let path = YtDlCommandBuilder::new("https://www.youtube.com/watch?v=-t7Aa6Dr4pI").infos().await?;

        println!("TAGS: {:?}", path.as_ref().and_then(|r| r.tags.clone()));
        assert!(path.unwrap().tags.unwrap().contains(&"axum".to_owned()));


        Ok(())
    }
}

