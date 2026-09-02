#[cfg(test)]
mod tests {
    use crate::tools::recognition::FaceRecognitionService;
    use std::path::Path;

    #[tokio::test]
    async fn test_sample_image() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let models = root.join("models");

        let service = FaceRecognitionService::new_async(models.to_str().unwrap())
            .await
            .unwrap();

        let img_path = root.join("test_data").join("face1.jpg");
        let img_path2 = root.join("test_data").join("face2.webp");

        println!("Testing image 1: {:?}", img_path);
        let img = image::open(&img_path).expect("Failed to open face1.jpg");
        let faces = service
            .detect_and_extract_faces_async(img.clone())
            .await
            .expect("Face extraction failed");
        assert!(!faces.is_empty(), "No face detected in face1.jpg");

        println!("Testing image 2: {:?}", img_path2);
        let img2 = image::open(&img_path2).expect("Failed to open face2.webp");
        let faces2 = service
            .detect_and_extract_faces_async(img2.clone())
            .await
            .expect("Face extraction failed");
        assert!(!faces2.is_empty(), "No face detected in face2.webp");

        let score =
            crate::model::people::cosine_similarity(&faces2[0].embedding, &faces[0].embedding);
        println!("Similarity Score: {:.4}", score);
        assert!(
            score > 0.6,
            "Expected same person (score > 0.6), got {:.4}",
            score
        );
    }

    #[tokio::test]
    async fn test_oversized_face_crop_is_detected() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let models = root.join("models");

        let service = FaceRecognitionService::new_async(models.to_str().unwrap())
            .await
            .unwrap();

        let img_path = root.join("test_data").join("face1.jpg");
        let img = image::open(&img_path).expect("Failed to open face1.jpg");
        let faces = service
            .detect_and_extract_faces_async(img.clone())
            .await
            .expect("Face extraction failed");
        assert!(!faces.is_empty(), "No baseline face detected");

        let bbox = &faces[0].bbox;
        let crop_x = bbox.x1.max(0.0) as u32;
        let crop_y = bbox.y1.max(0.0) as u32;
        let crop_w = (bbox.x2 - bbox.x1).min(img.width() as f32 - crop_x as f32) as u32;
        let crop_h = (bbox.y2 - bbox.y1).min(img.height() as f32 - crop_y as f32) as u32;
        let oversized = img.crop_imm(crop_x, crop_y, crop_w.max(1), crop_h.max(1));

        let oversized_faces = service
            .detect_and_extract_faces_async(oversized)
            .await
            .expect("Oversized face extraction failed");

        assert!(
            !oversized_faces.is_empty(),
            "No face detected when the face fills the frame"
        );
    }

    #[tokio::test]
    async fn test_rolled_face_is_detected() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let models = root.join("models");

        let service = FaceRecognitionService::new_async(models.to_str().unwrap())
            .await
            .unwrap();

        let img_path = root.join("test_data").join("face1.jpg");
        let img = image::open(&img_path).expect("Failed to open face1.jpg");
        let rotated = imageproc::geometric_transformations::rotate_about_center(
            &img.to_rgba8(),
            35.0_f32.to_radians(),
            imageproc::geometric_transformations::Interpolation::Bilinear,
            image::Rgba([0, 0, 0, 0]),
        );

        let faces = service
            .detect_and_extract_faces_async(image::DynamicImage::ImageRgba8(rotated))
            .await
            .expect("Rolled face extraction failed");

        assert!(!faces.is_empty(), "No face detected in a rolled image");
    }

    #[tokio::test]
    #[ignore]
    async fn test_external_face_image_is_detected() {
        let img_path = std::env::var("REDSEAT_FACE_TEST_IMAGE")
            .expect("Set REDSEAT_FACE_TEST_IMAGE to the image path to test");
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let models = root.join("models");

        let service = FaceRecognitionService::new_async(models.to_str().unwrap())
            .await
            .unwrap();

        let img = image::open(&img_path).expect("Failed to open REDSEAT_FACE_TEST_IMAGE");
        let faces = service
            .detect_and_extract_faces_async(img)
            .await
            .expect("External face extraction failed");

        assert!(
            !faces.is_empty(),
            "No face detected in REDSEAT_FACE_TEST_IMAGE"
        );
    }
}
