use std::ffi;
use std::io::Cursor;
use std::io::Write;
use std::ptr;
use std::ffi::c_void;
use std::slice;

use image::DynamicImage;
use image::RgbImage;
use image::RgbaImage;
use jxl_oxide::integration::JxlDecoder;
use libheif_sys as lh;

use crate::error::RsError;
use crate::error::RsResult;
use crate::tools::image_tools::ImageAndProfile;
use image::ImageDecoder;


const JPEG_XL_MAGIC: [u8; 12] = [
    0x00, 0x00, 0x00, 0x0C,
    b'J', b'X', b'L', b' ',
    0x0D, 0x0A, 0x87, 0x0A,
];

pub fn is_jxl(data: &[u8]) -> bool {
    if data.len() < 12 {
        return false;
    }
    data.starts_with(&JPEG_XL_MAGIC)
}

pub fn read_jxl_file_to_image(jxl_data: &[u8]) -> RsResult<ImageAndProfile> {

    let reader = Cursor::new(jxl_data);

    let mut decoder = JxlDecoder::new(reader)?;
    let icc = decoder.icc_profile()?;
    let orientation = decoder.orientation();
    let mut image = DynamicImage::from_decoder(decoder)?;
    if let Ok(orientation) = orientation {
        image.apply_orientation(orientation);
    }
    
    Ok(ImageAndProfile {
        image,
        profile: icc,
    })

}
