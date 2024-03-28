use std::{any::Any, io, path::{Path, PathBuf}, pin::Pin, process::Stdio, str::from_utf8};
use bytes::Bytes;
use stream_map_any::StreamMapAny;
use futures::{AsyncRead, Stream, StreamExt};
use nanoid::nanoid;
use plugin_request_interfaces::RsRequest;
use tokio::{fs::{remove_file, File}, io::{AsyncWrite, AsyncWriteExt, BufReader}, process::{Child, ChildStderr, ChildStdout, Command}};
use tokio_util::io::{ReaderStream, StreamReader};
use youtube_dl::{download_yt_dlp, SingleVideo, YoutubeDl};

use crate::{domain::progress::RsProgress, error::RsResult, plugins::sources::AsyncReadPinBox, server::{get_server_folder_path_array, get_server_temp_file_path}, tools::log::log_info, Error};

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

    pub async fn request(&self, request: &RsRequest) -> RsResult<Option<SingleVideo>> {
        let mut process = YoutubeDl::new(request.url.to_owned());
        process.socket_timeout("15");

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
        println!("path: {:?}", path);
        let output = process
        .run_async()
        .await?;
        if let Some(p) = path {
            remove_file(p).await?;
        }
        let video = output.into_single_video();
        println!("Video title: {:?}", video);
        Ok(video)
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

pub enum YtdlItem {
    Progress(RsProgress),
    Data(Result<Bytes, io::Error>)
}
pub struct YtDlCommandBuilder {
    cmd: Command,
    input_options: Vec<String>,
    output_options: Vec<String>,
    video_effects: Vec<String>,
}

impl YtDlCommandBuilder {
    pub fn new(path: &str) -> Self {
        let mut cmd = Command::new("yt-dlp");
        cmd.arg(path);
        Self {
            cmd,
            input_options: Vec::new(),
            output_options: Vec::new(),
            video_effects: Vec::new(),
        }
    }


    /// Ex: 500x500^
    pub fn set_size(&mut self, size: &str) -> &mut Self {
        self.cmd
            .arg("-resize")
            .arg(size);
        self
    }

    pub async fn run(&mut self) -> Result<(ReaderStream<ChildStdout>, Pin<Box<dyn Stream<Item = RsProgress>>>), Error>
    {
        self.cmd
        .arg("--progress-template")
        .arg("\"download:progress=%(progress.downloaded_bytes)s-%(progress.total_bytes)s\"")
        .arg("-o").arg("-")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        let mut child = self.cmd
        .spawn()?;
    
        let stdout = ReaderStream::new(child.stdout.take().unwrap());
        let stderr = ReaderStream::new(child.stderr.take().unwrap()).filter_map(|f| async { 
           let r = f.ok().and_then(|b| from_utf8(&b).ok().and_then(|b| RsProgress::from_ytdl(b)));
           r
            }
        );
        //let reader = BufReader::new(stdout);
        // let mut lines = reader.lines();
        // while let Some(line) = lines.next_line().await.expect("msg") {
        //      let line_spit = line.split("=").collect::<Vec<&str>>();
        //      if line_spit[0] == "frame" {
        //          if let Some(frames) = frames {
        //              let frame_number = line_spit[1].parse::<isize>().unwrap();
        //              let percent = frame_number as f64 / frames as f64 * 100 as f64;
        //              println!("\rProgress: {}%", round(percent, 1));
        //          } else {
        //              println!("\rProgress: {} frames", line_spit[1]);
        //          }
        //      }
        // }
        //  child.wait().await.expect("oops");

        Ok((stdout, Box::pin(stderr)))
    }
    
}




#[cfg(test)]
mod tests {
    use std::io;

    use bytes::Bytes;
    use futures::StreamExt;
    use tokio::{io::copy, join};
    use tokio_stream::StreamMap;
    use tokio_util::io::ReaderStream;

    use crate::domain::library::LibraryRole;

    use super::*;

    #[tokio::test]
    async fn test_role() {
        let ctx = YydlContext::new().await.unwrap();
        ctx.url("https://twitter.com/").await.unwrap();

    }

    
    #[tokio::test]
    async fn test_stream() -> RsResult<()> {
        let mut reader = YtDlCommandBuilder::new("https://www.youtube.com/watch?v=8kGIlALKO-s").run().await?;
        let mut file: File = File::create("C:\\Users\\arnau\\AppData\\Local\\redseat\\.cache\\test1.webm").await?;
        /*let a = reader.0.for_each(|f| async move { println!("test"); 
            if let Ok(f) = f {
                //&file.write(&f);
            }
        
    });*/
    tokio::spawn( async move {
       //let b = reader.1.for_each(|f| async move { println!("progress {:?}\n", f)}).await;
    });
        
        while let Some(data) = reader.0.next().await {
            file.write(&data?).await?;
        }

       //copy(&mut reader.1, &mut file).await?;
       //join!(b);
        //reader.0.wait().await?;
        Ok(())
    }

        
    #[tokio::test]
    async fn test_stream2() -> RsResult<()> {
        let reader = YtDlCommandBuilder::new("https://www.youtube.com/watch?v=8kGIlALKO-s").run().await?;
        let mut file: File = File::create("C:\\Users\\arnau\\AppData\\Local\\redseat\\.cache\\test1.webm").await?;
        let mut map = StreamMapAny::new();
        map.insert("data", reader.0);
        map.insert("progress", reader.1);


        while let Some(data) = map.next().await {
            //file.write(&data?).await?;
            if let ("data", variant) = data {
                let v = variant.value::<Result<Bytes, io::Error>>().map_err(|_| crate::Error::Error("map error".to_owned()))??;
                file.write(&v).await?;
                
            } else if let ("progress", variant) = data {
                println!("progress: {:?}", variant.value::<RsProgress>().map_err(|_| crate::Error::Error("Unable to get RsProgres".to_owned()))?.percent() )
            }
        }


        Ok(())
    }
}

