use std::io::{BufRead, Read};
use std::path::PathBuf;
use std::{path::Path, process::Stdio};
use std::{default, str};
use rs_plugin_common_interfaces::{RsVideoCodec, RsVideoFormat};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use strum_macros::EnumString;
use time::Instant;
use tokio::sync::mpsc::Sender;
use tokio::{io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader}, process::Command};
use tokio_util::io::ReaderStream;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::LinesStream;

use crate::domain::progress;
use crate::error::RsResult;
use crate::{domain::ffmpeg::FfprobeResult, Error};

use super::log::{log_error, LogServiceType};

pub mod ytdl;

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


#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, strum_macros::Display,EnumString, Default)]
#[serde(rename_all = "camelCase")] 
#[strum(serialize_all = "camelCase")]
pub enum VideoOverlayPosition {
	TopLeft,
    #[default]
    TopRight,
    BottomLeft,
    BottomRight
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, strum_macros::Display,EnumString,)]
#[serde(rename_all = "camelCase")] 
#[strum(serialize_all = "camelCase")]
pub enum VideoOverlayType {
	Watermark,
    File,
}

impl VideoOverlayPosition {
    pub fn as_filter(&self, margin: f64) -> String {
        match self {
            VideoOverlayPosition::TopLeft => format!("main_w*{}:main_h*{}",margin, margin),
            VideoOverlayPosition::TopRight => format!("(main_w-w):min(main_h,main_w)*{}", margin),
            VideoOverlayPosition::BottomLeft => format!("main_w*{}:(main_h-h)", margin),
            VideoOverlayPosition::BottomRight => format!("(main_w-w):(main_h-h)"),
        }
    }
}


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VideoConvertInterval {
    start: f64,
    duration: Option<f64>,
    /// will default to current input
    input: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VideoOverlay {
    #[serde(rename = "type")]
    pub kind: VideoOverlayType,
    pub path: String,
    #[serde(default)]
    pub position: VideoOverlayPosition,
    pub margin: Option<f64>,
    pub ratio: u16,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct VideoConvertRequest {
    pub id: String,
    pub format: RsVideoFormat,
    pub codec: Option<RsVideoCodec>,
    pub crf: Option<u16>,
    #[serde(default)]
    pub no_audio: bool,
    pub width: Option<String>,
    pub height: Option<String>,
    pub framerate: Option<u16>,
    pub crop_width: Option<u16>,
    pub crop_height: Option<u16>,
    pub aspect_ratio: Option<String>,
    pub overlay: Option<VideoOverlay>,
    #[serde(default)]
    pub intervals: Vec<VideoConvertInterval>,
}


#[derive(Debug)]
pub struct VideoCommandBuilder {
    cmd: Command,
    inputs: Vec<String>,
    current_input: u16,
    expected_start: Option<f64>,
    expected_duration: Option<f64>,
    input_options: Vec<String>,
    output_options: Vec<String>,
    video_effects: Vec<String>,
    current_effect_input: String,
    current_effect_count: u16,
    format: Option<RsVideoFormat>,
    progress: Option<Sender<f64>>
}

impl VideoCommandBuilder {
    pub fn new() -> Self {
        let cmd = Command::new("ffmpeg");
        Self {
            cmd,
            inputs: Vec::new(),
            current_input: 0,
            expected_start: None,
            expected_duration: None,
            input_options: Vec::new(),
            output_options: Vec::new(),
            video_effects: Vec::new(),
            current_effect_input: "0".to_string(),
            current_effect_count: 0,
            format: None,
            progress: None
        }
    }

    pub fn set_progress(&mut self, sender: Sender<f64>) {
        self.progress = Some(sender);
    }

    pub fn add_input<S: Into<String>>(&mut self, path: S) -> &mut Self{
        self.inputs.push(path.into());
        self.current_input += 1;
        self
    }

    pub fn add_input_option<S: Into<String>>(&mut self, value: S) -> &mut Self{
        self.input_options.push(value.into());
        self
    }
    pub fn add_out_option<S: Into<String>>(&mut self, value: S) -> &mut Self{
        self.output_options.push(value.into());
        self
    }

    pub fn add_video_effect<S: Into<String>>(&mut self, value: S) -> &mut Self{
        let line: String = value.into();

        let line = if self.current_effect_count > 0 {
            format!("[{}];{}", self.current_effect_input, line)
        } else {
            line
        };

        self.current_effect_count += 1;
        let line = if line.contains("#input#") { line.replace("#input#", &self.current_effect_input) } else { format!("[{}]{}", self.current_effect_input, line) };
        self.current_effect_input = format!("rs{}", self.current_effect_count);
        self.video_effects.push(line);
        self
    }

    pub async fn set_request(&mut self, request: VideoConvertRequest) -> RsResult<&mut Self> {
        self.set_intervals(request.intervals);

        self.set_size(request.width, request.height);
        
        if let (Some(width), Some(height)) = (request.crop_width, request.crop_height) {
            self.set_crop(width, height);
        }
        if let Some(aspect_ratio) = request.aspect_ratio {
            self.set_aspect_ratio(aspect_ratio)?;
        }
        if let Some(framerate) = request.framerate {
            self.set_framerate(framerate);
        }
        
        if request.no_audio {
            self.remove_audio();
        }

        
        if let Some(overlay) = request.overlay {
            self.add_overlay(overlay);
        }
        self.format = Some(request.format);
        self.set_video_codec(request.codec, request.crf).await;
        

   
        

        
        Ok(self)
    }

    /// Ex: 500x500^
    pub fn set_size(&mut self, width: Option<String>, height: Option<String>) -> &mut Self {
        if width.is_some() || height.is_some() {
            self.add_video_effect(format!("scale={}:{}", width.unwrap_or("-1".to_owned()), height.unwrap_or("-1".to_owned())));
        }
        self
    }

    pub fn set_crf(&mut self, crf: u16) -> &mut Self {
        
        self.add_out_option("-crf");
        self.add_out_option(crf.to_string());
        self
    }

    
    pub fn set_framerate(&mut self, fr: u16) -> &mut Self {
        
        self.add_out_option("-r");
        self.add_out_option(fr.to_string());
        self
    }

    pub fn remove_audio(&mut self) -> &mut Self {
        
        self.add_out_option("-an");
        self
    }

    pub async fn set_video_codec(&mut self, codec: Option<RsVideoCodec>, crf: Option<u16>) -> &mut Self {
        match codec {
            Some(RsVideoCodec::H265) => {
                self.add_out_option("-c:v");
                
                let supported_hw = video_hardware().await.unwrap_or_default();
                println!("supported transcoding hw: {:?}", supported_hw);
                if supported_hw.contains(&"cuda".to_string()) {

                    let cq = crf.unwrap_or(28);
                    println!("cuda");
                    self.add_out_option("hevc_nvenc");
                    self.add_out_option("-preset:v");
                    self.add_out_option( "p7");

                    self.add_out_option("-tune:v");
                    self.add_out_option( "hq");

                    self.add_out_option("-profile:v");
                    self.add_out_option( "main10");

                    self.add_out_option("-rc");
                    self.add_out_option( "vbr");

                    self.add_out_option("-rc-lookahead");
                    self.add_out_option( "20");

                    self.add_out_option("-cq:v");
                    self.add_out_option(cq.to_string());

                    self.add_out_option("-qmin");
                    self.add_out_option((cq - 1).to_string());

                    self.add_out_option("-qmax");
                    self.add_out_option((cq + 1).to_string());

                    self.add_out_option("-b:v");
                    self.add_out_option( "0");

                    self.add_out_option("-bufsize");
                    self.add_out_option( "12M");

                    self.add_out_option("-spatial-aq");
                    self.add_out_option( "1");

                    self.add_out_option("-aq-strength");
                    self.add_out_option("15");

                    self.add_out_option("-b:v");
                    self.add_out_option( "0K");

                    self.add_out_option("-pix_fmt");
                    self.add_out_option( "p010le");

                    //self.add_out_option("-level");
                    //self.add_out_option( "4.1");

                    self.add_out_option("-tier");
                    self.add_out_option( "high");

                    self.add_out_option("-bf");
                    self.add_out_option( "3");

                    self.add_out_option("-b_ref_mode");
                    self.add_out_option( "middle");

                    self.add_out_option("-b_strategy");
                    self.add_out_option( "1");

                    self.add_out_option("-i_qfactor");
                    self.add_out_option( "0.75");

                    self.add_out_option("-b_qfactor");
                    self.add_out_option( "1.1");

                    self.add_out_option("-refs");
                    self.add_out_option( "3");

                    self.add_out_option("-g");
                    self.add_out_option( "250");

                    self.add_out_option("-keyint_min");
                    self.add_out_option( "25");

                    self.add_out_option("-sc_threshold");
                    self.add_out_option( "40");

                    self.add_out_option("-qcomp");
                    self.add_out_option( "0.6");

                    self.add_out_option("-qblur");
                    self.add_out_option( "0.5");

                    self.add_out_option("-surfaces");
                    self.add_out_option( "64");

                                        /*self.add_out_option("-preset");
                    self.add_out_option( "slow");

                    self.add_out_option("-rc");
                    self.add_out_option( "vbr");

                    self.add_out_option("-cq");
                    self.add_out_option( cq.to_string());

                    self.add_out_option("-qmin");
                    self.add_out_option(cq.to_string());

                    self.add_out_option("-qmax");
                    self.add_out_option( (cq + 3).to_string());
                    
                    self.add_out_option("-b:v");
                    self.add_out_option( "6M");

                    self.add_out_option("-bufsize");
                    self.add_out_option( "15M");*/

                  //-maxrate:v "$MAXRATE" 
                    /*self.add_out_option("-rc:v");
                    self.add_out_option("vbr");
                    self.add_out_option("-cq:v");
                    self.add_out_option(crf.unwrap_or(28).to_string());*/
                  
                } else {
                    self.add_out_option("libx265");
                    self.add_out_option("-crf:v");
                    self.add_out_option(crf.unwrap_or(26).to_string());
                }
                if self.format.is_none() {
                    
                    self.add_out_option("-movflags");
                    self.add_out_option("faststart");
                    self.format = Some(RsVideoFormat::Mp4);
                }
            },
            Some(RsVideoCodec::H264) => {
                let supported_hw = video_hardware().await.unwrap_or_default();
                self.add_out_option("-c:v");
                if supported_hw.contains(&"cuda".to_string()) {
                    println!("cuda");
                    self.add_out_option("h264_nvenc");
                    self.add_out_option("-preset:v");
                    self.add_out_option( "p7");
                    self.add_out_option("-tune:v");
                    self.add_out_option("hq");
                    self.add_out_option("-rc:v");
                    self.add_out_option("vbr");
                    self.add_out_option("-cq:v");
                    self.add_out_option(crf.unwrap_or(24).to_string());
                    self.add_out_option("-b:v");
                    self.add_out_option("0");
                    self.add_out_option("-profile:v");
                    self.add_out_option( "high");
                } else {
                    self.add_out_option("libx264");
                    
                    self.add_out_option("-crf:v");
                    self.add_out_option(crf.unwrap_or(24).to_string());
                }
                
                
                
                if self.format.is_none() {
                    self.add_out_option("-movflags");
                    self.add_out_option("faststart");
                    self.format = Some(RsVideoFormat::Mp4);
                }
            },
            Some(RsVideoCodec::AV1) => {
                self.add_out_option("-c:v");
                self.add_out_option("libaom-av1");
                if self.format.is_none() {
                    self.format = Some(RsVideoFormat::WebM);
                }
            },
            Some(RsVideoCodec::Custom(custom)) => {
                self.add_out_option("-c:v");
                self.add_out_option(custom);
            },
            Some(RsVideoCodec::Unknown) => (),
            None => {
                self.add_out_option("-c:v");
                self.add_out_option("copy");
            }
        }
        self
    }

    pub fn set_aspect_ratio(&mut self, aspect_ratio: String) -> RsResult<&mut Self> {
        let mut splitted = aspect_ratio.split('/');
        let num = splitted.next().ok_or(Error::Error(format!("Unable to parse ratio {}", aspect_ratio)))?;
        let denum = splitted.next().ok_or(Error::Error(format!("Unable to parse ratio {}", aspect_ratio)))?;
        let num: u16 = num.parse().map_err(|_| Error::Error(format!("Unable to parse ratio numerator {}", aspect_ratio)))?;
        let denum: u16 = denum.parse().map_err(|_| Error::Error(format!("Unable to parse ratio denumerator {}", aspect_ratio)))?;
        if num == denum {
            self.add_video_effect("[#input#]crop='min(iw, ih)':'min(iw, ih)'".to_string());
        } else {
            self.add_video_effect(format!("[#input#]crop='if(gte(iw, ih),ih*({num}/{denum}),iw):if(gte(iw, ih),ih,iw*({denum}/{num}))'"));
        }
        Ok(self)
    }

    pub fn set_crop(&mut self, width: u16, height: u16) -> &mut Self {
        self.add_video_effect(format!("[#input#]crop={}:{}", width, height));
        self
    }

    pub fn add_overlay(&mut self, overlay: VideoOverlay) -> &mut Self {
        self.add_input(overlay.path);
        self.add_video_effect(format!("[{}][#input#]scale2ref=h=ow/mdar:w='max(ih,iw)/{}'[#A logo][bird];[#A logo]format=argb,colorchannelmixer=aa=0.2[#B logo transparent];[bird][#B logo transparent]overlay='{}'", self.current_input, overlay.ratio, overlay.position.as_filter(overlay.margin.unwrap_or(0.02))));
        
        self
    }

    //[${currentInput}][/prefix/]scale2ref=h=ow/mdar:w='max(ih,iw)/6'[#A logo][bird];[#A logo]format=argb,colorchannelmixer=aa=0.2[#B logo transparent];[bird][#B logo transparent]overlay='(main_w-w):min(main_h,main_w)*0.02'
    pub fn set_intervals(&mut self, intervals: Vec<VideoConvertInterval>) -> &mut Self {
        match intervals.len() {
            0 => self,
            1 => {
                let first = intervals.first().unwrap();
                println!("set interval {:?}", first);
                self.add_input_option("-ss").add_input_option(first.start.to_string());
                self.expected_start =  Some(first.start);
                if let Some(duration) = first.duration {
                    self.add_out_option("-t").add_out_option((duration).to_string());
                    self.expected_duration =  Some(duration)
                }
                
                self
            },
            _ => self
        }
        
        
    }

    pub async fn run_file(&mut self, uri: &str, to: &str) -> RsResult<()> {
        let mut frames = get_number_of_frames(uri).await;
        let duration = get_duration(uri).await.unwrap_or(None);

        println!("{:?} / {:?} / {:?}", duration, frames, self.expected_duration);
        if let (Some(duration), Some(all_frames), Some(expected_duration)) = (duration, frames, self.expected_duration) {
            frames = Some((all_frames as f64 * (expected_duration / duration)) as isize);
        } else if let (Some(duration), Some(all_frames), Some(expected_start)) = (duration, frames, self.expected_start) {
            let expected_duration = duration - expected_start;
            frames = Some((all_frames as f64 * (expected_duration / duration)) as isize);
        }

        println!("=> {:?}",frames);
        for input in &self.input_options {
            self.cmd.arg(input);
        }

        self.cmd.arg("-i")
                .arg(uri);

        for input in &self.inputs {
            self.cmd.arg("-i")
                    .arg(input);
        }
            
        if !self.video_effects.is_empty() {
            println!("-filter_complex {}", self.video_effects.join(""));
            self.cmd.arg("-filter_complex")
                    .arg(self.video_effects.join(""));
        }    
         
        for arg in &self.output_options {
            self.cmd.arg(arg);
        }


        if let Some(format) = &self.format {
            self.cmd.arg("-format")
                    .arg(format.to_string());
        }
            
        self.cmd.arg("-y")
            .arg("-progress")
            .arg("pipe:1")
            // Output file
            .arg(to)
            // stdin, stderr, and stdout are piped
            //.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
 
             // Run the child command
        let mut child = self.cmd   
            .spawn()
            .unwrap();
    
        // Take ownership of stdout and stderr from child.
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        // Wrap them up and merge them.
        let stdout = LinesStream::new(BufReader::new(stdout).lines());
        let stderr = LinesStream::new(BufReader::new(stderr).lines());
        let mut merged = StreamExt::merge(stdout, stderr);

        let mut lines: Vec<String> = vec![];
        // Iterate through the stream line-by-line.
        while let Some(line) = merged.next().await {
            let line = line?;
      
            if line.contains("error") {
                log_error(LogServiceType::Other, format!("ffmpeg error: {}", line));
            }
            let line_spit = line.split('=').collect::<Vec<&str>>();
           
            if line_spit[0] == "frame" {
                if let Some(frames) = frames {
                    let frame_number = line_spit[1].parse::<isize>();
                    if let Ok(frame_number) = frame_number {
                        let percent = frame_number as f64 / frames as f64;
                        if let Some(sender) = &self.progress {
                            let _ = sender.send(percent).await;
                        } else {
                            println!("\rProgress: {}%", round(percent * 100_f64, 1));
                        }
                    } else {
                        log_error(LogServiceType::Other, format!("ffmpeg error parsing progress: {}", line));
                        //println!("ERROR parsing: {}", line);
                    }
                    
                } else {
                    //println!("\rProgress: {} frames", line_spit[1]);
                }
            }
            lines.push(line);
       }
        let status = child.wait().await?;
        if !status.success() {
            for line in lines {
                log_error(LogServiceType::Other, line);
            }
            Err(Error::Error("Unable to process video".to_string()))
        } else {
            Ok(())
        }
        
    }

    


    pub async fn run<'a, W>(&mut self, source: &str, format: &str, _writer: &'a mut W) -> Result<(), Error>
    where
        W: AsyncWrite + Unpin + ?Sized,
    {
        let frames = get_number_of_frames(source).await;
        let mut child = self.cmd
        .arg(format!("{}:-", format))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    
        // Take ownership of stdout and stderr from child.
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        // Wrap them up and merge them.
        let stdout = LinesStream::new(BufReader::new(stdout).lines());
        let stderr = LinesStream::new(BufReader::new(stderr).lines());
        let mut merged = StreamExt::merge(stdout, stderr);

        // Iterate through the stream line-by-line.
        while let Some(line) = merged.next().await {
            let line = line?;
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



pub async fn video_hardware() -> Result<Vec<String>, Error> {
    let mut child = Command::new("ffmpeg")
    .arg("-hide_banner")
    .arg("-init_hw_device")
    .arg("list")
    .stdout(Stdio::piped())
    .spawn()?;
    
    let mut results = vec![];
    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();
    while let Some(line) = lines.next_line().await.expect("msg") {
         results.push(line.trim().to_string());
    }
    Ok(results)
    
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
    .await.map_err(|_| Error::Error("unable to probe video".to_owned()))?
    ;
    if let Ok(val) = str::from_utf8(&output.stderr) {
        if val != "" {
            return Err(Error::Error(val.to_string()))
        }
    }
    if let Ok(val) = str::from_utf8(&output.stdout) {

        let mut output_string = val.to_string();
        let len = output_string.trim_end_matches(&['\r', '\n', ' '][..]).len();
        output_string.truncate(len);
        
        let probe: FfprobeResult =  serde_json::from_str(&output_string)?;
        Ok(probe)
    } else {
        Err(Error::GenericRedseatError)
    }
    
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, strum_macros::Display)]
#[strum(serialize_all = "camelCase")]
#[serde(rename_all = "camelCase")]
pub enum VideoTime {
    Seconds(f64),
    Percent(u32)
}

impl VideoTime {
    pub fn position(&self, duration: f64) -> f64 {
        match self {
            VideoTime::Seconds(s) => {
                if s > &duration {
                    duration
                } else {
                    *s
                }
            },
            VideoTime::Percent(p) => duration * (*p as f64 / 100.0),
        }
    }
}

pub async fn thumb_video(uri: &str, at_time: VideoTime) -> Result<Vec<u8>, Error> {
    let duration = get_duration(uri).await?.ok_or(Error::Error("Unable to get video duration".to_owned()))?;
    let ss = at_time.position(duration);
    let output = Command::new("ffmpeg")
    .arg("-ss")
    .arg(ss.to_string())
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
    .await.map_err(|error| Error::Error(format!("unable to get video thumb ffmpeg: {:?}", error)))?;
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

pub async fn get_duration(uri: &str) -> RsResult<Option<f64>> {  
    let probe = probe_video(uri).await?;
    Ok(probe.duration())
    
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