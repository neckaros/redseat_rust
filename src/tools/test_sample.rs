#[cfg(test)]
mod tests {
    use crate::tools::recognition::FaceRecognitionService;
    use image::{DynamicImage, GenericImageView, Rgb, RgbImage};
    use std::fs;
    use std::path::Path;

    /// Draw a filled circle on an RGB image
    fn draw_circle(img: &mut RgbImage, cx: i32, cy: i32, radius: i32, color: Rgb<u8>) {
        let (w, h) = (img.width() as i32, img.height() as i32);
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx * dx + dy * dy <= radius * radius {
                    let px = cx + dx;
                    let py = cy + dy;
                    if px >= 0 && px < w && py >= 0 && py < h {
                        img.put_pixel(px as u32, py as u32, color);
                    }
                }
            }
        }
    }

    /// Draw landmark number near the point
    fn draw_number(img: &mut RgbImage, x: i32, y: i32, num: usize, color: Rgb<u8>) {
        // Simple 3x5 digit patterns
        let digits: [&[u8]; 10] = [
            &[0b111, 0b101, 0b101, 0b101, 0b111], // 0
            &[0b010, 0b110, 0b010, 0b010, 0b111], // 1
            &[0b111, 0b001, 0b111, 0b100, 0b111], // 2
            &[0b111, 0b001, 0b111, 0b001, 0b111], // 3
            &[0b101, 0b101, 0b111, 0b001, 0b001], // 4
            &[0b111, 0b100, 0b111, 0b001, 0b111], // 5
            &[0b111, 0b100, 0b111, 0b101, 0b111], // 6
            &[0b111, 0b001, 0b001, 0b001, 0b001], // 7
            &[0b111, 0b101, 0b111, 0b101, 0b111], // 8
            &[0b111, 0b101, 0b111, 0b001, 0b111], // 9
        ];
        let d = num % 10;
        let (w, h) = (img.width() as i32, img.height() as i32);
        for (row, &bits) in digits[d].iter().enumerate() {
            for col in 0..3 {
                if (bits >> (2 - col)) & 1 == 1 {
                    let px = x + col;
                    let py = y + row as i32;
                    if px >= 0 && px < w && py >= 0 && py < h {
                        img.put_pixel(px as u32, py as u32, color);
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn test_sample_image() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let models = root.join("models");

        let service = FaceRecognitionService::new_async(models.to_str().unwrap())
            .await
            .unwrap();

        //let img_path = Path::new("C:\\Users\\arnau\\Downloads\\IMG_2231.avif");
        let img_path = Path::new("E:\\downloads\\temp\\vXOtUQnmyCD_iUmla_mnL.jpeg");
        println!("Testing image: {:?}", img_path);
        // Create a dummy image for testing if file doesn't exist
        let img = if img_path.exists() {
            image::open(&img_path).expect("Failed to open image")
        } else {
            println!("Test image not found, using dummy image");
            DynamicImage::ImageRgb8(RgbImage::new(640, 640))
        };

        // Run full face extraction process
        let faces = service
            .detect_and_extract_faces_async(img.clone())
            .await
            .expect("Face extraction failed");

        //let img_path = Path::new("C:\\Users\\arnau\\Downloads\\IMG_2231.avif");
        let img_path2 = Path::new("E:\\downloads\\temp\\IMG_3105.jpeg");
        println!("Testing image: {:?}", img_path2);
        let img2 = if img_path2.exists() {
            image::open(&img_path2).expect("Failed to open image")
        } else {
            println!("Test image 2 not found, using dummy image");
            DynamicImage::ImageRgb8(RgbImage::new(640, 640))
        };

        // Run full face extraction process
        let faces2 = service
            .detect_and_extract_faces_async(img2.clone())
            .await
            .expect("Face extraction failed");

        let score =
            crate::model::people::cosine_similarity(&faces2[0].embedding, &faces[0].embedding);
        println!("Similarity Score: {:.4}", score);

        if score > 0.6 {
            println!("MATCH: Same Person!");
        } else {
            println!("Different People.");
        }
    }
}
