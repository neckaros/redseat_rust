use std::ffi;
use std::io::Write;
use std::ptr;
use std::ffi::c_void;
use std::slice;

use image::DynamicImage;
use image::RgbImage;
use image::RgbaImage;
use libheif_sys as lh;

use crate::error::RsError;
use crate::error::RsResult;
use crate::tools::image_tools::ImageAndProfile;

fn is_float(heif_image: *mut lh::heif_image) -> bool {
    if heif_image.is_null() {
        return false;
    }

    unsafe {
        //println!("1");
        let colorspace = match lh::heif_image_get_colorspace(heif_image) {
            // Use a safe default if unexpected value
            cs if cs == lh::heif_colorspace_heif_colorspace_RGB => true,
            cs if cs == lh::heif_colorspace_heif_colorspace_YCbCr => false,
            _ => false
        };
        //println!("colorspace {}", colorspace);
   

        let chroma = lh::heif_image_get_chroma_format(heif_image);
        //println!("chroma {}", chroma);

        let bit_depth_r = lh::heif_image_get_bits_per_pixel(heif_image, lh::heif_channel_heif_channel_R);
        
        //println!("bit_depth_r {}", bit_depth_r);
        let bit_depth_g = lh::heif_image_get_bits_per_pixel(heif_image, lh::heif_channel_heif_channel_G);
        //println!("bit_depth_g {}", bit_depth_g);
        let bit_depth_b: i32 = lh::heif_image_get_bits_per_pixel(heif_image, lh::heif_channel_heif_channel_B);
        //println!("bit_depth_b {}", bit_depth_b);
        // Check for floating point conditions
        (bit_depth_r >= 16 || bit_depth_g >= 16 || bit_depth_b >= 16) && 
        colorspace
    }
}

