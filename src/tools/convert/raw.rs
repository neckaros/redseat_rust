use std::fs::File;
use std::io::Read;
use std::path::Path;

use image::{DynamicImage, ImageBuffer, Rgb, RgbImage};
use quickraw::{data, DemosaicingMethod, Input, Output, Export, OutputType};

use crate::error::{RsError, RsResult};

pub type Rgb16Image = ImageBuffer<Rgb<u16>, Vec<u16>>;
pub struct RawImage {
    pub image: Vec<u8>,
    pub width: usize,
    pub height: usize
}

pub fn is_raw(buffer: &[u8]) -> Option<&'static str> {
    // On vérifie d'abord que le buffer est suffisamment long.
    if buffer.len() < 12 {
        return None;
    }
    let str = String::from_utf8(buffer[4..12].iter().cloned().collect());
    //println!("MAGIC: {:?}", str);
    
    // 1. Vérification pour RAF (Fuji) :
    //    Les fichiers RAF commencent par "FUJIFILM" (8 octets).
    if buffer.len() >= 8 && &buffer[0..8] == b"FUJIFILM" {
        return Some("RAF (Fuji)");
    }
    
    // 2. Vérification pour CR3 (ISO BMFF) :
    //    On vérifie si, à l'offset 4, on trouve "ftyp". Puis on regarde
    //    si les octets 8 à 12 correspondent à "cr3 " ou "CR3 ".
    if buffer.len() >= 12 && &buffer[4..8] == b"ftyp" {
        if &buffer[8..12] == b"cr3 " || &buffer[8..12] == b"CR3 " {
            return Some("CR3");
        } else {
            return Some("ISO BMFF RAW (possible CR3/HEIF)");
        }
    }
    
    // 3. Vérification pour les fichiers RAW basés sur TIFF :
    //    Les fichiers TIFF commencent par "II\* \0" (0x49,0x49,0x2A,0x00)
    //    ou "MM\0*" (0x4D,0x4D,0x00,0x2A).
    if (buffer[0] == 0x49 && buffer[1] == 0x49 && buffer[2] == 0x2A && buffer[3] == 0x00) ||
       (buffer[0] == 0x4D && buffer[1] == 0x4D && buffer[2] == 0x00 && buffer[3] == 0x2A) {
        // Vérification spécifique pour CR2 : on regarde si, à l'offset 8,
        // les deux octets suivants sont "CR".
        if buffer.len() >= 10 && &buffer[8..10] == b"CR" {
            return Some("CR2");
        }
        // Sinon, on renvoie un message générique indiquant qu'il s'agit d'un RAW basé sur TIFF.
        return Some("TIFF-based RAW (NEF, ARW, DNG, ORF, RW2, PEF, etc.)");
    }
    
    // Si aucune des conditions n'est satisfaite, on retourne None.
    None
}


fn load_raw_to_dynamic_image(data: Vec<u8>) -> RsResult<DynamicImage> {
    let demosaicing_method = DemosaicingMethod::Linear;
    let color_space = data::XYZ2SRGB;
    let gamma = data::GAMMA_SRGB;
    let output_type = OutputType::Raw16;
    let auto_crop = false;
    let auto_rotate = false;
    
    let export_job = Export::new(
        Input::ByBuffer(data),
        Output::new(
            demosaicing_method,
            color_space,
            gamma,
            output_type,
            auto_crop,
            auto_rotate,
        ),
    ).unwrap();
    
    let (image, width, height) = export_job.export_16bit_image();

    let image = Rgb16Image::from_raw(width as u32, height as u32, image)
        .ok_or_else(|| RsError::Error("Failed to create image buffer".to_string()))?;
    Ok(DynamicImage::from(image))
}

