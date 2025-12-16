use core::fmt;
use std::{fs::{remove_file, File}, io::{Cursor, Seek, Write}, num::ParseIntError, path::PathBuf, process::Stdio, str::{from_utf8, FromStr}};

use chrono::{TimeZone, Utc};
use exif::{In, Tag};
use image::{ColorType, DynamicImage, ExtendedColorType, ImageDecoder, ImageEncoder, ImageFormat};
use image_magick::Image;
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};
use strum_macros::{Display, EnumIter, EnumString};
use tokio::{io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt}, process::{Child, Command}};
use tokio_util::{compat::TokioAsyncReadCompatExt, io::SyncIoBridge};

use webp::WebPEncodingError;
use derive_more::From;
use which::which;

use libheif_sys as lh;

use crate::{domain::media::MediaForUpdate, error::{RsError, RsResult}, plugins::sources::AsyncReadPinBox, server::get_config, tools::convert::jxl::{is_jxl, read_jxl_file_to_image}, Error};

use self::image_magick::ImageMagickInfo;

use super::convert::{heic::{read_heic_file_to_image, read_heic_infos}, raw::is_raw};

pub mod image_magick;

pub type ImageResult<T> = core::result::Result<T, ImageError>;

pub static IMAGES_MIME_FULL_BROWSER_SUPPORT: [&str; 5] = ["image/jpeg", "image/gif", "image/png", "image/avif", "image/webp"];

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
    pub fn image_format_to_extension(format: &ImageFormat) -> String {
        format.extensions_str().concat()
    }
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



fn is_heic(data: &[u8]) -> bool {
    // Check if the data is large enough to contain the magic bytes
    if data.len() < 12 {
        return false;
    }

    // HEIC magic bytes typically start at offset 4
    let magic = &data[4..12];
    //println!("Magic: {:?}", magic);
    matches!(magic, b"ftypheic" | b"ftypheix" | b"ftypmif1" | b"ftypmsf1")
}
fn is_avif(data: &[u8]) -> bool {
    // Check if the data is large enough to contain the magic bytes
    if data.len() < 12 {
        return false;
    }
    // HEIC magic bytes typically start at offset 4
    let magic = &data[4..12];
    matches!(magic, b"ftypavif")
}
#[derive(Default, Debug, Clone)]
pub struct ImageAndProfile {
    pub image: DynamicImage,
    pub profile: Option<Vec<u8>>
}

pub fn read_avif(data: Vec<u8>) -> RsResult<ImageAndProfile> {
    let mut reader = image::io::Reader::new(Cursor::new(data));
    reader.set_format(ImageFormat::Avif);
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation();
    let profile = decoder.icc_profile().ok().flatten();
    let mut image = DynamicImage::from_decoder(decoder)?;
    if let Ok(orientation) = orientation {
        image.apply_orientation(orientation);
    }
    Ok(ImageAndProfile {
        image,
        profile
    })
}

pub fn read_guess(data: Vec<u8>) -> RsResult<ImageAndProfile> {
    let mut decoder = image::io::Reader::new(Cursor::new(data)).with_guessed_format()?.into_decoder()?;
    let orientation = decoder.orientation();
    let profile = decoder.icc_profile().ok().flatten();
    let mut image = DynamicImage::from_decoder(decoder)?;
    if let Ok(orientation) = orientation {
        image.apply_orientation(orientation);
    }
    Ok(ImageAndProfile {
        image,
        profile
    })
}

pub async fn reader_to_image<R>(reader: &mut R) -> RsResult<ImageAndProfile> where
    R: AsyncRead + Unpin + ?Sized,   {

    let mut data = Vec::new();
    let mut reader = tokio::io::BufReader::new(reader);
    reader.read_to_end(&mut data).await?;
    let heic = is_heic(&data);
    let avif = is_avif(&data);
    let raw = is_raw(&data);
    let image = if heic {
        let result = tokio::task::spawn_blocking(move || {
            read_heic_file_to_image(&data)
        }).await?;
        result?
    } else if avif {
        tokio::task::spawn_blocking(move || {
            read_avif(data)
        }).await??
    
    } else if is_jxl(&data) {
        tokio::task::spawn_blocking(move || {
            read_jxl_file_to_image(&data)
        }).await??
    } else {
        tokio::task::spawn_blocking(move || {
            read_guess(data)
        }).await??
    };

    Ok(image)
}

pub async fn resize_image_reader(reader: AsyncReadPinBox, size: u32, format: ImageFormat, quality: Option<u16>, fast: bool) -> RsResult<Vec<u8>>    {
    let config = get_config().await;
    
    let data = if config.imagesUseIm {
        resize_image_reader_im(reader, size).await?
    } else {
        resize_image_reader_native(reader, size, format, quality, fast).await?
    };
    
    Ok(data)
}

