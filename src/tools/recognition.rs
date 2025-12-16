use crate::error::{RsError, RsResult};
use crate::tools::log::{log_info, LogServiceType};
use futures::StreamExt;
use image::ImageBuffer;
use image::{imageops::FilterType, GenericImage, DynamicImage, GenericImageView, Rgb, RgbImage, Rgba};
use ndarray::{Array, Array4, ArrayD, Axis};
use ort::{GraphOptimizationLevel, Session, SessionInputs, Tensor};
use std::path::Path;
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use imageproc::geometric_transformations::{warp, Interpolation, Projection};
use nalgebra::{Matrix3, Point2, Vector2, Matrix2, SVD};

const MODELS: &[(&str, &str)] = &[
    (
        "det_10g.onnx", 
        "https://huggingface.co/immich-app/buffalo_l/resolve/main/detection/model.onnx"
    ),
    (
        "2d106det.onnx", 
        "https://huggingface.co/fofr/comfyui/resolve/main/insightface/models/buffalo_l/2d106det.onnx"
    ),
    (
        "w600k_r50.onnx", 
        "https://huggingface.co/immich-app/buffalo_l/resolve/main/recognition/model.onnx"
    ),
];

pub async fn ensure_models_exist(models_path: &str) -> RsResult<()> {
    fs::create_dir_all(models_path).await?;

    for (filename, url) in MODELS {
        let final_path = format!("{}/{}", models_path, filename);
        if !Path::new(&final_path).exists() {
            log_info(
                LogServiceType::Source,
                format!("Downloading model: {}", filename),
            );

            // Download to .tmp first to prevent corrupt files on interrupt
            let tmp_path = format!("{}.tmp", final_path);
            match download_file(url, &tmp_path).await {
                Ok(_) => {
                    fs::rename(&tmp_path, &final_path).await?;
                    log_info(
                        LogServiceType::Source,
                        format!("Successfully installed {}", filename),
                    );
                }
                Err(e) => {
                    // Cleanup temp file on failure
                    let _ = fs::remove_file(&tmp_path).await;
                    return Err(e);
                }
            }
        }
    }
    Ok(())
}

async fn download_file(url: &str, dest: &str) -> RsResult<()> {
    let client = reqwest::Client::new();
    let response = client.get(url).send().await?;

    if !response.status().is_success() {
        return Err(RsError::Error(format!(
            "Failed to download model: HTTP {}",
            response.status()
        )));
    }

    let total_size = response.content_length().unwrap_or(0);
    let mut file = fs::File::create(dest).await?;
    let mut stream = response.bytes_stream();
    let mut downloaded: u64 = 0;
    let mut last_log_percent: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;

        if total_size > 0 {
            let percent = (downloaded as f64 / total_size as f64 * 100.0) as u64;
            if percent >= last_log_percent + 10 {
                log_info(
                    LogServiceType::Source,
                    format!("Downloading {}: {}%", dest, percent),
                );
                last_log_percent = percent;
            }
        }
    }

    file.flush().await?;
    Ok(())
}

#[derive(Debug, Clone)]
struct Config {
    name: String,
    min_sizes: Vec<Vec<f32>>, // e.g. [[16, 32], [64, 128], [256, 512]]
    steps: Vec<f32>,          // e.g. [8, 16, 32]
    variance: (f32, f32),     // e.g. (0.1, 0.2)
    clip: bool,
}

#[derive(Debug, Clone, Copy)]
struct Anchor {
    cx: f32,
    cy: f32,
    w: f32,
    h: f32,
}

#[derive(Debug, Clone)]
pub struct BBox {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub confidence: f32,
}

#[derive(Debug, Clone)]
pub struct DetectedFace {
    pub bbox: BBox,
    pub landmarks: Vec<(f32, f32)>, // 106 2D landmarks in face_crop coordinates
    pub pose: (f32, f32, f32),      // (pitch, yaw, roll)
    pub confidence: f32,
    pub embedding: Vec<f32>,
    pub aligned_image: Option<DynamicImage>,
}

#[derive(Clone)]
pub struct FaceRecognitionService {
    detection_session: Arc<Session>,   // det_10g
    alignment_session: Arc<Session>,   // 2d106det
    recognition_session: Arc<Session>, // w600k_r50
}

impl FaceRecognitionService {
    pub async fn new_async(models_path: &str) -> RsResult<Self> {
        // Download missing models first
        ensure_models_exist(models_path).await?;

        // Then load synchronously in spawn_blocking
        let path = models_path.to_string();
        tokio::task::spawn_blocking(move || Self::new(&path))
            .await
            .map_err(|e| RsError::Error(format!("Join error: {}", e)))?
    }

    pub fn new(models_path: &str) -> RsResult<Self> {
        // CRITICAL: Set intra_threads(1) to prevent CPU thrashing
        let detection_session = Session::builder()?
            .with_optimization_level(GraphOptimizationLevel::Level3)?
            .with_intra_threads(1)?
            .commit_from_file(format!("{}/det_10g.onnx", models_path))?;

        let alignment_session = Session::builder()?
            .with_optimization_level(GraphOptimizationLevel::Level3)?
            .with_intra_threads(1)?
            .commit_from_file(format!("{}/2d106det.onnx", models_path))?;

        let recognition_session = Session::builder()?
            .with_optimization_level(GraphOptimizationLevel::Level3)?
            .with_intra_threads(1)?
            .commit_from_file(format!("{}/w600k_r50.onnx", models_path))?;

        Ok(Self {
            detection_session: Arc::new(detection_session),
            alignment_session: Arc::new(alignment_session),
            recognition_session: Arc::new(recognition_session),
        })
    }

    // CRITICAL: Async wrapper using spawn_blocking
    pub async fn detect_and_extract_faces_async(
        &self,
        image: DynamicImage,
    ) -> RsResult<Vec<DetectedFace>> {
        let service = self.clone();
        // We need to clone the image data because DynamicImage is not Send/Sync efficiently if it shares underlying buffers,
        // but normally it owns data. However, passing it to a closure requires move.
        // DynamicImage is Send + Sync.
        let image = image.clone();

        tokio::task::spawn_blocking(move || service.detect_and_extract_faces_blocking(&image))
            .await
            .map_err(|e| RsError::Error(format!("Join error: {}", e)))?
    }

