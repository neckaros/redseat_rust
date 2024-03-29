use std::{any::Any, io, path::{Path, PathBuf}, pin::Pin, process::Stdio, str::from_utf8};
use bytes::Bytes;
use stream_map_any::StreamMapAny;
use futures::{AsyncRead, Stream};

use nanoid::nanoid;
use plugin_request_interfaces::{RsCookie, RsRequest};
use tokio::{fs::{remove_file, File}, io::{AsyncWrite, AsyncWriteExt, BufReader}, process::{Child, ChildStderr, ChildStdout, Command}};
use tokio_util::io::{ReaderStream, StreamReader};
use youtube_dl::{download_yt_dlp, SingleVideo, YoutubeDl};
use tokio_stream::StreamExt;


use crate::{domain::progress::RsProgress, error::RsResult, plugins::sources::AsyncReadPinBox, server::{get_server_folder_path_array, get_server_temp_file_path}, tools::log::{log_error, log_info}, Error};

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

    pub async fn url(&self, url: &str) -> RsResult<Option<SingleVideo>> {
            let output = YoutubeDl::new(url)
            .socket_timeout("15")
            .run_async()
            .await?;
        let video = output.into_single_video();
        println!("Video title: {:?}", video);
        Ok(video)
    }

    pub async fn request(&self, request: &RsRequest) -> RsResult<Pin<Box<dyn Stream<Item = ProgressStreamItem> + Send >>> {

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
        let output = command.run().await?;
        //if let Some(p) = path {
        //    remove_file(p).await?;
        //}
        Ok(output)
    }

    pub async fn download_to(&self, request: &RsRequest) -> RsResult<PathBuf> {
        let mut process = YoutubeDl::new(request.url.to_owned());
        process.socket_timeout("15");


        let mut download_path = get_server_folder_path_array(vec![".cache"]).await?;
        let filename = format!("{}.mp4", nanoid!());
        let path = if let Some(cookies) = &request.cookies {
            let p = get_server_temp_file_path().await?;
            let mut file = File::create(&p).await?;
            file.write("# Netscape HTTP Cookie File\n".as_bytes()).await?;
            for cookie in cookies {
                file.write(format!("{}\n", cookie.netscape()).as_bytes()).await?;
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
        println!("ytdl {}", str);
        let mut split = str.split("progress=");
        if let Some(progress_part) = split.nth(1) {
            let mut parts = progress_part.split("-");
           Some(Self {
                id: nanoid!(),
                current: parts.next().and_then(|p| p.parse::<u64>().ok()),
                total: parts.next().and_then(|p| p.replace("\"", "").parse::<u64>().ok())
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
    pub fn add_header(&mut self, name: &str, value: &str) -> &mut Self {
        self.cmd.arg("--add-headers").arg(format!("{}:{}", name, value));
        self
    }
    /// Ex: Path to cookies file in netscape format
    pub async fn set_cookies(&mut self, cookies: &Vec<RsCookie>) -> RsResult<&mut Self> {
        let p = get_server_temp_file_path().await?;
        let mut file = File::create(&p).await?;
        file.write("# Netscape HTTP Cookie File\n".as_bytes()).await?;
        for cookie in cookies {
            file.write(format!("{}\n", cookie.netscape()).as_bytes()).await?;
        }
        file.flush().await?;

        self.cmd
            .arg("--cookies")
            .arg(&p);
        self.cookies_path = Some(p);
        Ok(self)
    }

    pub async fn run_with_cache(&mut self, progress: impl Fn(RsProgress)) -> RsResult<(Pin<Box<dyn Stream<Item = ProgressStreamItem> + Send>>, Pin<Box<dyn Stream<Item = ProgressStreamItem> + Send>>)>
    {
        let p = get_server_temp_file_path().await?;
        
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
        
       

        Ok((Box::pin(stdout), Box::pin(stderr)))
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
    use tokio::{io::copy, join};
    use tokio_stream::{StreamExt, StreamMap};
    use tokio_util::io::ReaderStream;

    use crate::domain::library::LibraryRole;

    use super::*;

    #[tokio::test]
    async fn test_role() {
        let ctx = YydlContext::new().await.unwrap();
        ctx.url("https://twitter.com/").await.unwrap();

    }

    
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
}

