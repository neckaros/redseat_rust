#[cfg(test)]
mod tests {
    use crate::tools::recognition::FaceRecognitionService;
    use image::{GenericImageView, Rgb, RgbImage, DynamicImage};
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
        let img = image::open(&img_path).expect("Failed to open image");

        // Run full face extraction process
        let faces = service.detect_and_extract_faces_async(img.clone())
            .await
            .expect("Face extraction failed");

        println!("\nFound {} faces (expected 3)", faces.len());

        // Create output directory
        let output_dir = Path::new("C:\\Users\\arnau\\Downloads\\test");
        fs::create_dir_all(output_dir).unwrap();

        // Colors for landmarks: 0=red(left_eye), 1=green(right_eye), 2=blue(nose), 3=yellow(left_mouth), 4=cyan(right_mouth)
        let colors = [
            Rgb([255, 0, 0]),     // 0: left eye - red
            Rgb([0, 255, 0]),     // 1: right eye - green
            Rgb([0, 0, 255]),     // 2: nose - blue
            Rgb([255, 255, 0]),   // 3: left mouth - yellow
            Rgb([0, 255, 255]),   // 4: right mouth - cyan
        ];

        // Save each face and log embedding stats
        for (i, face) in faces.iter().enumerate() {
            println!(
                "Face {}: confidence={:.3}, bbox=({:.1},{:.1}) to ({:.1},{:.1})",
                i, face.confidence, face.bbox.x1, face.bbox.y1, face.bbox.x2, face.bbox.y2
            );

            // Log the 5 key landmarks
            let landmark_names = ["left_eye", "right_eye", "nose", "left_mouth", "right_mouth"];
            let key_indices = [10, 96, 63, 6, 20]; // CORRECTED indices based on runtime analysis
            println!("  Landmarks (indices {:?}):", key_indices);
            for (j, &idx) in key_indices.iter().enumerate() {
                if let Some(pt) = face.landmarks.get(idx) {
                    println!("    {}: idx={} -> ({:.1}, {:.1})", landmark_names[j], idx, pt.0, pt.1);
                }
            }

            // Log embedding statistics
            let embedding = &face.embedding;
            let embedding_mean = embedding.iter().sum::<f32>() / embedding.len() as f32;
            let embedding_variance: f32 = embedding.iter()
                .map(|&x| (x - embedding_mean).powi(2))
                .sum::<f32>() / embedding.len() as f32;
            let embedding_std = embedding_variance.sqrt();
            let embedding_min = embedding.iter().fold(f32::INFINITY, |a, &b| a.min(b));
            let embedding_max = embedding.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
            let embedding_norm: f32 = embedding.iter().map(|&x| x * x).sum::<f32>().sqrt();
            
            println!(
                "  Embedding stats: len={}, mean={:.6}, std={:.6}, min={:.6}, max={:.6}, norm={:.6}",
                embedding.len(), embedding_mean, embedding_std, embedding_min, embedding_max, embedding_norm
            );

            // Use aligned image if available, otherwise crop from original
            let face_image = if let Some(ref aligned) = face.aligned_image {
                aligned.clone()
            } else {
                // Crop and save
                let (width, height) = img.dimensions();
                let x1 = (face.bbox.x1.max(0.0) as u32).min(width);
                let y1 = (face.bbox.y1.max(0.0) as u32).min(height);
                let x2 = (face.bbox.x2.max(0.0) as u32).min(width);
                let y2 = (face.bbox.y2.max(0.0) as u32).min(height);

                let crop_w = if x2 > x1 { x2 - x1 } else { 1 };
                let crop_h = if y2 > y1 { y2 - y1 } else { 1 };
                img.crop_imm(x1, y1, crop_w, crop_h)
            };

            let output_path = output_dir.join(format!(
                "face_{:02}_conf_{:.3}.webp",
                i, face.confidence
            ));
            face_image.save(&output_path).unwrap();
            println!("  Saved aligned to {:?}", output_path);
            
            

            // Also save image with ALL 106 landmarks drawn on the ORIGINAL face crop
            // This helps identify the correct indices for eyes, nose, mouth
            let (width, height) = img.dimensions();
            let bbox = &face.bbox;
            let padding = 0.3;
            let bw = bbox.x2 - bbox.x1;
            let bh = bbox.y2 - bbox.y1;
            let pad_w = bw * padding;
            let pad_h = bh * padding;
            let x1 = (bbox.x1 - pad_w).max(0.0) as u32;
            let y1 = (bbox.y1 - pad_h).max(0.0) as u32;
            let x2 = ((bbox.x2 + pad_w) as u32).min(width);
            let y2 = ((bbox.y2 + pad_h) as u32).min(height);
            let crop_w = if x2 > x1 { x2 - x1 } else { 1 };
            let crop_h = if y2 > y1 { y2 - y1 } else { 1 };
            
            let face_crop = img.crop_imm(x1, y1, crop_w, crop_h);
            let mut face_crop_rgb = face_crop.to_rgb8();
            
            // Color scheme by region (based on typical 106-point layout):
            // 0-32: contour (white)
            // 33-42: left eyebrow (orange)
            // 43-51: right eyebrow (pink)
            // 52-71: nose (blue)
            // 72-87: left eye (red)
            // 88-95: right eye (green)
            // 96-105: mouth (yellow)
            fn get_color_for_idx(idx: usize) -> Rgb<u8> {
                match idx {
                    0..=32 => Rgb([200, 200, 200]),    // contour - gray
                    33..=42 => Rgb([255, 165, 0]),    // left eyebrow - orange
                    43..=51 => Rgb([255, 105, 180]),  // right eyebrow - pink
                    52..=71 => Rgb([0, 100, 255]),    // nose - blue
                    72..=87 => Rgb([255, 0, 0]),      // left eye - red
                    88..=95 => Rgb([0, 255, 0]),      // right eye - green
                    96..=105 => Rgb([255, 255, 0]),   // mouth - yellow
                    _ => Rgb([128, 128, 128]),
                }
            }
            
            // Draw ALL landmarks with their index numbers
            for (idx, pt) in face.landmarks.iter().enumerate() {
                let cx = pt.0 as i32;
                let cy = pt.1 as i32;
                let color = get_color_for_idx(idx);
                draw_circle(&mut face_crop_rgb, cx, cy, 3, color);
                // Draw index number (2-digit)
                let tens = idx / 10;
                let ones = idx % 10;
                draw_number(&mut face_crop_rgb, cx + 5, cy - 2, tens, Rgb([255, 255, 255]));
                draw_number(&mut face_crop_rgb, cx + 9, cy - 2, ones, Rgb([255, 255, 255]));
            }
            
            let landmarks_path = output_dir.join(format!(
                "face_{:02}_all_landmarks.png",
                i
            ));
            DynamicImage::ImageRgb8(face_crop_rgb).save(&landmarks_path).unwrap();
            println!("  Saved ALL landmarks visualization to {:?}", landmarks_path);
            println!("  Color legend: gray=contour(0-32), orange=L-eyebrow(33-42), pink=R-eyebrow(43-51),");
            println!("                blue=nose(52-71), red=L-eye(72-87), green=R-eye(88-95), yellow=mouth(96-105)");
        }

        println!("\nAll faces saved to {:?}", output_dir);
    }
}
