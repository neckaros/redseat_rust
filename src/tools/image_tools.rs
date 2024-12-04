use core::fmt;
use std::{fs::{remove_file, File}, io::{Seek, Write}, num::ParseIntError, path::PathBuf, process::Stdio, str::{from_utf8, FromStr}};

use image::{ColorType, DynamicImage, ImageEncoder, ImageOutputFormat};
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};
use strum_macros::{Display, EnumIter, EnumString};
use tokio::{io::{AsyncRead, AsyncWrite, AsyncWriteExt}, process::{Child, Command}};
use webp::WebPEncodingError;
use derive_more::From;
use which::which;

use crate::{error::RsResult, Error};

use self::image_magick::ImageMagickInfo;

pub mod image_magick;

pub type ImageResult<T> = core::result::Result<T, ImageError>;



#[derive(Debug, Serialize, Deserialize, Clone, EnumIter, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ImageSize {
    Thumb,
    Small,
    Large,
    Custom(u32)
}

impl fmt::Display for ImageSize {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ImageSize::Thumb => write!(f, "thumb"),
            ImageSize::Small => write!(f, "small"),
            ImageSize::Large => write!(f, "large"),
            ImageSize::Custom(width) => write!(f, "{}", width),
        }
    }
}
impl FromStr for ImageSize {
    type Err = ();

    fn from_str(input: &str) -> std::result::Result<ImageSize, ()> {
        let int_size: core::result::Result<u32, ParseIntError> = input.parse();
        match int_size {
            Ok(size) => Ok(ImageSize::Custom(size)),
            Err(_) => match input {
                "thumb"  => Ok(ImageSize::Thumb),
                "small"  => Ok(ImageSize::Small),
                "large"  => Ok(ImageSize::Small),
                _      => Err(()),
            },
        }
        
    }
}
impl ImageSize {
    pub fn to_size(&self) -> u32 {
        match self {
            ImageSize::Thumb => 258,
            ImageSize::Small => 512,
            ImageSize::Large => 1024,
            ImageSize::Custom(width) => width.clone(),
        }
    }

    pub fn to_filename_element(&self) -> String {
        format!(".{}", self.to_string())
    }
    pub fn optional_to_filename_element(optinal: &Option<Self>) -> String {
        match optinal {
            Some(kind) => kind.to_filename_element(),
            None => "".to_string(),
        }
        
    }
}



#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Display)]
#[serde(rename_all = "camelCase")]
pub enum ImageType {
    Poster,
    Background,
    Still,
    Card,
    ClearLogo,
    ClearArt,
    Custom(String)
}

impl ImageType {
    pub fn to_filename_element(&self) -> String {
        format!(".{}", self.to_string())
    }
    pub fn optional_to_filename_element(optinal: &Option<Self>) -> String {
        match optinal {
            Some(kind) => kind.to_filename_element(),
            None => "".to_string(),
        }
        
    }
}

#[serde_as]
#[derive(Debug, Serialize, strum_macros::AsRefStr, From)]
pub enum ImageError {

    Error,
    FfmpegError,

    
    UnableToDecodeWebp(String),
    