    fn detect_and_extract_faces_blocking(
        &self,
        image: &DynamicImage,
    ) -> RsResult<Vec<DetectedFace>> {
        let detections = self.detect_faces_retinaface(image)?;

        let mut faces = Vec::new();
        for (face_idx, det) in detections.iter().enumerate() {


            
            // Use 30% padding for face crop
            let (mut face_crop, offset_x, offset_y) = FaceRecognitionService::crop_face_with_padding(image, &det.bbox, 0.3)?;
            
            let cropped_landmarks: Vec<(f32, f32)> = det.landmarks
                .iter()
                .map(|(lx, ly)| {
                    (
                        lx - offset_x as f32, // Shift X
                        ly - offset_y as f32  // Shift Y
                    )
                })
                .collect();

            let mut face_crop_rgb = face_crop.to_rgb8();
            for (i, (lx, ly)) in cropped_landmarks.iter().enumerate() {
              let gx = cropped_landmarks[i].0;
              let gy = cropped_landmarks[i].1;
              //println!("Landmark {}: {}, {}", i, gx, gy);
              Self::draw_circle(&mut face_crop_rgb, gx as i32, gy as i32, 12, image::Rgb([255, 255, 255]));  
            }
            /* 
            println!("bbox: {:?}", det.bbox);
            face_crop_rgb.save(&format!("C:\\Users\\arnau\\Downloads\\test\\debug_face_crop_{}.png", face_idx))?;
            */

            let aligned_face_112 = align_face_manual(&face_crop, &cropped_landmarks);
                // aligned_face is now a perfect 112x112 image ready for embedding
            //aligned_face_112.save(format!("C:\\Users\\arnau\\Downloads\\test\\aligned_{}.png", face_idx))?;
            

            let embedding = self.extract_embedding(&aligned_face_112)?;

            let pose: (f32, f32, f32) = estimate_head_pose(&cropped_landmarks);

            faces.push(DetectedFace {
                bbox: det.bbox.clone(),
                landmarks: cropped_landmarks, // Store aligned landmarks for visualization
                pose,
                confidence: det.bbox.confidence,
                embedding,
                aligned_image: Some(aligned_face_112),
            });
        }
        Ok(faces)
    }



    // WRAP FACE CODE


    // Standard ArcFace 112x112 reference coordinates
    const ARCFACE_DST: [[f32; 2]; 5] = [
        [38.2946, 51.6963],  // Left Eye
        [73.5318, 51.5014],  // Right Eye
        [56.0252, 71.7366],  // Nose
        [41.5493, 92.3655],  // Left Mouth
        [70.7299, 92.2041],  // Right Mouth
    ];

    /// Extracts the 5 key landmarks from the 106-point set (InsightFace standard)
    pub fn extract_5_landmarks_from_106(landmarks_106: &[(f32, f32)]) -> [(f32, f32); 5] {
        let avg = |indices: std::ops::Range<usize>| -> (f32, f32) {
            let count = (indices.end - indices.start) as f32;
            let sum = indices.fold((0.0, 0.0), |acc, i| (acc.0 + landmarks_106[i].0, acc.1 + landmarks_106[i].1));
            (sum.0 / count, sum.1 / count)
        };

        [
            avg(33..43),       // Left Eye Center
            avg(87..97),       // Right Eye Center
            landmarks_106[54], // Nose Tip
            landmarks_106[76], // Left Mouth Corner
            landmarks_106[82], // Right Mouth Corner
        ]
    }

    /// Warps the face to the standard 112x112 ArcFace template using Similarity Transform
    pub fn warp_face_standard(
        source_image: &DynamicImage,
        landmarks_106: &[(f32, f32)],
    ) -> Result<DynamicImage, String> {
        
        // 1. Get the 5 source points from the global 106 landmarks
        let src_pts = Self::extract_5_landmarks_from_106(landmarks_106);
        
        // 2. Estimate Similarity Transform Matrix (Scale, Rotation, Translation)
        // Returns a 2x3 matrix as [m0, m1, m2, m3, m4, m5]
        let m = Self::estimate_similarity_transform(&src_pts, &Self::ARCFACE_DST)?;

        // 3. Invert the matrix for mapping destination pixels back to source
        let m_inv = Self::invert_affine_matrix(m)?;

        // 4. Perform the Warp (Bilinear Interpolation)
        let width = 112;
        let height = 112;
        let mut out_img = RgbImage::new(width, height);
        let src_img = source_image.to_rgb8();
        let (src_w, src_h) = src_img.dimensions();

        for y in 0..height {
            for x in 0..width {
                // Apply inverse matrix: [src_x, src_y] = M_inv * [x, y, 1]
                let src_x = m_inv[0] * x as f32 + m_inv[1] * y as f32 + m_inv[2];
                let src_y = m_inv[3] * x as f32 + m_inv[4] * y as f32 + m_inv[5];

                // Sample from source image
                let pixel = Self::bilinear_interpolate(&src_img, src_x, src_y, src_w, src_h);
                out_img.put_pixel(x, y, pixel);
            }
        }

        Ok(DynamicImage::ImageRgb8(out_img))
    }


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
    // --- Minimal Math Helpers (No external crate required) ---

    fn estimate_similarity_transform(src: &[(f32, f32); 5], dst: &[[f32; 2]; 5]) -> Result<[f32; 6], String> {
        let mut src_mean = (0.0, 0.0);
        let mut dst_mean = (0.0, 0.0);
        for i in 0..5 {
            src_mean.0 += src[i].0; src_mean.1 += src[i].1;
            dst_mean.0 += dst[i][0]; dst_mean.1 += dst[i][1];
        }
        src_mean.0 /= 5.0; src_mean.1 /= 5.0;
        dst_mean.0 /= 5.0; dst_mean.1 /= 5.0;

        let mut src_demean = [[0.0; 2]; 5];
        let mut dst_demean = [[0.0; 2]; 5];
        for i in 0..5 {
            src_demean[i] = [src[i].0 - src_mean.0, src[i].1 - src_mean.1];
            dst_demean[i] = [dst[i][0] - dst_mean.0, dst[i][1] - dst_mean.1];
        }

        let mut a = 0.0; 
        let mut b = 0.0;
        let mut d = 0.0;
        for i in 0..5 {
            a += src_demean[i][0] * dst_demean[i][0] + src_demean[i][1] * dst_demean[i][1];
            b += src_demean[i][0] * dst_demean[i][1] - src_demean[i][1] * dst_demean[i][0];
            d += src_demean[i][0].powi(2) + src_demean[i][1].powi(2);
        }

        if d < 1e-6 { return Err("Degenerate points".to_string()); }

        let scale = (a*a + b*b).sqrt() / d;
        let angle = b.atan2(a);
        let cos_a = angle.cos();
        let sin_a = angle.sin();

        // Matrix = [ s*cos  -s*sin  tx ]
        //          [ s*sin   s*cos  ty ]
        let m0 = scale * cos_a;
        let m1 = -scale * sin_a;
        let m3 = scale * sin_a;
        let m4 = scale * cos_a;
        
        let m2 = dst_mean.0 - (m0 * src_mean.0 + m1 * src_mean.1);
        let m5 = dst_mean.1 - (m3 * src_mean.0 + m4 * src_mean.1);

        Ok([m0, m1, m2, m3, m4, m5])
    }

