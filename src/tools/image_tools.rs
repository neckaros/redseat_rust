use core::fmt;
use std::{fs::File, io::{self, BufWriter, Seek, Write}, num::ParseIntError, path::PathBuf, str::FromStr};

use image::{DynamicImage, ImageError as RsImageError, ImageFormat, ImageOutputFormat};
use serde::{Deserialize, Serialize};
use tokio::fs::remove_file;



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

#[derive(Debug, strum_macros::AsRefStr)]
pub enum ImageError {

    Error,
    FfmpegError,
    RsImageError { error: RsImageError },
    IoError { error: io::Error },

}


impl From<RsImageError> for ImageError {
    fn from(error: RsImageError) -> Self {
        ImageError::RsImageError { error }
    }
}

impl From<io::Error> for ImageError {
    fn from(error: io::Error) -> Self {
        ImageError::IoError { error }
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

pub async fn resize_image_path(path: &PathBuf, to: &PathBuf, size: u32, format: ImageFormat) -> ImageResult<()> {
    let output = File::create(to)?;
    let img = image::open(path)?;
    let scaled = resize(img, size);
    let retour = scaled.save_with_format(to, format);
    if retour.is_err() {
        let _ = remove_file(&to).await;
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
    use super::*;

    #[tokio::test]
    async fn convert() {
        //resize_image_path("test_data/image.jpg", "test_data/image.jpg", 500, ImageOutputFormat::Jpeg(80)).await.unwrap()
        //convert_to_pipe("C:/Users/arnau/Downloads/IMG_5020.mov", None).await;
    }
}