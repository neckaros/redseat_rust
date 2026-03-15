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
        assert!(score > 0.6, "Expected same person (score > 0.6), got {:.4}", score);
    }
}