pub async fn resize_image_reader_native(mut reader: AsyncReadPinBox, size: u32, format: ImageFormat, quality: Option<u16>, fast: bool) -> RsResult<Vec<u8> >   {

        let mut image = reader_to_image(&mut reader).await?;
        let width = image.image.width();
        let height = image.image.height();
        if (width as f64) / (height as f64) < 0.25 {
            let square = image.image.crop(0, 0, width, width);
            image.image = square;
        }
        let scaled = resize(image.image, size).await?;
        image.image = scaled;
        tokio::task::spawn_blocking(move || {
        save_image_native(image, format, quality, fast)
        }).await?

}

pub async fn resize_image_reader_im(mut reader: AsyncReadPinBox, size: u32) -> ImageResult<Vec<u8>>  {
    let mut builder = ImageCommandBuilder::new();
    builder.auto_orient();
    builder.set_quality(80);
    builder.set_size(&format!("{}x{}^", size, size));
    let data = builder.run("avif",&mut reader).await?;
    Ok(data)
}

pub async fn convert_image_reader(reader:AsyncReadPinBox, format: ImageFormat, quality: Option<u16>, fast: bool) -> RsResult<Vec<u8>> {
        let config = get_config().await;
    
        let data = if config.imagesUseIm {
            convert_image_reader_im(reader, format, quality, fast).await
        } else {
            convert_image_reader_native(reader, format, quality, fast).await
        };
        data
}

pub async fn convert_image_reader_im(mut reader: AsyncReadPinBox, format: ImageFormat, quality: Option<u16>, fast: bool) -> RsResult<Vec<u8>> {
    let mut builder = ImageCommandBuilder::new();
    builder.auto_orient();
    builder.set_quality(quality.unwrap_or(80));
    let data = builder.run(&ImageCommandBuilder::image_format_to_extension(&format),&mut reader).await?;
    
    Ok(data)
}

pub async fn convert_image_reader_native(mut reader: AsyncReadPinBox, format: ImageFormat, quality: Option<u16>, fast: bool) -> RsResult<Vec<u8>> {

    let image = reader_to_image(&mut reader).await?;
    tokio::task::spawn_blocking(move || {
    save_image_native(image, format, quality, fast)
    }).await?

}

pub fn save_image_native(image: ImageAndProfile, format: ImageFormat, quality: Option<u16>, fast: bool) -> RsResult<Vec<u8>>  {
    let data = if format == ImageFormat::WebP { webp::Encoder::from_image(&image.image).map_err(|e| ImageError::UnableToDecodeWebp(e.into()))?
        .encode_simple(false, quality.unwrap_or(80) as f32)?.to_vec()
    } else {
        let mut buffer = Cursor::new(Vec::new());
        let width = image.image.width();
        let height = image.image.height();
        let color = image.image.color();

        if format == ImageFormat::Avif {
            let mut encoder = image::codecs::avif::AvifEncoder::new_with_speed_quality(&mut buffer, if fast {8} else {5}, quality.unwrap_or(80) as u8);
            if let Some(profile) = image.profile {
                encoder.set_icc_profile(profile);
            }
            encoder.write_image(&image.image.into_bytes(), width, height, color.into())?;
        } else if format == ImageFormat::Jpeg {
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buffer, quality.unwrap_or(80) as u8);
            if let Some(profile) = image.profile {
                //println!("Setting icc profile {:?}", profile);
                let r = encoder.set_icc_profile(profile);
                //println!("result! {:?}", r);
            }
            encoder.write_image(&image.image.into_bytes(), width, height, color.into())?;
        } else if format == ImageFormat::Png {
            let mut encoder = image::codecs::png::PngEncoder::new(&mut buffer);
            if let Some(profile) = image.profile {
                //println!("Setting icc profile {:?}", profile);
                let r = encoder.set_icc_profile(profile);
                //println!("result! {:?}", r);
            }
            image.image.write_with_encoder(encoder)?;
        } else {
            image.image.write_to(&mut buffer, format)?;
        }
        buffer.into_inner()
    };
    Ok(data)
}