    fn invert_affine_matrix(m: [f32; 6]) -> Result<[f32; 6], String> {
        let det = m[0] * m[4] - m[1] * m[3];
        if det.abs() < 1e-6 { return Err("Matrix singular".to_string()); }
        let inv_det = 1.0 / det;
        
        // Standard 2x3 inversion (assuming bottom row is 0 0 1)
        let i0 = m[4] * inv_det;
        let i1 = -m[1] * inv_det;
        let i2 = (m[1] * m[5] - m[4] * m[2]) * inv_det;
        let i3 = -m[3] * inv_det;
        let i4 = m[0] * inv_det;
        let i5 = (m[3] * m[2] - m[0] * m[5]) * inv_det;

        Ok([i0, i1, i2, i3, i4, i5])
    }

    fn bilinear_interpolate(img: &RgbImage, x: f32, y: f32, w: u32, h: u32) -> Rgb<u8> {
        // Check bounds (with 1px padding for interpolation window)
        if x < 0.0 || x > (w as f32 - 1.0) || y < 0.0 || y > (h as f32 - 1.0) {
            return Rgb([0, 0, 0]); // Zero padding
        }

        let x0 = x.floor() as u32;
        let y0 = y.floor() as u32;
        // Clamp upper bounds to avoid panic at edge
        let x1 = (x0 + 1).min(w - 1);
        let y1 = (y0 + 1).min(h - 1);

        let dx = x - x0 as f32;
        let dy = y - y0 as f32;

        // Direct pixel access (unsafe is faster but safe is fine here)
        let p00 = img.get_pixel(x0, y0).0;
        let p10 = img.get_pixel(x1, y0).0;
        let p01 = img.get_pixel(x0, y1).0;
        let p11 = img.get_pixel(x1, y1).0;

        let mut out = [0u8; 3];
        for i in 0..3 {
            let top = p00[i] as f32 * (1.0 - dx) + p10[i] as f32 * dx;
            let btm = p01[i] as f32 * (1.0 - dx) + p11[i] as f32 * dx;
            out[i] = (top * (1.0 - dy) + btm * dy) as u8;
        }
        Rgb(out)
    }



    //==============================================================================================================







    pub fn detect_faces_retinaface(&self, img: &DynamicImage) -> RsResult<Vec<Detection>> {
        let cfg = Config {
            name: "mobilenet0.25".to_string(),
            min_sizes: vec![vec![16.0, 32.0], vec![64.0, 128.0], vec![256.0, 512.0]],
            steps: vec![8.0, 16.0, 32.0],
            variance: (0.1, 0.2),
            clip: false,
        };

        let target_size = 640;
        let (orig_w, orig_h) = img.dimensions();
        let scale = target_size as f32 / orig_w.max(orig_h) as f32;
        let new_w = (orig_w as f32 * scale) as u32;
        let new_h = (orig_h as f32 * scale) as u32;
        let pad_w = (32 - (new_w % 32)) % 32;
        let pad_h = (32 - (new_h % 32)) % 32;
        let final_w = new_w + pad_w;
        let final_h = new_h + pad_h;

        let resized = img.resize_exact(new_w, new_h, FilterType::Triangle);
        let rgb = resized.to_rgb8();
        let mut input = Array4::<f32>::zeros((1, 3, final_h as usize, final_w as usize));

        for y in 0..new_h {
            for x in 0..new_w {
                let pixel = rgb.get_pixel(x, y);
                input[[0, 0, y as usize, x as usize]] = (pixel[0] as f32 - 127.5) / 128.0;
                input[[0, 1, y as usize, x as usize]] = (pixel[1] as f32 - 127.5) / 128.0;
                input[[0, 2, y as usize, x as usize]] = (pixel[2] as f32 - 127.5) / 128.0;
            }
        }

        let input_tensor_value = Tensor::from_array(input)?;
        let name = self
            .detection_session
            .inputs
            .iter()
            .next()
            .ok_or(RsError::Error("Unable to get input name".to_string()))?
            .name
            .clone();

        let outputs = self
            .detection_session
            .run(ort::inputs![name => input_tensor_value]?)?;

        // SCRFD format: outputs[0,1,2]=scores, outputs[3,4,5]=bbox, outputs[6,7,8]=kps
        // Process all 3 pyramid levels (stride 8, 16, 32)
        let conf_thres = 0.60f32;
        let iou_thres = 0.4f32;
        let strides = [8, 16, 32];
        let fmc = 3; // number of feature map types (scores, bbox, kps)
        
        let input_height = final_h as usize;
        let input_width = final_w as usize;
        
        let mut scores_list = Vec::new();
        let mut bboxes_list = Vec::new();
        let mut kps_list: Vec<Vec<(f32, f32)>> = Vec::new();
        
        // Process each pyramid level
        for level_idx in 0..3 {
            let stride = strides[level_idx];
            let scores_tensor = &outputs[level_idx];
            let bbox_tensor = &outputs[level_idx + fmc];
            let kps_tensor = &outputs[level_idx + fmc * 2]; // keypoints at outputs[6,7,8]
            
            let scores_arr: ArrayD<f32> = scores_tensor.try_extract_tensor()?.to_owned();
            let bbox_arr: ArrayD<f32> = bbox_tensor.try_extract_tensor()?.to_owned();
            let kps_arr: ArrayD<f32> = kps_tensor.try_extract_tensor()?.to_owned();
            
            // Generate anchor centers for this level
            let height = input_height / stride;
            let width = input_width / stride;
            let anchor_centers = generate_anchor_centers(height, width, stride);
            
            // Reshape bbox predictions: [N, 4] format
            let bbox_reshaped = if bbox_arr.shape().len() == 2 && bbox_arr.shape()[1] == 4 {
                bbox_arr
            } else {
                return Err(RsError::Error(format!(
                    "Unexpected bbox shape for level {}: {:?}, expected [N, 4]",
                    level_idx, bbox_arr.shape()
                )));
            };
            
            // Scale bbox predictions by stride (as per Python reference)
            let bbox_scaled = &bbox_reshaped * stride as f32;
            
            // Decode bboxes using distance-based decoding
            let decoded_bboxes = distance2bbox(&anchor_centers, &bbox_scaled);
            
            // Extract scores: [N, 1] format
            let scores_reshaped = if scores_arr.shape().len() == 2 && scores_arr.shape()[1] == 1 {
                scores_arr
            } else {
                return Err(RsError::Error(format!(
                    "Unexpected scores shape for level {}: {:?}, expected [N, 1]",
                    level_idx, scores_arr.shape()
                )));
            };
            
            // Decode keypoints: [N, 10] format (5 keypoints × 2 coords)
            let kps_scaled = &kps_arr * stride as f32;
            let decoded_kps = distance2kps(&anchor_centers, &kps_scaled);
            
            // Filter by confidence threshold and collect
            for i in 0..decoded_bboxes.len().min(scores_reshaped.shape()[0]) {
                let score = scores_reshaped[[i, 0]];
                if score >= conf_thres {
                    scores_list.push(score);
                    bboxes_list.push(decoded_bboxes[i]);
                    if i < decoded_kps.len() {
                        kps_list.push(decoded_kps[i].clone());
                    } else {
                        kps_list.push(vec![]);
                    }
                }
            }
        }
        
        // Sort by confidence (descending)
        let mut indices: Vec<usize> = (0..scores_list.len()).collect();
        indices.sort_by(|&a, &b| scores_list[b].partial_cmp(&scores_list[a]).unwrap_or(std::cmp::Ordering::Equal));
        
        // Convert to Detection format and scale back to original image size
        let mut detections = Vec::new();
        for &idx in &indices {
            let [x1, y1, x2, y2] = bboxes_list[idx];
            // Scale from model input size back to original image size
            let bbox = BBox {
                x1: x1 / scale,
                y1: y1 / scale,
                x2: x2 / scale,
                y2: y2 / scale,
                confidence: scores_list[idx],
            };
            
            // Scale keypoints back to original image size
            let landmarks: Vec<(f32, f32)> = if idx < kps_list.len() {
                kps_list[idx]
                    .iter()
                    .map(|(x, y)| {
                        // First, clip to valid region (removes padding effects)
                        let x_clipped = x.min(new_w as f32);
                        let y_clipped = y.min(new_h as f32);
                        
                        // Then scale back to original image size
                        (x_clipped / scale, y_clipped / scale)
                    })
                    .collect()
            } else {
                vec![]
            };
            
            detections.push(Detection {
                bbox,
                landmarks,
            });
        }
        
        let nms_result = non_max_suppression(detections, iou_thres);
        
        // Post-detection validation: filter out low-quality detections
        let validated: Vec<Detection> = nms_result
            .into_iter()
            .filter(|det| {
                // 1. Minimum face size validation (filter very small faces)
                let w = det.bbox.x2 - det.bbox.x1;
                let h = det.bbox.y2 - det.bbox.y1;
                let min_size = 64.0; // Minimum 32x32 pixels
                if w < min_size || h < min_size {
                    return false;
                }
                
                // 2. Landmark validation (require exactly 5 landmarks for proper alignment)
                // SCRFD/RetinaFace models output 5 keypoints (left eye, right eye, nose, left mouth, right mouth)
                // If we don't have 5 landmarks, the detection is likely poor quality or a false positive
                if det.landmarks.len() != 5 {
                    return false;
                }
                
                // 3. Pose validation (filter extreme poses that are likely false positives)
                let (pitch, yaw, roll) = estimate_head_pose(&det.landmarks);
                
                // Filter extreme poses:
                // - Yaw > 60°: too much profile view (hard to identify)
                // - Pitch > 45°: looking too far up/down
                // - Roll > 30°: head tilted too much
                if yaw.abs() > 60.0 || pitch.abs() > 45.0 || roll.abs() > 30.0 {
                    return false;
                }
                
                true
            })
            .collect();
        
        Ok(validated)
    }

