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
    // A picture with no colour profile, or one that cannot be read, is still a picture.
    let profile = heic_get_color_profile(handle.0)
        .ok()
        .and_then(|infos| infos.1);

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
        profile,
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
                // The profile is kept even when its name cannot be read.
                let name = extract_icc_profile_name(profile_data).ok();
                Ok((name, Some(profile_data.to_vec())))
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

/// The description of an ICC profile ("Display P3", "sRGB IEC61966-2.1"...), read
/// from its `desc` tag: a `mluc` record in a version 4 profile, plain ASCII in a
/// version 2 one. Every read is checked: profiles come from files.
fn extract_icc_profile_name(raw_profile: &[u8]) -> RsResult<String> {
    let invalid = |what: &str| RsError::Error(format!("Invalid ICC profile: {what}"));
    let number = |data: &[u8], at: usize| -> Option<usize> {
        let bytes: [u8; 4] = data.get(at..at.checked_add(4)?)?.try_into().ok()?;
        Some(u32::from_be_bytes(bytes) as usize)
    };
    let slice = |data: &'_ [u8], from: usize, length: usize| -> Option<Vec<u8>> {
        Some(data.get(from..from.checked_add(length)?)?.to_vec())
    };

    // The tag table follows the 128 bytes of header: a count, then 12 bytes a tag
    // (signature, offset, size).
    let tag_count = number(raw_profile, 128).ok_or_else(|| invalid("no tag table"))?;
    for tag in 0..tag_count {
        let malformed = || invalid("malformed tag table");
        let entry = tag
            .checked_mul(12)
            .and_then(|offset| offset.checked_add(132))
            .ok_or_else(malformed)?;
        let signature = slice(raw_profile, entry, 4).ok_or_else(malformed)?;
        if signature != b"desc" {
            continue;
        }
        let offset = number(raw_profile, entry + 4).ok_or_else(malformed)?;
        let size = number(raw_profile, entry + 8).ok_or_else(malformed)?;
        let desc =
            slice(raw_profile, offset, size).ok_or_else(|| invalid("desc tag out of bounds"))?;
        let text_out_of_bounds = || invalid("desc text out of bounds");

        return match desc.get(..4) {
            // Records of 12 bytes from byte 16, the first one read: its text is
            // UTF-16BE, its place counted from the start of the tag.
            Some(b"mluc") => {
                let length = number(&desc, 20).ok_or_else(text_out_of_bounds)?;
                let start = number(&desc, 24).ok_or_else(text_out_of_bounds)?;
                let text = slice(&desc, start, length).ok_or_else(text_out_of_bounds)?;
                let units: Vec<u16> = text
                    .chunks_exact(2)
                    .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                    .collect();
                String::from_utf16(&units).map_err(|_| invalid("desc text is not UTF-16"))
            }
            // ASCII from byte 12, its length counting the NUL that ends it.
            Some(b"desc") => {
                let length = number(&desc, 8).ok_or_else(text_out_of_bounds)?;
                let text = slice(&desc, 12, length).ok_or_else(text_out_of_bounds)?;
                Ok(String::from_utf8_lossy(&text)
                    .trim_end_matches('\0')
                    .to_string())
            }
            _ => Err(invalid("unknown type of desc tag")),
        };
    }

    Err(invalid("no desc tag"))
}

#[cfg(test)]
mod tests {
    use super::{extract_icc_profile_name, read_heic_file_to_image};

    /// An ICC profile reduced to what the name is read from: a header, one tag.
    fn profile_with_desc(desc: &[u8]) -> Vec<u8> {
        let mut profile = vec![0u8; 128];
        profile.extend(1u32.to_be_bytes());
        profile.extend(b"desc");
        profile.extend(144u32.to_be_bytes());
        profile.extend((desc.len() as u32).to_be_bytes());
        profile.extend(desc);
        profile
    }

    #[test]
    fn reads_the_name_of_both_versions_of_icc_profile() {
        // Version 2, as the front camera of an iPhone writes it.
        let mut text = b"desc\0\0\0\0".to_vec();
        text.extend(11u32.to_be_bytes());
        text.extend(b"Display P3\0");
        text.extend([0u8; 78]);
        assert_eq!(
            extract_icc_profile_name(&profile_with_desc(&text)).unwrap(),
            "Display P3"
        );

        // Version 4: one record, its text 28 bytes into the tag.
        let name: Vec<u8> = "sRGB".encode_utf16().flat_map(u16::to_be_bytes).collect();
        let mut unicode = b"mluc\0\0\0\0".to_vec();
        unicode.extend(1u32.to_be_bytes());
        unicode.extend(12u32.to_be_bytes());
        unicode.extend(b"enUS");
        unicode.extend((name.len() as u32).to_be_bytes());
        unicode.extend(28u32.to_be_bytes());
        unicode.extend(&name);
        assert_eq!(
            extract_icc_profile_name(&profile_with_desc(&unicode)).unwrap(),
            "sRGB"
        );
    }

    #[test]
    fn a_broken_icc_profile_is_an_error_not_a_panic() {
        let mut text = b"desc\0\0\0\0".to_vec();
        text.extend(11u32.to_be_bytes());
        text.extend(b"Display P3\0");
        let whole = profile_with_desc(&text);
        // Cut anywhere, and with every length field pointing far outside.
        for length in 0..whole.len() {
            assert!(extract_icc_profile_name(&whole[..length]).is_err());
        }
        for field in [136, 140, 152] {
            let mut broken = whole.clone();
            broken[field..field + 4].copy_from_slice(&u32::MAX.to_be_bytes());
            assert!(extract_icc_profile_name(&broken).is_err(), "field {field}");
        }
        // A count of tags that cannot be true does not hide the one that is there.
        let mut counted = whole.clone();
        counted[128..132].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(extract_icc_profile_name(&counted).unwrap(), "Display P3");
        // A version 2 text read as a version 4 record was the panic.
        let mut unicode = text.clone();
        unicode[..4].copy_from_slice(b"mluc");
        assert!(extract_icc_profile_name(&profile_with_desc(&unicode)).is_err());
    }

    #[test]
    fn decodes_bundled_heic_fixture() {
        let data = include_bytes!("../../../test_data/image.heic");
        let decoded = read_heic_file_to_image(data).expect("HEIC fixture should decode");

        assert!(decoded.image.width() > 0);
        assert!(decoded.image.height() > 0);
    }
}
