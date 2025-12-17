use std::ffi;
use std::ffi::c_void;
use std::io::Write;
use std::ptr;
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
            _ => false,
        };
        //println!("colorspace {}", colorspace);

        let chroma = lh::heif_image_get_chroma_format(heif_image);
        //println!("chroma {}", chroma);

        let bit_depth_r =
            lh::heif_image_get_bits_per_pixel(heif_image, lh::heif_channel_heif_channel_R);

        //println!("bit_depth_r {}", bit_depth_r);
        let bit_depth_g =
            lh::heif_image_get_bits_per_pixel(heif_image, lh::heif_channel_heif_channel_G);
        //println!("bit_depth_g {}", bit_depth_g);
        let bit_depth_b: i32 =
            lh::heif_image_get_bits_per_pixel(heif_image, lh::heif_channel_heif_channel_B);
        //println!("bit_depth_b {}", bit_depth_b);
        // Check for floating point conditions
        (bit_depth_r >= 16 || bit_depth_g >= 16 || bit_depth_b >= 16) && colorspace
    }
}

// RAII Wrappers
struct HeifContext(*mut lh::heif_context);
impl Drop for HeifContext {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() {
                lh::heif_context_free(self.0);
            }
        }
    }
}

struct HeifHandle(*mut lh::heif_image_handle);
impl Drop for HeifHandle {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() {
                lh::heif_image_handle_release(self.0);
            }
        }
    }
}

struct HeifImage(*mut lh::heif_image);
impl Drop for HeifImage {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() {
                lh::heif_image_release(self.0);
            }
        }
    }
}

struct HeifDecodingOptions(*mut lh::heif_decoding_options);
impl Drop for HeifDecodingOptions {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() {
                lh::heif_decoding_options_free(self.0);
            }
        }
    }
}