    pub fn crop_face_with_padding(
        image: &DynamicImage,
        bbox: &BBox,
        padding: f32,
    ) -> RsResult<(DynamicImage, f32, f32)> {
        let (width, height) = image.dimensions();
        let w = bbox.x2 - bbox.x1;
        let h = bbox.y2 - bbox.y1;
    
        // Calculate padding
        let pad_w = w * padding;
        let pad_h = h * padding;
    
        // Calculate crop coordinates with boundary checks
        // We keep these as f32 initially for the offset return
        let x1_f = (bbox.x1 - pad_w).max(0.0);
        let y1_f = (bbox.y1 - pad_h).max(0.0);
        let x2_f = (bbox.x2 + pad_w).min(width as f32);
        let y2_f = (bbox.y2 + pad_h).min(height as f32);
    
        let x1 = x1_f as u32;
        let y1 = y1_f as u32;
        let x2 = x2_f as u32;
        let y2 = y2_f as u32;
    
        // Ensure valid crop dimensions
        let crop_w = if x2 > x1 { x2 - x1 } else { 1 };
        let crop_h = if y2 > y1 { y2 - y1 } else { 1 };
    
        // Perform the crop
        let crop = image.crop_imm(x1, y1, crop_w, crop_h);
    
        // Return the crop and the top-left coordinate (x1, y1)
        // This (x1, y1) is the offset you add to local crop landmarks 
        // to get back to global image coordinates.
        Ok((crop, x1 as f32, y1 as f32))
    }

    fn resize_with_padding(img: &DynamicImage, target_size: u32) -> DynamicImage {
        let (w, h) = img.dimensions();
        let max_dim = w.max(h);
    
        // 1. Create a square canvas (black/transparent)
        // Using Rgba<u8> ensures compatibility, init with (0,0,0,0) or (0,0,0,255)
        let mut canvas = image::ImageBuffer::from_pixel(max_dim, max_dim, image::Rgba([0, 0, 0, 0]));
    
        // 2. Calculate offsets to center the image
        let offset_x = (max_dim - w) / 2;
        let offset_y = (max_dim - h) / 2;
    
        // 3. Paste the original image onto the canvas
        // We use copy_from which handles the pasting
        // Note: requires `GenericImage` trait import
        let _ = canvas.copy_from(img, offset_x, offset_y);
    
        // 4. Resize the now-square canvas to target size (192x192)
        let square_img = DynamicImage::ImageRgba8(canvas);
        square_img.resize_exact(target_size, target_size, FilterType::Triangle)
    }

    fn extract_106_landmarks(&self, face_crop: &DynamicImage) -> RsResult<Vec<(f32, f32)>> {
        // 1. Preprocess (Resize to 192x192 if not already, convert to RGB, normalize 0..1)
        // Ensure input is 192x192. If caller already did it, this is cheap.
        let face_crop_192 = if face_crop.width() != 192 || face_crop.height() != 192 {
            println!("Resizing face crop to 192x192");
            face_crop.resize_exact(192, 192, image::imageops::FilterType::Triangle)
        } else {
            face_crop.clone()
        };
    
        let input = preprocess_for_landmark(&face_crop_192);
        let tensor = Tensor::from_array(input)?;
    
        // 2. Run Inference
        // The model typically has one output: [1, 212] (flattened x,y pairs) OR [1, 106, 2]
        let outputs = self.alignment_session.run(ort::inputs![self.alignment_session.inputs[0].name.as_str() => tensor]?)?;
        let output_tensor = outputs[0].try_extract_tensor::<f32>()?;
        
        // 3. Parse Output
        // Handle both [1, 212] and [1, 106, 2] shapes automatically
        let data = output_tensor.as_slice().ok_or(RsError::Error("Failed to convert tensor to slice".to_string()))?;
        
        let mut landmarks = Vec::with_capacity(106);
        
        // Iterate pairwise (x, y)
        // Note: Some models output 0..1 (normalized), some output 0..192 (pixels).
        // Based on your logs, your model outputs PIXELS (0..192).
        for chunk in data.chunks(2) {
            if chunk.len() == 2 {
                let x = (chunk[0] + 1.0) * 96.0; 
                let y = (chunk[1] + 1.0) * 96.0;
                landmarks.push((x, y));
            }
        }
        
    
        if landmarks.len() != 106 {
            return Err(RsError::Error(format!("Expected 106 landmarks, got {}", landmarks.len())));
        }
    
        Ok(landmarks)
    }