async fn resize(image: DynamicImage, size: u32) -> RsResult<DynamicImage> {
    let result = tokio::task::spawn_blocking(move || {
        image.thumbnail(size, size)
    }).await?;
    Ok(result)
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExifInfos {
    pub colorspace: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub orientation: Option<String>,
    pub focal: Option<u32>,
    pub model: Option<String>,
    pub lat: Option<f64>,
    pub long: Option<f64>,
    pub date: Option<i64>,

}

pub async fn image_infos<R>(reader: &mut R) -> RsResult<MediaForUpdate>where
    R: AsyncRead + Unpin + ?Sized,   {
        let config = get_config().await;
    
        let mut data = if config.imagesUseIm {
            image_infos_im(reader).await
        } else {
            image_infos_native(reader).await
        }?;
        if data.mp.is_none() {
            if let (Some(width), Some(height)) = (data.width, data.height) {
                data.mp = Some(u32::from(width * height / 1000000));
            }
        }
        
        Ok(data)
}

pub async fn image_infos_native<R>(reader: &mut R) -> RsResult<MediaForUpdate> where R: AsyncRead + Unpin + ?Sized    {
    let mut data = Vec::new();
    let mut reader = tokio::io::BufReader::new(reader);
    reader.read_to_end(&mut data).await?;


        
    
    let exifreader = exif::Reader::new();
    
    let mut update = MediaForUpdate::default();

    let mut asyncbufreader = tokio::io::BufReader::new(std::io::Cursor::new(&data));
    let image = reader_to_image(&mut asyncbufreader).await;
    if let Ok(image) = image {
        update.width = Some(image.image.width());
        update.height = Some(image.image.height());
    }

    let mut bufreader = std::io::BufReader::new(std::io::Cursor::new(data));

    //update.orientation = exif.get_field(Tag::Orientation, In::PRIMARY).map(|f| f.display_value().to_string());
    let exif = exifreader.read_from_container(&mut bufreader);
    if let Ok(exif) = exif {    
        for field in exif.fields() {
            match field.tag {
                Tag::Model => {
                    if let exif::Value::Ascii(_) = field.value {
                        let string_value = format!("{}", field.value.display_as(field.tag)).replace("\"", "");
                        update.model = Some(string_value);
                    }
                }

                Tag::Orientation => {
                    if let exif::Value::Short(ref orientation) = field.value {
                        update.orientation = orientation.first().map(|i| i.to_owned() as u8);
                    }
                }

                Tag::FocalLengthIn35mmFilm => {
                    if let exif::Value::Short(ref x) = field.value {
                        update.focal = x.first().map(|i| i.to_owned().into());
                    }
                }

                Tag::DateTimeOriginal | Tag::DateTime => {
                    if let exif::Value::Ascii(_) = field.value {
                        let string_value = format!("{}", field.value.display_as(field.tag));
                        update.created = Some(
                            Utc.datetime_from_str(string_value.as_str(), "%F %T")?
                                .timestamp_millis(),
                        );
                    }
                }
                Tag::GPSLatitude => {
                    if let exif::Value::Rational(ref x) = field.value {
                        update.lat = Some(to_decimal_coordinate(x));
                    }
                }
                
                Tag::PixelXDimension => {
                    if let exif::Value::Long(ref x) = field.value {
                        update.width = x.first().map(|i| i.to_owned().into());
                    }
                }
                Tag::PixelYDimension => {
                    if let exif::Value::Long(ref x) = field.value {
                        update.height = x.first().map(|i| i.to_owned().into());
                    }
                }
                Tag::PhotographicSensitivity => {
                    if let exif::Value::Short(ref x) = field.value {
                        update.iso = x.first().map(|i| i.to_owned().into());
                    }
                }
                Tag::ExposureTime => {
                    if let exif::Value::Rational(ref x) = field.value {
                        update.sspeed = x.first().map(|i| format!("{}/{}", i.num, i.denom));
                    }
                }
                Tag::FNumber => {
                    if let exif::Value::Rational(ref x) = field.value {
                        update.f_number = x.first().map(|i| ((i.num as f64 / i.denom as f64) * 1000.0).round() / 1000.0);
                    }
                }
            /*  Tag::GPSLatitudeRef => {
                    let string_value = format!("{}", field.value.display_as(field.tag));
                    if let "S" = string_value.as_str() {
                        latitude_sign = -1.0
                    }
                }*/
                Tag::GPSLongitude => {
                    if let exif::Value::Rational(ref x) = field.value {
                        update.long = Some(to_decimal_coordinate(x));
                    }
                }
                /*
                Tag::GPSLongitudeRef => {
                    let string_value = format!("{}", field.value.display_as(field.tag));
                    if let "W" = string_value.as_str() {
                        longitude_sign = -1.0
                    }
                }*/
                _ => {}
            }
        }
    }
    // for f in exif.fields() {
    //     println!("{} {}: {} ({})",
    //                 f.tag, f.ifd_num, f.display_value().with_unit(&exif), f.display_value());
    // }
    let data = bufreader.get_ref().get_ref();
    if (is_heic(data)) {
        let infos = read_heic_infos(data)?;
        update.icc = infos.profile_name;
    }

    Ok(update)
}

pub async fn image_infos_im<R>(reader: &mut R) -> RsResult<MediaForUpdate> where R: AsyncRead + Unpin + ?Sized    {
    let images_infos = ImageCommandBuilder::new().infos(reader).await?;
    let mut update = MediaForUpdate::default();
    if let Some(infos) = images_infos.first() {
        

        


        update.width = Some(infos.image.geometry.width);
        update.height = Some(infos.image.geometry.height);
        update.orientation = infos.image.orientation();
        update.iso = infos.image.iso();
        update.focal = infos.image.focal();
        update.f_number = infos.image.f_number();
        update.model = infos.image.properties.exif_model.clone();
        update.sspeed = infos.image.properties.exif_exposure_time.clone();
        update.icc = infos.image.properties.icc_description.clone();
        update.created = infos.image.created();

        if let Some(color_space) = &infos.image.colorspace {
            update.color_space = Some(color_space.clone());
        }
    }


    Ok(update)
}

fn to_decimal_coordinate(dms: &[exif::Rational]) -> f64 {
    dms[0].to_f64() + dms[1].to_f64() / 60.0 + dms[2].to_f64() / 3600.0
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
        //resize_image_path(&source, &target, 7680).await.unwrap()
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