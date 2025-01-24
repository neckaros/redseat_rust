use std::ffi;
use std::ptr;
use std::ffi::c_void;

use image::DynamicImage;
use image::RgbImage;
use image::RgbaImage;
use libheif_sys as lh;

use crate::error::RsError;

fn is_float(heif_image: *mut lh::heif_image) -> bool {
    if heif_image.is_null() {
        return false;
    }

    unsafe {
        println!("1");
        let colorspace = match lh::heif_image_get_colorspace(heif_image) {
            // Use a safe default if unexpected value
            cs if cs == lh::heif_colorspace_heif_colorspace_RGB => true,
            cs if cs == lh::heif_colorspace_heif_colorspace_YCbCr => false,
            _ => false
        };
        println!("colorspace {}", colorspace);
   

        let chroma = lh::heif_image_get_chroma_format(heif_image);
        println!("chroma {}", chroma);

        let bit_depth_r = lh::heif_image_get_bits_per_pixel(heif_image, lh::heif_channel_heif_channel_R);
        
        println!("bit_depth_r {}", bit_depth_r);
        let bit_depth_g = lh::heif_image_get_bits_per_pixel(heif_image, lh::heif_channel_heif_channel_G);
        println!("bit_depth_g {}", bit_depth_g);
        let bit_depth_b: i32 = lh::heif_image_get_bits_per_pixel(heif_image, lh::heif_channel_heif_channel_B);
        println!("bit_depth_b {}", bit_depth_b);
        // Check for floating point conditions
        (bit_depth_r >= 16 || bit_depth_g >= 16 || bit_depth_b >= 16) && 
        colorspace
    }
}

pub fn read_heic_file_to_image(heif_data: &[u8]) -> DynamicImage {
    unsafe {
        lh::heif_init(ptr::null_mut());
        let data_len = heif_data.len();
        let ctx = lh::heif_context_alloc();
        assert!(!ctx.is_null());

        let c_name = ffi::CString::new("test_data/image.heic").unwrap();
        let err = lh::heif_context_read_from_memory_without_copy(
            ctx,
            heif_data.as_ptr() as *const c_void,
            data_len,
            ptr::null()
        );
        assert_eq!(err.code, lh::heif_error_code_heif_error_Ok);

        let mut handle = ptr::null_mut();
        let err = lh::heif_context_get_primary_image_handle(ctx, &mut handle);
        assert_eq!(err.code, lh::heif_error_code_heif_error_Ok);
        assert!(!handle.is_null());

        let width = lh::heif_image_handle_get_width(handle);
        let height = lh::heif_image_handle_get_height(handle);

        let mut image = ptr::null_mut();

        let options = lh::heif_decoding_options_alloc();
        let err = lh::heif_decode_image(
            handle,
            &mut image,
            lh::heif_colorspace_heif_colorspace_RGB,
            lh::heif_chroma_heif_chroma_interleaved_RGB,
            options,
        );
        lh::heif_decoding_options_free(options);
        assert_eq!(err.code, lh::heif_error_code_heif_error_Ok);
        assert!(!image.is_null());
        println!("Checking hdr");
        let is_hdr = is_float(image);
        println!("Is HDR? {}", is_hdr);

        // Get image data
        let mut stride = 0;
        let data = lh::heif_image_get_plane_readonly(
            image,
            lh::heif_channel_heif_channel_interleaved,
            &mut stride,
        );
        
        // We need to copy row by row because stride might be larger than width * 4
        let width = width as usize;
        let height = height as usize;
        let stride = stride as usize;
        let mut buffer = Vec::with_capacity((width * height * 4) as usize);
        
        let src = std::slice::from_raw_parts(data, (stride * height) as usize);
        // Print debug info
        println!("Image dimensions: {}x{}", width, height);
        println!("Stride: {}", stride);
        println!("Source buffer length: {}", src.len());
        println!("Expected row width in bytes: {}", width * 3);
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
            .ok_or_else(|| RsError::Error("Failed to create image buffer".to_string())).unwrap();
        //image.save("test_data/testout.jpg");

        lh::heif_context_free(ctx);

        lh::heif_deinit();
        DynamicImage::from(image)
    }
}