    fn extract_embedding(&self, aligned_face: &DynamicImage) -> RsResult<Vec<f32>> {
        // Validate input image dimensions
        let (w, h) = aligned_face.dimensions();
        if w != 112 || h != 112 {
            log_info(
                LogServiceType::Other,
                format!(
                "Warning: Aligned face has unexpected dimensions {}x{}, expected 112x112, resizing",
                w, h
            ),
            );
        }

        // CRITICAL: ArcFace normalization
        let input = preprocess_for_arcface(aligned_face);
        let tensor = Tensor::from_array(input)?;

        // ArcFace input name usually "input" or "data"
        // Let's get the first input name dynamically
        let name = self
            .recognition_session
            .inputs
            .iter()
            .next()
            .ok_or(RsError::Error(
                "Unable to get input name for recognition".to_string(),
            ))?
            .name
            .clone();

        let outputs = self
            .recognition_session
            .run(ort::inputs![name => tensor]?)?;
        let embedding: ArrayD<f32> = outputs[0].try_extract_tensor()?.to_owned();

        // Validate embedding shape
        let embedding_size = embedding.len();
        if embedding_size == 0 {
            return Err(RsError::Error("Embedding tensor is empty".to_string()));
        }
        if embedding_size != 512 {
            log_info(
                LogServiceType::Other,
                format!(
                    "Warning: Embedding size is {} instead of expected 512",
                    embedding_size
                ),
            );
        }

        // L2 normalize for cosine similarity
        let embedding_vec: Vec<f32> = embedding.iter().copied().collect();
        let normalized = l2_normalize(&embedding_vec);

        Ok(normalized)
    }
}

// Helpers

fn compute_feature_maps(image_size: (f32, f32), steps: &[f32]) -> Vec<(usize, usize)> {
    let (img_h, img_w) = image_size;
    steps
        .iter()
        .map(|&step| {
            (
                ((img_h / step).ceil()) as usize,
                ((img_w / step).ceil()) as usize,
            )
        })
        .collect()
}

fn prior_box(cfg: &Config, image_size: (f32, f32)) -> Vec<Anchor> {
    let feature_maps = compute_feature_maps(image_size, &cfg.steps);
    let (img_h, img_w) = image_size;
    let mut anchors = Vec::new();

    for (k, &(fm_h, fm_w)) in feature_maps.iter().enumerate() {
        let min_sizes = &cfg.min_sizes[k];
        let step = cfg.steps[k];
        for i in 0..fm_h {
            for j in 0..fm_w {
                let cx = (j as f32 + 0.5) * step / img_w;
                let cy = (i as f32 + 0.5) * step / img_h;
                for &min_size in min_sizes.iter() {
                    let s_kx = min_size / img_w;
                    let s_ky = min_size / img_h;
                    let mut anchor = Anchor {
                        cx,
                        cy,
                        w: s_kx,
                        h: s_ky,
                    };
                    if cfg.clip {
                        anchor.cx = anchor.cx.max(0.0).min(1.0);
                        anchor.cy = anchor.cy.max(0.0).min(1.0);
                        anchor.w = anchor.w.max(0.0).min(1.0);
                        anchor.h = anchor.h.max(0.0).min(1.0);
                    }
                    anchors.push(anchor);
                }
            }
        }
    }
    anchors
}

// SCRFD distance-based bbox decoding (from Python reference)
// distance format: [N, 4] where columns are [left, top, right, bottom] distances from center
fn distance2bbox(anchor_centers: &[[f32; 2]], distance: &ArrayD<f32>) -> Vec<[f32; 4]> {
    let mut bboxes = Vec::with_capacity(anchor_centers.len());
    let n = distance.shape()[0];
    for (i, &center) in anchor_centers.iter().enumerate() {
        if i >= n {
            break;
        }
        // Distance format: [left, top, right, bottom] from center
        let left = distance[[i, 0]];
        let top = distance[[i, 1]];
        let right = distance[[i, 2]];
        let bottom = distance[[i, 3]];

        let x1 = center[0] - left;
        let y1 = center[1] - top;
        let x2 = center[0] + right;
        let y2 = center[1] + bottom;
        bboxes.push([x1, y1, x2, y2]);
    }
    bboxes
}

// SCRFD keypoint decoding: [N, 10] where 10 = 5 keypoints × 2 (x, y) offsets from anchor
fn distance2kps(anchor_centers: &[[f32; 2]], distance: &ArrayD<f32>) -> Vec<Vec<(f32, f32)>> {
    let mut kps_list = Vec::with_capacity(anchor_centers.len());
    let n = distance.shape()[0];
    let num_kps = if distance.shape().len() > 1 { distance.shape()[1] / 2 } else { 0 };
    
    for (i, &center) in anchor_centers.iter().enumerate() {
        if i >= n {
            break;
        }
        let mut kps = Vec::with_capacity(num_kps);
        for k in 0..num_kps {
            // Keypoint format: offsets from anchor center
            let dx = distance[[i, k * 2]];
            let dy = distance[[i, k * 2 + 1]];
            let x = center[0] + dx;
            let y = center[1] + dy;
            kps.push((x, y));
        }
        kps_list.push(kps);
    }
    kps_list
}

// Generate anchor centers for a pyramid level (SCRFD style)
fn generate_anchor_centers(height: usize, width: usize, stride: usize) -> Vec<[f32; 2]> {
    let num_anchors = 2; // SCRFD uses 2 anchors per location
    let mut centers = Vec::with_capacity(height * width * num_anchors);
    for i in 0..height {
        for j in 0..width {
            let cx = (j as f32 ) * stride as f32;
            let cy = (i as f32) * stride as f32;
            // Add both anchors at same location (SCRFD uses 2 anchors per location)
            centers.push([cx, cy]);
            centers.push([cx, cy]);
        }
    }
    centers
}

fn decode_retinaface_box(
    delta: [f32; 4],
    anchor: Anchor,
    variance: (f32, f32),
    net_input: (f32, f32),
    orig_size: (f32, f32),
    conf: f32,
) -> BBox {
    let (var0, var1) = variance;
    let cx = anchor.cx + delta[0] * var0 * anchor.w;
    let cy = anchor.cy + delta[1] * var0 * anchor.h;
    let w = anchor.w * (delta[2] * var1).exp();
    let h = anchor.h * (delta[3] * var1).exp();

    let x1_net = (cx - w / 2.0) * net_input.0;
    let y1_net = (cy - h / 2.0) * net_input.1;
    let x2_net = (cx + w / 2.0) * net_input.0;
    let y2_net = (cy + h / 2.0) * net_input.1;

    let scale_x = orig_size.0 / net_input.0;
    let scale_y = orig_size.1 / net_input.1;

    let x1 = x1_net * scale_x;
    let y1 = y1_net * scale_y;
    let x2 = x2_net * scale_x;
    let y2 = y2_net * scale_y;

    BBox {
        x1,
        y1,
        x2,
        y2,
        confidence: conf,
    }
}