pub fn read_heic_file_to_image(heif_data: &[u8]) -> RsResult<ImageAndProfile> {
    // Note: heif_init is referenced, but ideally should be called once globally.
    // Calling it here repeatedly is inefficient but likely not the main leak source compared to missing frees.
    // We will keep it but wrap it if we want to be safe, or assume the user accepts the overhead.
    // For leak fixing, focusing on the objects is key.
    unsafe { lh::heif_init(ptr::null_mut()) };

    // Deinit guard (though manual deinit might be problematic in threaded env, we'll leave it for now but ensure it runs)
    struct HeifLibGuard;
    impl Drop for HeifLibGuard {
        fn drop(&mut self) {
            unsafe {
                lh::heif_deinit();
            }
        }
    }
    let _lib_guard = HeifLibGuard;

    let ctx = unsafe { lh::heif_context_alloc() };
    if ctx.is_null() {
        return Err(RsError::HeifErrorCode(0));
    }
    let ctx = HeifContext(ctx);

    let err = unsafe {
        lh::heif_context_read_from_memory_without_copy(
            ctx.0,
            heif_data.as_ptr() as *const c_void,
            heif_data.len(),
            ptr::null(),
        )
    };
    if err.code != lh::heif_error_code_heif_error_Ok {
        return Err(RsError::HeifErrorCode(err.code as i32));
    }

    let mut handle_ptr = ptr::null_mut();
    let err = unsafe { lh::heif_context_get_primary_image_handle(ctx.0, &mut handle_ptr) };
    if err.code != lh::heif_error_code_heif_error_Ok {
        return Err(RsError::HeifErrorCode(err.code as i32));
    }
    if handle_ptr.is_null() {
        return Err(RsError::HeifErrorCode(0));
    }
    let handle = HeifHandle(handle_ptr);

    let width = unsafe { lh::heif_image_handle_get_width(handle.0) };
    let height = unsafe { lh::heif_image_handle_get_height(handle.0) };

    // Pass the raw pointer, careful not to double-free in helper if it took ownership (helper does not take ownership of handle)
    let infos = heic_get_color_profile(handle.0)?;

    let mut image_ptr = ptr::null_mut();
    let options = unsafe { lh::heif_decoding_options_alloc() };
    let options = HeifDecodingOptions(options);

    let err = unsafe {
        lh::heif_decode_image(
            handle.0,
            &mut image_ptr,
            lh::heif_colorspace_heif_colorspace_RGB,
            lh::heif_chroma_heif_chroma_interleaved_RGB,
            options.0,
        )
    };

    if err.code != lh::heif_error_code_heif_error_Ok {
        return Err(RsError::HeifErrorCode(err.code as i32));
    }
    if image_ptr.is_null() {
        return Err(RsError::HeifErrorCode(0));
    }
    let image = HeifImage(image_ptr);

    // Get image data
    let mut stride = 0;
    let data = unsafe {
        lh::heif_image_get_plane_readonly(
            image.0,
            lh::heif_channel_heif_channel_interleaved,
            &mut stride,
        )
    };

    // We need to copy row by row because stride might be larger than width * 4 (actually width * 3 for RGB)
    let width = width as usize;
    let height = height as usize;
    let stride = stride as usize;
    let mut buffer = Vec::with_capacity(width * height * 3);

    let src = unsafe { std::slice::from_raw_parts(data, stride * height) };

    for y in 0..height {
        let row_start = y * stride;
        let row_width = width * 3;

        // Safety check
        if row_start + row_width <= src.len() {
            let row_data = &src[row_start..row_start + row_width];
            buffer.extend_from_slice(row_data);
        }
    }

    let image_buf = RgbImage::from_raw(width as u32, height as u32, buffer)
        .ok_or_else(|| RsError::Error("Failed to create image buffer".to_string()))?;

    Ok(ImageAndProfile {
        image: DynamicImage::from(image_buf),
        profile: infos.1,
    })
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct HeicFileInfos {
    pub width: i32,
    pub height: i32,
    pub profile_name: Option<String>,
    pub profile: Option<Vec<u8>>,
}

pub fn read_heic_infos(heif_data: &[u8]) -> RsResult<HeicFileInfos> {
    unsafe { lh::heif_init(ptr::null_mut()) };
    struct HeifLibGuard;
    impl Drop for HeifLibGuard {
        fn drop(&mut self) {
            unsafe {
                lh::heif_deinit();
            }
        }
    }
    let _lib_guard = HeifLibGuard;

    let ctx = unsafe { lh::heif_context_alloc() };
    if ctx.is_null() {
        return Err(RsError::HeifErrorCode(0));
    }
    let ctx = HeifContext(ctx);

    let err = unsafe {
        lh::heif_context_read_from_memory_without_copy(
            ctx.0,
            heif_data.as_ptr() as *const c_void,
            heif_data.len(),
            ptr::null(),
        )
    };
    if err.code != lh::heif_error_code_heif_error_Ok {
        return Err(RsError::HeifErrorCode(err.code as i32));
    }

    let mut handle_ptr = ptr::null_mut();
    let err = unsafe { lh::heif_context_get_primary_image_handle(ctx.0, &mut handle_ptr) };
    if err.code != lh::heif_error_code_heif_error_Ok {
        return Err(RsError::HeifErrorCode(err.code as i32));
    }
    if handle_ptr.is_null() {
        return Err(RsError::HeifErrorCode(0));
    }
    let handle = HeifHandle(handle_ptr);

    let width = unsafe { lh::heif_image_handle_get_width(handle.0) };
    let height = unsafe { lh::heif_image_handle_get_height(handle.0) };
    let data = heic_get_color_profile(handle.0).ok();

    Ok(HeicFileInfos {
        width,
        height,
        profile_name: data.as_ref().and_then(|d| d.0.clone()),
        profile: data.and_then(|d| d.1),
    })
}

fn heic_get_color_profile(
    handle: *mut libheif_sys::heif_image_handle,
) -> Result<(Option<String>, Option<Vec<u8>>), crate::Error> {
    let profile_type = unsafe { lh::heif_image_handle_get_color_profile_type(handle) };
    //println!("Pofile type: {}", profile_type);

    match profile_type {
        lh::heif_color_profile_type_heif_color_profile_type_nclx => {
            // NCLX profile
            let mut nclx_profile: *mut lh::heif_color_profile_nclx = ptr::null_mut();
            let err =
                unsafe { lh::heif_image_handle_get_nclx_color_profile(handle, &mut nclx_profile) };
            if err.code == lh::heif_error_code_heif_error_Ok && !nclx_profile.is_null() {
                let nclx = unsafe { *nclx_profile };
                println!("Color space: NCLX profile");
                println!("Color primaries: {}", nclx.color_primaries);
                println!(
                    "Transfer characteristics: {}",
                    nclx.transfer_characteristics
                );
                println!("Matrix coefficients: {}", nclx.matrix_coefficients);
                Ok((Some(nclx.color_primaries.to_string()), None))
            } else {
                Err(RsError::Error(format!(
                    "Error retrieving NCLX color profile: {:?}/{:?} : {:?}",
                    err.code, err.subcode, err.message
                )))
            }
        }
        lh::heif_color_profile_type_heif_color_profile_type_rICC
        | lh::heif_color_profile_type_heif_color_profile_type_prof => {
            // ICC profile
            let BUFFER_SIZE: usize =
                unsafe { lh::heif_image_handle_get_raw_color_profile_size(handle) }; // Set a reasonable size for the color profile
            let mut buffer = vec![0u8; BUFFER_SIZE];
            let buffer_ptr = buffer.as_mut_ptr() as *mut c_void;
            let err = unsafe { lh::heif_image_handle_get_raw_color_profile(handle, buffer_ptr) };
            if err.code == lh::heif_error_code_heif_error_Ok && !buffer_ptr.is_null() {
                //println!("Color space: ICC profile (raw data)");
                // Use the buffer as a slice (actual size depends on your application)
                let profile_data =
                    unsafe { slice::from_raw_parts(buffer_ptr as *const u8, BUFFER_SIZE) };
                //println!("First 10 bytes of raw color profile: {:?}", &profile_data[..10]);
                let name = extract_icc_profile_name(profile_data)?;
                Ok((Some(name), Some(profile_data.to_vec())))
            } else {
                Err(RsError::Error(format!(
                    "Error retrieving raw color profile: {:?}/{:?} : {:?}",
                    err.code, err.subcode, err.message
                )))
            }
        }
        _ => Err(RsError::Error(
            "No color profile or unsupported profile type.".to_string(),
        )),
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
        return Err(RsError::Error(
            "Invalid ICC profile: no tag table".to_string(),
        ));
    }
    let bytes: [u8; 4] = raw_profile[tag_count_offset..tag_count_offset + 4].try_into()?;
    let tag_count = u32::from_be_bytes(bytes);

    // Parse each tag to find the "desc" tag
    let mut tag_table_offset = tag_count_offset + 4;
    for _ in 0..tag_count {
        if raw_profile.len() < tag_table_offset + 12 {
            return Err(RsError::Error(
                "Invalid ICC profile: malformed tag table".to_string(),
            ));
        }

        // Read the tag signature, offset, and size
        let tag_signature = &raw_profile[tag_table_offset..tag_table_offset + 4];
        let tag_offset =
            u32::from_be_bytes(raw_profile[tag_table_offset + 4..tag_table_offset + 8].try_into()?)
                as usize;
        let tag_size = u32::from_be_bytes(
            raw_profile[tag_table_offset + 8..tag_table_offset + 12].try_into()?,
        ) as usize;

        // Check if this is the "desc" tag
        if tag_signature == b"desc" {
            if raw_profile.len() < tag_offset + tag_size {
                return Err(RsError::Error(
                    "Invalid ICC profile: desc tag out of bounds".to_string(),
                ));
            }

            // Parse the "desc" tag
            let desc_data = &raw_profile[tag_offset..tag_offset + tag_size];
            if desc_data.len() < 8 {
                return Err(RsError::Error(
                    "Invalid ICC profile: malformed desc tag".to_string(),
                ));
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

            return String::from_utf16(&utf16_string)
                .map_err(|e| RsError::Error("Unable to convert icc profile utf16".to_string()));
        }

        // Move to the next tag
        tag_table_offset += 12;
    }

    return Err(RsError::Error(
        "ICC profile does not contain a desc tag".to_string(),
    ));
}
