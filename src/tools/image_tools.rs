use core::fmt;
use std::{fs::{remove_file, File}, io::{Cursor, Seek, Write}, num::ParseIntError, path::PathBuf, process::Stdio, str::{from_utf8, FromStr}};

use image::{ColorType, DynamicImage, ExtendedColorType, ImageEncoder, ImageFormat};
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};
use strum_macros::{Display, EnumIter, EnumString};
use tokio::{io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt}, process::{Child, Command}};
use webp::WebPEncodingError;
use derive_more::From;
use which::which;

use libheif_sys as lh;

use crate::{error::RsResult, server::get_config, Error};

use self::image_magick::ImageMagickInfo;

use super::convert::heic::read_heic_file_to_image;

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
    let config = get_config().await;
    
    let data = if config.noIM {
        resize_image_reader_native(reader, size).await?
    } else {
        resize_image_reader_im(reader, size).await?
    };
    
    Ok(data)
}

fn is_heic(data: &[u8]) -> bool {
    // Check if the data is large enough to contain the magic bytes
    if data.len() < 12 {
        return false;
    }

    // HEIC magic bytes typically start at offset 4
    let magic = &data[4..12];
    matches!(magic, b"ftypheic" | b"ftypheix" | b"ftypmif1" | b"ftypmsf1")
}

pub async fn resize_image_reader_native<R>(reader: &mut R, size: u32) -> ImageResult<Vec<u8>>where
    R: AsyncRead + Unpin + ?Sized,   {

    let mut data = Vec::new();
    let mut reader = tokio::io::BufReader::new(reader);
    reader.read_to_end(&mut data).await?;
    let heic = is_heic(&data);
    let image = if heic {
        println!("using HEIC");
        read_heic_file_to_image(&data)
    } else {
        image::io::Reader::new(Cursor::new(data)).with_guessed_format()?.decode()?
    };
    let scaled = resize(image, size);
    let webp_data = webp::Encoder::from_image(&scaled).map_err(|e| ImageError::UnableToDecodeWebp(e.into()))?
        .encode_simple(false, 80.0)?;
    Ok(webp_data.to_vec())
}

pub async fn resize_image_reader_im<R>(reader: &mut R, size: u32) -> ImageResult<Vec<u8>>where
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



pub fn resize_image<T: Write + Seek>(buffer: &[u8], to: &mut T, size: u32, format: ImageFormat) -> ImageResult<()> {
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