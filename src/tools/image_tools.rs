use std::{fs::File, io::{self, BufWriter, Seek, Write}};

use image::{DynamicImage, ImageError as RsImageError, ImageOutputFormat};



pub type ImageResult<T> = core::result::Result<T, ImageError>;

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

pub async fn resize_image_path(path: &str, to: &str, size: u32, format: ImageOutputFormat) -> ImageResult<()> {
    let output = File::create(to)?;
    let mut save_file_buffer = BufWriter::new(output);
    let img = image::open(path)?;
    let scaled = resize(img, size);
    
    scaled.write_to(&mut save_file_buffer, format)?;
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
        resize_image_path("test_data/image.jpg", "test_data/image.jpg", 500, ImageOutputFormat::Jpeg(80)).await.unwrap()
        //convert_to_pipe("C:/Users/arnau/Downloads/IMG_5020.mov", None).await;
    }
}