	#[from]
	Io(#[serde_as(as = "DisplayFromStr")] std::io::Error),


    
	#[from]
	RsImageError(#[serde_as(as = "DisplayFromStr")] image::ImageError),

}

impl From<WebPEncodingError> for ImageError {
    fn from(_error: WebPEncodingError) -> Self {
        ImageError::Error
    }
}

// region:    --- Error Boilerplate

impl core::fmt::Display for ImageError {
	fn fmt(
		&self,
		fmt: &mut core::fmt::Formatter,
	) -> core::result::Result<(), core::fmt::Error> {
		write!(fmt, "{self:?}")
	}
}

impl std::error::Error for ImageError {}

pub fn has_image_magick() -> bool {
    which("magick").is_ok()
}

pub struct ImageCommandBuilder {
    cmd: Command
}

impl ImageCommandBuilder {
    pub fn new() -> Self {
        let mut cmd = Command::new("magick");
        cmd.arg("-");
        Self { cmd}
    }

    pub fn set_quality(&mut self, quality: u16) -> &mut Self {
        self.cmd
            .arg("-quality")
            .arg(quality.to_string());
        self
    }

    pub fn auto_orient(&mut self) -> &mut Self {
        self.cmd
            .arg("-auto-orient");
        self
    }

    

    /// Ex: 500x500^
    pub fn set_size(&mut self, size: &str) -> &mut Self {
        self.cmd
            .arg("-resize")
            .arg(size);
        self
    }
    
    pub async fn infos<'a, R>(&mut self, reader: &'a mut R) -> RsResult<Vec<ImageMagickInfo>> where
    R: AsyncRead + Unpin + ?Sized
    {
        let mut cmd = self.cmd
        .arg("json:-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    
        if let Some(mut stdin) = cmd.stdin.take() {  
            tokio::io::copy(reader, &mut stdin).await?;
        }
        let output = cmd.wait_with_output().await?;
        
        let str = from_utf8(&output.stdout).map_err(|e| Error::Error(format!("Unable to parse output to string: {:?}", e)))?;

        let info: Vec<ImageMagickInfo> = serde_json::from_str(str)?;
        
        //writer.write_all(&output.stdout).await?;
        Ok(info)
    }


    pub async fn run<'a, R>(&mut self, format: &str, reader: &'a mut R) -> ImageResult<Vec<u8>>
    where
        R: AsyncRead + Unpin + ?Sized
    {
        let mut cmd = self.cmd
        .arg(format!("{}:-", format))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    
        if let Some(mut stdin) = cmd.stdin.take() {  
            tokio::io::copy(reader, &mut stdin).await?;
        }
        let output = cmd.wait_with_output().await?;
        
        //writer.write_all(&output.stdout).await?;
        Ok(output.stdout)
    }
}

pub async fn resize_image_path(path: &PathBuf, to: &PathBuf, size: u32) -> ImageResult<()> {

    let mut source = tokio::fs::File::open(&path).await?;
    let mut file = tokio::fs::File::create(to).await?;

    let mut builder = ImageCommandBuilder::new();
    builder.auto_orient();
    builder.set_quality(80);
    builder.set_size(&format!("{}x{}^", size, size));
    let data = builder.run("webp",&mut source).await?;
    file.write_all(&data).await?;
    Ok(())
}
pub async fn resize_image_reader<R>(reader: &mut R, size: u32) -> ImageResult<Vec<u8>>where
    R: AsyncRead + Unpin + ?Sized,   {
    let mut builder = ImageCommandBuilder::new();
    builder.auto_orient();
    builder.set_quality(80);
    builder.set_size(&format!("{}x{}^", size, size));
    let data = builder.run("webp",reader).await?;
    
    Ok(data)
}

pub async fn convert_image_reader<R>(reader: &mut R, format: &str, quality: Option<u16>) -> ImageResult<Vec<u8>>where
    R: AsyncRead + Unpin + ?Sized,   {
    let mut builder = ImageCommandBuilder::new();
    builder.auto_orient();
    builder.set_quality(quality.unwrap_or(80));
    let data = builder.run(format,reader).await?;
    
    Ok(data)
}


pub async fn resize_image_path_native(path: &PathBuf, to: &PathBuf, size: u32) -> ImageResult<()> {
    let mut output = File::create(to)?;
    let img = image::open(path)?;
    let scaled = resize(img, size);
    let result = webp::Encoder::from_image(&scaled).map_err(|e| ImageError::UnableToDecodeWebp(e.into()))?
        .encode_simple(false, 80.0);

    if result.is_err() {
        let _ = remove_file(&to);
    } else {
        output.write_all(&*result.unwrap())?;
    }
    Ok(())
}

pub async fn resize_image_path_avif(path: &PathBuf, to: &PathBuf, size: u32) -> ImageResult<()> {
    let mut output = File::create(to)?;
    let img = image::open(path)?;
    
    let scaled = resize(img, size);
    let imbuf = scaled.to_rgba8();

    let mut encoded = Vec::new();
    let encoder = image::codecs::avif::AvifEncoder::new_with_speed_quality(&mut encoded, 8, 80);
    let result = encoder.write_image(&imbuf, imbuf.width(), imbuf.height(), ColorType::Rgba8);
   
    if result.is_err() {
        let _ = remove_file(&to);
    } else {
        output.write_all(&*encoded)?;
    }
    Ok(())
}


pub fn resize_image<T: Write + Seek>(buffer: &[u8], to: &mut T, size: u32, format: ImageOutputFormat) -> ImageResult<()> {
    let img = image::load_from_memory(buffer)?;
    let thumb = resize(img, size);
    thumb.write_to(to, format)?;
    Ok(())
}

fn resize(image: DynamicImage, size: u32) -> DynamicImage {
    let thumb = image.thumbnail(size, size);
    thumb
}


#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[tokio::test]
    async fn convert() {
        let source = PathBuf::from_str("test_data/image.jpg").expect("unable to set path");
        let target = PathBuf::from_str("test_data/image.avif").expect("unable to set path");
        if target.exists() {
            fs::remove_file(&target).expect("failed to remove existing result file");
        }
        resize_image_path(&source, &target, 7680).await.unwrap()
        //convert_to_pipe("C:/Users/arnau/Downloads/IMG_5020.mov", None).await;
    }


    #[tokio::test]
    async fn info() {
        let source = PathBuf::from_str("test_data/image.jpg").expect("unable to set path");
        let mut file = tokio::fs::File::open(source).await.unwrap();
        let info = ImageCommandBuilder::new().infos(&mut file).await.unwrap();
        println!("INFOS: {:?}", info)
        //convert_to_pipe("C:/Users/arnau/Downloads/IMG_5020.mov", None).await;
    }
}