pub struct Detection {
    pub bbox: BBox,
    pub landmarks: Vec<(f32, f32)>,
}

fn extract_bbox(bbox_tensor: ndarray::ArrayViewD<f32>) -> RsResult<[f32; 4]> {
    let slice = bbox_tensor.as_slice().ok_or("Could not get slice")?;
    slice
        .try_into()
        .map_err(|_| RsError::Error("Slice length is not 4".to_string()))
}

fn softmax_last_dim(x: &ArrayD<f32>) -> ArrayD<f32> {
    let last_axis = Axis(x.ndim() - 1);
    let mut result = x.clone();
    result.map_axis_mut(last_axis, |mut subview| {
        let max_val = subview.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
        subview.iter_mut().for_each(|v| *v = (*v - max_val).exp());
        let sum: f32 = subview.iter().sum();
        subview.iter_mut().for_each(|v| *v /= sum);
    });
    result
}

fn compute_iou(b1: &BBox, b2: &BBox) -> f32 {
    let xx1 = b1.x1.max(b2.x1);
    let yy1 = b1.y1.max(b2.y1);
    let xx2 = b1.x2.min(b2.x2);
    let yy2 = b1.y2.min(b2.y2);

    let w = (xx2 - xx1).max(0.0);
    let h = (yy2 - yy1).max(0.0);
    let inter_area = w * h;
    let area1 = (b1.x2 - b1.x1) * (b1.y2 - b1.y1);
    let area2 = (b2.x2 - b2.x1) * (b2.y2 - b2.y1);
    inter_area / (area1 + area2 - inter_area)
}