pub fn read_heic_file_to_image(heif_data: &[u8]) -> RsResult<ImageAndProfile> {
        unsafe { lh::heif_init(ptr::null_mut()) };
        let data_len = heif_data.len();
        let ctx = unsafe { lh::heif_context_alloc() };
        if ctx.is_null() {
            return Err(RsError::HeifErrorCode(0))
        }

        let err = unsafe { lh::heif_context_read_from_memory_without_copy(
            ctx,
            heif_data.as_ptr() as *const c_void,
            data_len,
            ptr::null()
        ) };
        if err.code != lh::heif_error_code_heif_error_Ok {
            return Err(RsError::HeifErrorCode(err.code))
        }

        let mut handle = ptr::null_mut();
        let err = unsafe { lh::heif_context_get_primary_image_handle(ctx, &mut handle) };
        if err.code != lh::heif_error_code_heif_error_Ok {
            return Err(RsError::HeifErrorCode(err.code))
        }
        if handle.is_null() {
            return Err(RsError::HeifErrorCode(0))
        }

        let width = unsafe { lh::heif_image_handle_get_width(handle) };
        let height = unsafe { lh::heif_image_handle_get_height(handle) };

        let infos = heic_get_color_profile(handle)?;

        let mut image = ptr::null_mut();

        let options = unsafe { lh::heif_decoding_options_alloc() };
        let err = unsafe { lh::heif_decode_image(
            handle,
            &mut image,
            lh::heif_colorspace_heif_colorspace_RGB,
            lh::heif_chroma_heif_chroma_interleaved_RGB,
            options,
        ) };
        unsafe { lh::heif_decoding_options_free(options) };
        if err.code != lh::heif_error_code_heif_error_Ok {
            return Err(RsError::HeifErrorCode(err.code))
        }
        if image.is_null() {
            return Err(RsError::HeifErrorCode(0))
        }


        // Get image data
        let mut stride = 0;
        let data = unsafe { lh::heif_image_get_plane_readonly(
            image,
            lh::heif_channel_heif_channel_interleaved,
            &mut stride,
        ) };
        
        // We need to copy row by row because stride might be larger than width * 4
        let width = width as usize;
        let height = height as usize;
        let stride = stride as usize;
        let mut buffer = Vec::with_capacity((width * height * 4) as usize);
        
        let src = unsafe { std::slice::from_raw_parts(data, (stride * height) as usize) };
        // Print debug info
        //println!("Image dimensions: {}x{}", width, height);
        //println!("Stride: {}", stride);
        //println!("Source buffer length: {}", src.len());
        //println!("Expected row width in bytes: {}", width * 3);
        // Copy row by row, skipping the padding
        for y in 0..height {
            let row_start = y as usize * stride as usize;
            let row_width = width * 3;
            if row_start + row_width > src.len() {
                println!(
                    "Buffer overflow would occur at row {}. Start: {}, Width: {}, Buffer len: {}", 
                    y, row_start, row_width, src.len()
                );
            }

            let row_data = &src[row_start..row_start + (width as usize * 3)];
            buffer.extend_from_slice(row_data);
        }
        
        let image = RgbImage::from_raw(width as u32, height as u32, buffer)
            .ok_or_else(|| RsError::Error("Failed to create image buffer".to_string()))?;
        //image.save("test_data/testout.jpg");

        unsafe { lh::heif_context_free(ctx) };

        unsafe { lh::heif_deinit() };
        Ok(ImageAndProfile {
            image: DynamicImage::from(image),
            profile: infos.1})
    
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct HeicFileInfos {
    pub width: i32,
    pub height: i32,
    pub profile_name: Option<String>,
    pub profile: Option<Vec<u8>>

}

pub fn read_heic_infos(heif_data: &[u8]) -> RsResult<HeicFileInfos> {
    unsafe { lh::heif_init(ptr::null_mut()) };
    let data_len = heif_data.len();
    let ctx = unsafe { lh::heif_context_alloc() };
    if ctx.is_null() {
        return Err(RsError::HeifErrorCode(0))
    }

    let err = unsafe { lh::heif_context_read_from_memory_without_copy(
        ctx,
        heif_data.as_ptr() as *const c_void,
        data_len,
        ptr::null()
    ) };
    if err.code != lh::heif_error_code_heif_error_Ok {
        return Err(RsError::HeifErrorCode(err.code))
    }
    
    let mut handle = ptr::null_mut();
    let err = unsafe { lh::heif_context_get_primary_image_handle(ctx, &mut handle) };
    if err.code != lh::heif_error_code_heif_error_Ok {
        return Err(RsError::HeifErrorCode(err.code))
    }
    if handle.is_null() {
        return Err(RsError::HeifErrorCode(0))
    }

    let width = unsafe { lh::heif_image_handle_get_width(handle) };
    let height = unsafe { lh::heif_image_handle_get_height(handle) };
    let data = heic_get_color_profile(handle).ok();

    Ok(HeicFileInfos {
        width,
        height,
        profile_name: data.as_ref().and_then(|d| d.0.clone()),
        profile: data.and_then(|d| d.1)
    })
}



fn heic_get_color_profile(handle: *mut libheif_sys::heif_image_handle) -> Result<(Option<String>, Option<Vec<u8>>), crate::Error> {
            let profile_type = unsafe {
        lh::heif_image_handle_get_color_profile_type(handle)
            };
            //println!("Pofile type: {}", profile_type);

        match profile_type {
        lh::heif_color_profile_type_heif_color_profile_type_nclx => {
            // NCLX profile
            let mut nclx_profile: *mut lh::heif_color_profile_nclx = ptr::null_mut();
            let err = unsafe {
                lh::heif_image_handle_get_nclx_color_profile(handle, &mut nclx_profile)
            };
            if err.code == lh::heif_error_code_heif_error_Ok && !nclx_profile.is_null() {
                let nclx = unsafe { *nclx_profile };
                println!("Color space: NCLX profile");
                println!("Color primaries: {}", nclx.color_primaries);
                println!("Transfer characteristics: {}", nclx.transfer_characteristics);
                println!("Matrix coefficients: {}", nclx.matrix_coefficients);
                Ok((Some(nclx.color_primaries.to_string()), None))
            } else {
                Err(RsError::Error(format!("Error retrieving NCLX color profile: {:?}/{:?} : {:?}", err.code , err.subcode, err.message)))
            }
        }
        lh::heif_color_profile_type_heif_color_profile_type_rICC
        | lh::heif_color_profile_type_heif_color_profile_type_prof => {
            // ICC profile
            let BUFFER_SIZE: usize = unsafe { lh::heif_image_handle_get_raw_color_profile_size(handle) }; // Set a reasonable size for the color profile
            let mut buffer = vec![0u8; BUFFER_SIZE];
            let buffer_ptr = buffer.as_mut_ptr() as *mut c_void;
            let err = unsafe {
                lh::heif_image_handle_get_raw_color_profile(handle, buffer_ptr)
            };
            if err.code == lh::heif_error_code_heif_error_Ok && !buffer_ptr.is_null() {
                //println!("Color space: ICC profile (raw data)");
                // Use the buffer as a slice (actual size depends on your application)
                let profile_data = unsafe { slice::from_raw_parts(buffer_ptr as *const u8, BUFFER_SIZE) };
                //println!("First 10 bytes of raw color profile: {:?}", &profile_data[..10]);
                let name = extract_icc_profile_name(profile_data)?;
                Ok((Some(name), Some(profile_data.to_vec())))

            } else {
                Err(RsError::Error(format!("Error retrieving raw color profile: {:?}/{:?} : {:?}", err.code , err.subcode, err.message)))
            }
        }
        _ => {
            Err(RsError::Error("No color profile or unsupported profile type.".to_string()))
        }
            }
}




fn extract_icc_profile_name(raw_profile: &[u8]) -> RsResult<String> {
    // Ensure the profile is large enough to contain the ICC header
    if raw_profile.len() < 128 {
        return Err(RsError::Error("Invalid ICC profile: too small".to_string()));
    }

    // Read the number of tags from the tag table
    let tag_count_offset = 128; // Offset where the tag table starts
    if raw_profile.len() < tag_count_offset + 4 {
        return Err(RsError::Error("Invalid ICC profile: no tag table".to_string()));
    }
    let bytes: [u8; 4] = raw_profile[tag_count_offset..tag_count_offset + 4].try_into()?;
    let tag_count = u32::from_be_bytes(bytes);

    // Parse each tag to find the "desc" tag
    let mut tag_table_offset = tag_count_offset + 4;
    for _ in 0..tag_count {
        if raw_profile.len() < tag_table_offset + 12 {
            return Err(RsError::Error("Invalid ICC profile: malformed tag table".to_string()));
        }

        // Read the tag signature, offset, and size
        let tag_signature = &raw_profile[tag_table_offset..tag_table_offset + 4];
        let tag_offset = u32::from_be_bytes(raw_profile[tag_table_offset + 4..tag_table_offset + 8].try_into()?) as usize;
        let tag_size = u32::from_be_bytes(raw_profile[tag_table_offset + 8..tag_table_offset + 12].try_into()?) as usize;

        // Check if this is the "desc" tag
        if tag_signature == b"desc" {
            if raw_profile.len() < tag_offset + tag_size {
                return Err(RsError::Error("Invalid ICC profile: desc tag out of bounds".to_string()));
            }

            // Parse the "desc" tag
            let desc_data = &raw_profile[tag_offset..tag_offset + tag_size];
            if desc_data.len() < 8 {
                return Err(RsError::Error("Invalid ICC profile: malformed desc tag".to_string()));
            }

            // Read the length of the description string
            let first_name_length = u32::from_be_bytes(desc_data[20..24].try_into()?) as usize;

            // Read the length of the description string
            let first_name_offset = u32::from_be_bytes(desc_data[24..28].try_into()?) as usize;

            // Extract the UTF-16BE string and convert it to UTF-8
            let utf16_data = &desc_data[first_name_offset..first_name_offset + first_name_length];
            let utf16_string: Vec<u16> = utf16_data
                .chunks(2)
                .map(|chunk| u16::from_be_bytes(chunk.try_into().unwrap_or_default()))
                .collect();

            return String::from_utf16(&utf16_string).map_err(|e| RsError::Error("Unable to convert icc profile utf16".to_string()));
        }

        // Move to the next tag
        tag_table_offset += 12;
    }

    return Err(RsError::Error("ICC profile does not contain a desc tag".to_string()));
}