use std::ffi;
use std::ptr;

use image::RgbImage;
use image::RgbaImage;
use libheif_sys as lh;

use crate::error::RsError;

pub fn read_heic_file_to_image() {
    unsafe {
        lh::heif_init(ptr::null_mut());

        let ctx = lh::heif_context_alloc();
        assert!(!ctx.is_null());

        let c_name = ffi::CString::new("test_data/image.heic").unwrap();
        let err = lh::heif_context_read_from_file(
            ctx,
            c_name.as_ptr(),
            ptr::null()
        );
        assert_eq!(err.code, lh::heif_error_code_heif_error_Ok);

        let mut handle = ptr::null_mut();
        let err = lh::heif_context_get_primary_image_handle(ctx, &mut handle);
        assert_eq!(err.code, lh::heif_error_code_heif_error_Ok);
        assert!(!handle.is_null());

        let width = lh::heif_image_handle_get_width(handle);
        assert_eq!(width, 4284);
        let height = lh::heif_image_handle_get_height(handle);
        assert_eq!(height, 5712);

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
        image.save("test_data/testout.jpg");

        lh::heif_context_free(ctx);

        lh::heif_deinit();
    };
}



#[test]
fn read_heic_file_to_image() {
    unsafe {
        lh::heif_init(ptr::null_mut());

        let ctx = lh::heif_context_alloc();
        assert!(!ctx.is_null());

        let c_name = ffi::CString::new("data/test.heic").unwrap();
        let err = lh::heif_context_read_from_file(
            ctx,
            c_name.as_ptr(),
            ptr::null()
        );
        assert_eq!(err.code, lh::heif_error_code_heif_error_Ok);

        let mut handle = ptr::null_mut();
        let err = lh::heif_context_get_primary_image_handle(ctx, &mut handle);
        assert_eq!(err.code, lh::heif_error_code_heif_error_Ok);
        assert!(!handle.is_null());

        let width = lh::heif_image_handle_get_width(handle);
        assert_eq!(width, 4284);
        let height = lh::heif_image_handle_get_height(handle);
        assert_eq!(height, 5712);

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
        image.save("data/testout.jpg");

        lh::heif_context_free(ctx);

        lh::heif_deinit();
    };
}


#[test]
fn read_and_decode_heic_file() {
    unsafe {
        lh::heif_init(ptr::null_mut());

        let ctx = lh::heif_context_alloc();
        assert!(!ctx.is_null());

        let c_name = ffi::CString::new("data/test.heic").unwrap();
        let err = lh::heif_context_read_from_file(
            ctx,
            c_name.as_ptr(),
            ptr::null()
        );
        assert_eq!(err.code, lh::heif_error_code_heif_error_Ok);

        let mut handle = ptr::null_mut();
        let err = lh::heif_context_get_primary_image_handle(ctx, &mut handle);
        assert_eq!(err.code, lh::heif_error_code_heif_error_Ok);
        assert!(!handle.is_null());

        let width = lh::heif_image_handle_get_width(handle);
        assert_eq!(width, 4284);
        let height = lh::heif_image_handle_get_height(handle);
        assert_eq!(height, 5712);

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

        let colorspace = lh::heif_image_get_colorspace(image);
        assert_eq!(colorspace, lh::heif_colorspace_heif_colorspace_RGB);
        let chroma_format = lh::heif_image_get_chroma_format(image);
        assert_eq!(chroma_format, lh::heif_chroma_heif_chroma_interleaved_RGB);
        let width = lh::heif_image_get_width(
            image,
            lh::heif_channel_heif_channel_interleaved
        );
        assert_eq!(width, 4284);
        let height = lh::heif_image_get_height(
            image,
            lh::heif_channel_heif_channel_interleaved
        );
        assert_eq!(height, 5712);

        lh::heif_context_free(ctx);

        lh::heif_deinit();
    };
}