use core::fmt;
use std::{fs::{remove_file, File}, io::{self, Seek, Write}, num::ParseIntError, path::PathBuf, str::FromStr};

use image::{ColorType, DynamicImage, ImageEncoder, ImageError as RsImageError, ImageFormat, ImageOutputFormat};
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};
use webp::WebPEncodingError;
use derive_more::From;


pub type ImageResult<T> = core::result::Result<T, ImageError>;



#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")] 
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



#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")] 
pub enum ImageType {
    Poster,
    Background,
    Still,
    Card,
    ClearLogo,
    ClearArt,
    Custom(String)
}

impl fmt::Display for ImageType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ImageType::Poster => write!(f, "poster"),
            ImageType::Background => write!(f, "background"),
            ImageType::Still => write!(f, "still"),
            ImageType::Card => write!(f, "card"),
            ImageType::ClearLogo => write!(f, "clearlogo"),
            ImageType::ClearArt => write!(f, "clearart"),
            ImageType::Custom(text) => write!(f, "{}", text),
        }
    }
}
impl FromStr for ImageType {
    type Err = ();

    fn from_str(input: &str) -> std::result::Result<ImageType, ()> {
        match input {
                "poster"  => Ok(ImageType::Poster),
                "background"  => Ok(ImageType::Background),
                "still"  => Ok(ImageType::Still),
                "card"  => Ok(ImageType::Poster),
                "clearlogo"  => Ok(ImageType::Background),
                "clearart"  => Ok(ImageType::Still),
                text      => Ok(ImageType::Custom(text.into())),
        }
    }
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
    fn from(error: WebPEncodingError) -> Self {
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

pub async fn resize_image_path(path: &PathBuf, to: &PathBuf, size: u32) -> ImageResult<()> {
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
}