fn non_max_suppression(detections: Vec<Detection>, iou_threshold: f32) -> Vec<Detection> {
    let mut dets = detections;
    // Sort by descending confidence
    dets.sort_by(|a, b| {
        b.bbox
            .confidence
            .partial_cmp(&a.bbox.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut picked = Vec::new();

    // Process from highest confidence first
    while !dets.is_empty() {
        let current = dets.remove(0); // Take highest confidence
        dets.retain(|det| compute_iou(&current.bbox, &det.bbox) < iou_threshold);
        picked.push(current);
    }
    picked
}

fn preprocess_for_landmark(img: &DynamicImage) -> Array4<f32> {
    let rgb = img.to_rgb8(); 
    let (w, h) = rgb.dimensions();
    
    let mut input = Array4::<f32>::zeros((1, 3, h as usize, w as usize));

    // RGB Format, Normalized -1.0 to 1.0
    for y in 0..h {
        for x in 0..w {
            let pixel = rgb.get_pixel(x, y);
            // Channel 0 = R, 1 = G, 2 = B (RGB Order)
            // (x / 127.5) - 1.0 is equivalent to (x - 127.5) / 127.5
            
            input[[0, 0, y as usize, x as usize]] = (pixel[0] as f32 / 127.5) - 1.0; 
            input[[0, 1, y as usize, x as usize]] = (pixel[1] as f32 / 127.5) - 1.0; 
            input[[0, 2, y as usize, x as usize]] = (pixel[2] as f32 / 127.5) - 1.0; 
        }
    }
    input
}

// ArcFace Preprocessing
fn preprocess_for_arcface(img: &DynamicImage) -> Array4<f32> {
    // Ensure image is 112x112, resize if necessary
    let rgb = if img.dimensions() != (112, 112) {
        img.resize_exact(112, 112, FilterType::Triangle).to_rgb8()
    } else {
        img.to_rgb8()
    };

    let mut input = Array4::<f32>::zeros((1, 3, 112, 112));

    for y in 0..112 {
        for x in 0..112 {
            let pixel = rgb.get_pixel(x, y);
            // CRITICAL: Normalize to [-1, 1] range
            input[[0, 0, y as usize, x as usize]] = (pixel[0] as f32 - 127.5) / 128.0;
            input[[0, 1, y as usize, x as usize]] = (pixel[1] as f32 - 127.5) / 128.0;
            input[[0, 2, y as usize, x as usize]] = (pixel[2] as f32 - 127.5) / 128.0;
        }
    }
    input
}

fn l2_normalize(v: &[f32]) -> Vec<f32> {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm < 1e-6 {
        v.to_vec()
    } else {
        v.iter().map(|x| x / norm).collect()
    }
}

fn get_5_points_from_106(landmarks: &[(f32, f32)]) -> Vec<(f32, f32)> {
    // Safety: ensure we have enough landmarks
    if landmarks.len() < 106 {
        // Return default centered points if landmarks are invalid
        return vec![
            (38.2946, 51.6963),
            (73.5318, 51.5014),
            (56.0252, 71.7366),
            (41.5493, 92.3655),
            (70.7299, 92.2041),
        ];
    }

    // InsightFace 2d106det landmark indices - CORRECTED based on runtime analysis
    // These indices were determined by finding landmarks closest to ARCFACE_DST positions
    // Verified across multiple faces: [10, 96, 63, 6, 20]
    // NOTE: For face alignment, prefer using SCRFD 5-point keypoints directly
    // as they are more reliable. This function is for 106-landmark-based operations.
    
    // Use the correct indices determined from runtime analysis
    let left_eye = landmarks[10];
    let right_eye = landmarks[96];
    let nose = landmarks[63];
    let left_mouth = landmarks[6];
    let right_mouth = landmarks[20];

    vec![left_eye, right_eye, nose, left_mouth, right_mouth]
}

/// Estimate head pose (pitch, yaw, roll) from 5 facial landmarks using geometric heuristics.
/// Returns (pitch, yaw, roll) in degrees.
/// - Pitch: positive = looking up, negative = looking down
/// - Yaw: positive = turning right, negative = turning left
/// - Roll: positive = tilting right, negative = tilting left
fn estimate_head_pose(landmarks: &[(f32, f32)]) -> (f32, f32, f32) {
    // Safety check
    if landmarks.len() < 5 {
        return (0.0, 0.0, 0.0);
    }

    // Extract key points from 5-point landmark format:
    // [0] = Left Eye, [1] = Right Eye, [2] = Nose, [3] = Left Mouth, [4] = Right Mouth
    let left_eye = landmarks[0];
    let right_eye = landmarks[1];
    let nose = landmarks[2];
    let left_mouth = landmarks[3];
    let right_mouth = landmarks[4];
    let mouth_center = (
        (left_mouth.0 + right_mouth.0) / 2.0,
        (left_mouth.1 + right_mouth.1) / 2.0,
    );
    let eye_center = (
        (left_eye.0 + right_eye.0) / 2.0,
        (left_eye.1 + right_eye.1) / 2.0,
    );

    // Calculate distances
    let dist_nose_to_left_eye =
        ((nose.0 - left_eye.0).powi(2) + (nose.1 - left_eye.1).powi(2)).sqrt();
    let dist_nose_to_right_eye =
        ((nose.0 - right_eye.0).powi(2) + (nose.1 - right_eye.1).powi(2)).sqrt();
    let dist_eyes_to_nose =
        ((eye_center.0 - nose.0).powi(2) + (eye_center.1 - nose.1).powi(2)).sqrt();
    let dist_nose_to_mouth =
        ((nose.0 - mouth_center.0).powi(2) + (nose.1 - mouth_center.1).powi(2)).sqrt();

    // Yaw: Ratio of distance from nose to left eye vs. nose to right eye
    // If left eye is closer, person is turning right (positive yaw)
    // If right eye is closer, person is turning left (negative yaw)
    let yaw_ratio = if dist_nose_to_right_eye > 0.0 {
        (dist_nose_to_left_eye - dist_nose_to_right_eye)
            / (dist_nose_to_left_eye + dist_nose_to_right_eye)
    } else {
        0.0
    };
    // Convert ratio to degrees (empirically calibrated: ratio of 0.3 ≈ 30 degrees)
    let yaw = yaw_ratio * 60.0; // Scale factor to convert to degrees

    // Pitch: Ratio of distance from eyes to nose vs. nose to mouth
    // If eyes-to-nose is smaller relative to nose-to-mouth, person is looking up (positive pitch)
    // If eyes-to-nose is larger relative to nose-to-mouth, person is looking down (negative pitch)
    let pitch_ratio = if dist_nose_to_mouth > 0.0 {
        (dist_eyes_to_nose - dist_nose_to_mouth) / (dist_eyes_to_nose + dist_nose_to_mouth)
    } else {
        0.0
    };
    // Convert ratio to degrees (empirically calibrated)
    let pitch = pitch_ratio * 45.0; // Scale factor to convert to degrees

    // Roll: Angle of the line connecting the eyes relative to horizontal
    let eye_dx = right_eye.0 - left_eye.0;
    let eye_dy = right_eye.1 - left_eye.1;
    let roll_rad = eye_dy.atan2(eye_dx);
    let roll = roll_rad.to_degrees();

    (pitch, yaw, roll)
}

fn average_points(points: &[(f32, f32)]) -> (f32, f32) {
    let sum = points
        .iter()
        .fold((0.0, 0.0), |acc, p| (acc.0 + p.0, acc.1 + p.1));
    (sum.0 / points.len() as f32, sum.1 / points.len() as f32)
}



fn estimate_similarity_transform(src: &[(f32, f32)], dst: &[(f32, f32)]) -> [f32; 6] {
    let n = src.len().min(dst.len());
    if n == 0 {
        // Return identity transform if no points
        return [1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    }

    let n_f = n as f32;
    let mut src_mean = (0.0, 0.0);
    let mut dst_mean = (0.0, 0.0);
    for p in src.iter().take(n) {
        src_mean.0 += p.0;
        src_mean.1 += p.1;
    }
    for p in dst.iter().take(n) {
        dst_mean.0 += p.0;
        dst_mean.1 += p.1;
    }
    src_mean.0 /= n_f;
    src_mean.1 /= n_f;
    dst_mean.0 /= n_f;
    dst_mean.1 /= n_f;

    let mut src_demean = Vec::new();
    let mut dst_demean = Vec::new();
    for p in src.iter().take(n) {
        src_demean.push((p.0 - src_mean.0, p.1 - src_mean.1));
    }
    for p in dst.iter().take(n) {
        dst_demean.push((p.0 - dst_mean.0, p.1 - dst_mean.1));
    }

    let mut sum_a_num = 0.0;
    let mut sum_b_num = 0.0;
    let mut sum_den = 0.0;

    for i in 0..n {
        let x = src_demean[i].0;
        let y = src_demean[i].1;
        let u = dst_demean[i].0;
        let v = dst_demean[i].1;

        sum_a_num += x * u + y * v;
        sum_b_num += x * v - y * u;
        sum_den += x * x + y * y;
    }

    if sum_den.abs() < 1e-6 {
        sum_den = 1.0;
    }

    let a = sum_a_num / sum_den;
    let b = sum_b_num / sum_den;

    let tx = dst_mean.0 - (a * src_mean.0 - b * src_mean.1);
    let ty = dst_mean.1 - (b * src_mean.0 + a * src_mean.1);

    [a, -b, tx, b, a, ty]
}

fn invert_transform(t: &[f32; 6]) -> [f32; 6] {
    let a = t[0];
    let b = t[3];
    let det = a * a + b * b;
    let idet = if det.abs() < 1e-6 { 1.0 } else { 1.0 / det };

    let r00 = a * idet;
    let r01 = b * idet;
    let r10 = -b * idet;
    let r11 = a * idet;

    let tx = t[2];
    let ty = t[5];

    let r02 = -(r00 * tx + r01 * ty);
    let r12 = -(r10 * tx + r11 * ty);

    [r00, r01, r02, r10, r11, r12]
}











// ============== FACE WARPING CODE ==============

// Standard 5 facial points for 112x112 ArcFace model
const REFERENCE_POINTS_112: [[f32; 2]; 5] = [
    [38.2946, 51.6963], // Left Eye
    [73.5318, 51.5014], // Right Eye
    [56.0252, 71.7366], // Nose
    [41.5493, 92.3655], // Left Mouth Corner
    [70.7299, 92.2041], // Right Mouth Corner
];



fn simple_align_matrix(src: &[[f32; 2]], dst: &[[f32; 2]]) -> [f32; 9] {
    // 1. Centroids
    let src_mean_x: f32 = src.iter().map(|p| p[0]).sum::<f32>() / 5.0;
    let src_mean_y: f32 = src.iter().map(|p| p[1]).sum::<f32>() / 5.0;
    let dst_mean_x: f32 = dst.iter().map(|p| p[0]).sum::<f32>() / 5.0;
    let dst_mean_y: f32 = dst.iter().map(|p| p[1]).sum::<f32>() / 5.0;

    // 2. Scale (Distance between eyes)
    // Src Eyes: Index 0 (Left) and 1 (Right)
    let src_eye_dx = src[1][0] - src[0][0];
    let src_eye_dy = src[1][1] - src[0][1];
    let src_eye_dist = (src_eye_dx.powi(2) + src_eye_dy.powi(2)).sqrt();
    
    let dst_eye_dx = dst[1][0] - dst[0][0];
    let dst_eye_dy = dst[1][1] - dst[0][1];
    let dst_eye_dist = (dst_eye_dx.powi(2) + dst_eye_dy.powi(2)).sqrt();
    
    // We want Inverse Scale (Dst -> Src)
    let scale = src_eye_dist / dst_eye_dist;

    // 3. Rotation
    // Calculate angle of eyes in both images
    let angle_src = src_eye_dy.atan2(src_eye_dx);
    let angle_dst = dst_eye_dy.atan2(dst_eye_dx);
    
    // We want to rotate Dst coordinate to match Src orientation
    let rotation = angle_src - angle_dst;
    
    let cos_a = rotation.cos();
    let sin_a = rotation.sin();

    // 4. Construct Inverse Matrix Elements (M_inv)
    // Formula: Src = Scale * Rot * (Dst - DstMean) + SrcMean
    // Expanded:
    // SrcX = s*cos*DstX - s*sin*DstY + Tx
    // SrcY = s*sin*DstX + s*cos*DstY + Ty
    
    let a = scale * cos_a;
    let b = scale * -sin_a; // -sin because standard rotation matrix is [cos -sin; sin cos]
    let d = scale * sin_a;
    let e = scale * cos_a;

    // Translation components
    // Tx = SrcMeanX - (a * DstMeanX + b * DstMeanY)
    // Ty = SrcMeanY - (d * DstMeanX + e * DstMeanY)
    let tx = src_mean_x - (a * dst_mean_x + b * dst_mean_y);
    let ty = src_mean_y - (d * dst_mean_x + e * dst_mean_y);

    //println!("Align Matrix: Scale={:.3}, Rot={:.3}rad, Tx={:.1}, Ty={:.1}", scale, rotation, tx, ty);

    // Return 3x3 Matrix for Projection [a, b, c, d, e, f, 0, 0, 1]
    [
        a, b, tx,
        d, e, ty,
        0.0, 0.0, 1.0
    ]
}
fn umeyama_dst_to_src(src: &[[f32; 2]], dst: &[[f32; 2]]) -> [f32; 9] {
    let n = src.len() as f32;
    
    // 1. Compute centroids
    let src_mean = src.iter().fold(Vector2::zeros(), |acc, p| acc + Vector2::new(p[0], p[1])) / n;
    let dst_mean = dst.iter().fold(Vector2::zeros(), |acc, p| acc + Vector2::new(p[0], p[1])) / n;

    // 2. Compute variance of DST (since we are transforming Dst -> Src)
    let dst_var = dst.iter().fold(0.0, |acc, p| {
        let diff = Vector2::new(p[0], p[1]) - dst_mean;
        acc + diff.norm_squared()
    }) / n;

    // 3. Compute Covariance Matrix (Sigma)
    // Sigma = (1/n) * sum( (Src - SrcMean) * (Dst - DstMean)^T )
    // Note order: Src * Dst^T because we want R such that Src ~ R * Dst
    let mut sigma = Matrix2::zeros();
    for i in 0..src.len() {
        let s = Vector2::new(src[i][0], src[i][1]) - src_mean;
        let d = Vector2::new(dst[i][0], dst[i][1]) - dst_mean;
        sigma += s * d.transpose(); 
    }
    sigma /= n;

    // 4. Compute SVD
    let svd = SVD::new(sigma, true, true);
    let u = svd.u.unwrap();
    let v_t = svd.v_t.unwrap();
    
    // 5. Compute Rotation (R = U * S * V^T)
    let mut s = Matrix2::identity();
    if (u * v_t).determinant() < 0.0 {
        s[(1, 1)] = -1.0;
    }
    let r = u * s * v_t;

    // 6. Compute Scale
    // scale = trace(Sigma * R^T) / Var(Dst)
    // Because we map Dst -> Src, we divide by Dst variance.
    let scale = (sigma * r.transpose()).trace() / dst_var;

    // 7. Compute Translation
    // t = SrcMean - Scale * R * DstMean
    let t = src_mean - scale * r * dst_mean;

    //println!("Umeyama Dst->Src: Scale={:.3}, Tx={:.1}, Ty={:.1}", scale, t.x, t.y);

    // 8. Return 3x3 Matrix [a, b, c, d, e, f, 0, 0, 1]
    let scaled_r = r * scale;
    [
        scaled_r[(0, 0)], scaled_r[(0, 1)], t.x,
        scaled_r[(1, 0)], scaled_r[(1, 1)], t.y,
        0.0,              0.0,              1.0
    ]
}

pub fn align_face_manual(
    image: &DynamicImage,
    landmarks: &[(f32, f32)],
) -> DynamicImage {
    // 1. Convert landmarks to array format [f32; 2]
    let src_pts: Vec<[f32; 2]> = landmarks.iter().map(|(x, y)| [*x, *y]).collect();

    // 2. Calculate Matrix (Dst -> Src) using simple solver
    let matrix = umeyama_dst_to_src(&src_pts, &REFERENCE_POINTS_112);
    // matrix is [a, b, c, d, e, f, 0, 0, 1]

    let (a, b, c) = (matrix[0], matrix[1], matrix[2]);
    let (d, e, f) = (matrix[3], matrix[4], matrix[5]);

    let src_img = image.to_rgba8();
    let mut warped = ImageBuffer::from_pixel(112, 112, Rgba([0, 0, 0, 0]));

    //println!("Manual Warp Debug:");
    //println!("Matrix: a={}, b={}, c={}, d={}, e={}, f={}", a, b, c, d, e, f);
    
    // 3. Iterate Output Pixels
    for y in 0..112 {
        for x in 0..112 {
            // Map Dst(x,y) -> Src(u,v)
            let u = a * (x as f32) + b * (y as f32) + c;
            let v = d * (x as f32) + e * (y as f32) + f;



            // Sample (Bilinear Interpolation)
            if u >= 0.0 && u < (src_img.width() as f32 - 1.0) && v >= 0.0 && v < (src_img.height() as f32 - 1.0) {
                // Manual Bilinear Interpolation
                let u_i = u.floor() as u32;
                let v_i = v.floor() as u32;
                let u_f = u - u.floor();
                let v_f = v - v.floor();

                let p00 = src_img.get_pixel(u_i, v_i);
                let p10 = src_img.get_pixel(u_i + 1, v_i);
                let p01 = src_img.get_pixel(u_i, v_i + 1);
                let p11 = src_img.get_pixel(u_i + 1, v_i + 1);

                // Blend X
                let w00 = (1.0 - u_f) * (1.0 - v_f);
                let w10 = u_f * (1.0 - v_f);
                let w01 = (1.0 - u_f) * v_f;
                let w11 = u_f * v_f;

                let mut pixel = Rgba([0, 0, 0, 255]);
                for c in 0..3 { // RGB channels
                    pixel[c] = (
                        p00[c] as f32 * w00 +
                        p10[c] as f32 * w10 +
                        p01[c] as f32 * w01 +
                        p11[c] as f32 * w11
                    ) as u8;
                }
                warped.put_pixel(x, y, pixel);
            }
        }
    }

    DynamicImage::ImageRgba8(warped)
}

// ============== FACE WARPING CODE END ==============