use std::{path::Path, process::Stdio};
use std::str;
use serde::Serialize;
use serde_with::serde_as;
use tokio::{io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader}, process::Command};

use crate::{domain::ffmpeg::FfprobeResult, Error};


pub type VideoResult<T> = core::result::Result<T, VideoError>;

#[serde_as]
#[derive(Debug, Serialize, strum_macros::AsRefStr)]
pub enum VideoError {

    Error,
    FfmpegError,

}

// region:    --- Error Boilerplate

impl core::fmt::Display for VideoError {
	fn fmt(
		&self,
		fmt: &mut core::fmt::Formatter,
	) -> core::result::Result<(), core::fmt::Error> {
		write!(fmt, "{self:?}")
	}
}

impl std::error::Error for VideoError {}

struct VideoCommandBuilder {
    cmd: Command,
    input_options: Vec<String>,
    output_options: Vec<String>,
    video_effects: Vec<String>,
}

impl VideoCommandBuilder {
    pub fn new(path: &str) -> Self {
        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-i")
            .arg(path);
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

    pub async fn run<'a, W>(&mut self, source: &str, format: &str, writer: &'a mut W) -> Result<(), Error>
    where
        W: AsyncWrite + Unpin + ?Sized,
    {
        let frames = get_number_of_frames(source).await;
        let mut child = self.cmd
        .arg(format!("{}:-", format))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    
        let stdout = child.stdout.take().unwrap();
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        while let Some(line) = lines.next_line().await.expect("msg") {
             let line_spit = line.split("=").collect::<Vec<&str>>();
             if line_spit[0] == "frame" {
                 if let Some(frames) = frames {
                     let frame_number = line_spit[1].parse::<isize>().unwrap();
                     let percent = frame_number as f64 / frames as f64 * 100 as f64;
                     println!("\rProgress: {}%", round(percent, 1));
                 } else {
                     println!("\rProgress: {} frames", line_spit[1]);
                 }
             }
        }
         child.wait().await.expect("oops");

        Ok(())
    }
}



pub async fn probe_video(uri: &str) -> Result<FfprobeResult, Error> {
    let output = Command::new("ffprobe")
    .arg("-v")
    .arg("error")
    .arg("-show_streams")
    .arg("-show_entries")
    .arg("format")
    .arg("-of")
    .arg("json")
    .arg(uri)
    .output()
    .await.map_err(|_| Error::GenericRedseatError)?
    ;
    if let Ok(val) = str::from_utf8(&output.stderr) {
        if val != "" {
            return Err(Error::Error { message: val.to_string() })
        }
    }
    if let Ok(val) = str::from_utf8(&output.stdout) {

        let mut output_string = val.to_string();
        let len = output_string.trim_end_matches(&['\r', '\n', ' '][..]).len();
        output_string.truncate(len);
        
        let probe: FfprobeResult =  serde_json::from_str(&output_string).unwrap();
        Ok(probe)
    } else {
        Err(Error::GenericRedseatError)
    }
    
}

pub async fn thumb_video(uri: &str, at_time: f64) -> Result<Vec<u8>, Error> {
    let output = Command::new("ffmpeg")
    .arg("-ss")
    .arg(at_time.to_string())
    .arg("-i")
    .arg(uri)
    .arg("-vframes")
    .arg("1")
    .arg("-f")
    .arg("image2pipe")
    .arg("-vcodec")
    .arg("png")
    .arg("pipe:1")

    .output()
    .await.map_err(|_| Error::GenericRedseatError)?
    ;
    /*if let Ok(val) = str::from_utf8(&output.stderr) {
        if val != "" {
            return Err(Error::Error { message: val.to_string() })
        }
    }*/
        
    Ok(output.stdout)
    
    
}


pub async fn get_number_of_frames(uri: &str) -> Option<isize> {  
    if let Some(probe) = probe_video(uri).await.ok() {
        probe.number_of_video_frames()
    } else {
        None
    } 
}

pub async fn get_duration(uri: &str) -> Option<f64> {  
    if let Some(probe) = probe_video(uri).await.ok() {
        probe.duration()
    } else {
        None
    } 
}

pub async fn convert(uri: &str, to: &str, args: Option<Vec<String>>) {
    let frames = get_number_of_frames(uri).await;
    let mut command = Command::new("ffmpeg");
        command.arg("-i")
        .arg(uri)
        .arg("-y")
        .arg("-progress")
        .arg("pipe:1")
        // Output file
        .arg(to)
        // stdin, stderr, and stdout are piped
        //.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(args) = args {
        command.args(args);
    }
         // Run the child command
    let mut child = command   
        .spawn()
        .unwrap();

   // let stdin = child.stdin.as_mut().unwrap();
   let stdout = child.stdout.take().unwrap();
   let reader = BufReader::new(stdout);
   let mut lines = reader.lines();
   while let Some(line) = lines.next_line().await.expect("msg") {
        let line_spit = line.split("=").collect::<Vec<&str>>();
        if line_spit[0] == "frame" {
            if let Some(frames) = frames {
                let frame_number = line_spit[1].parse::<isize>().unwrap();
                let percent = frame_number as f64 / frames as f64 * 100 as f64;
                println!("\rProgress: {}%", round(percent, 1));
            } else {
                println!("\rProgress: {} frames", line_spit[1]);
            }
        }
   }
    child.wait().await.expect("oops");
}

fn round(x: f64, decimals: u32) -> f64 {
    let y = 10i32.pow(decimals) as f64;
    (x * y).round() / y
}



#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn convert() {
        //convert_to_pipe("C:/Users/arnau/Downloads/IMG_5020.mov", None).await;
    }
}