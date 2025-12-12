use std::sync::Arc;
use std::path::Path;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use futures::StreamExt;
use image::{imageops::FilterType, DynamicImage, GenericImageView, Rgb, RgbImage};
use ndarray::{Array, Array4, ArrayD, Axis};
use ort::{GraphOptimizationLevel, Session, SessionInputs, Tensor};
use crate::error::{RsError, RsResult};
use crate::tools::log::{log_info, LogServiceType};

const MODELS: &[(&str, &str)] = &[
    (
        "det_10g.onnx", 
        "https://huggingface.co/fofr/comfyui/resolve/main/insightface/models/buffalo_l/det_10g.onnx"
    ),
    (
        "2d106det.onnx", 
        "https://huggingface.co/fofr/comfyui/resolve/main/insightface/models/buffalo_l/2d106det.onnx"
    ),
    (
        "w600k_r50.onnx", 
        "https://huggingface.co/fofr/comfyui/resolve/main/insightface/models/buffalo_l/w600k_r50.onnx"
    ),
];

pub async fn ensure_models_exist(models_path: &str) -> RsResult<()> {
    fs::create_dir_all(models_path).await?;
    
    for (filename, url) in MODELS {
        let final_path = format!("{}/{}", models_path, filename);
        if !Path::new(&final_path).exists() {
            log_info(LogServiceType::Source, format!("Downloading model: {}", filename));
            
            // Download to .tmp first to prevent corrupt files on interrupt
            let tmp_path = format!("{}.tmp", final_path);
            match download_file(url, &tmp_path).await {
                Ok(_) => {
                    fs::rename(&tmp_path, &final_path).await?;
                    log_info(LogServiceType::Source, format!("Successfully installed {}", filename));
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
        return Err(RsError::Error(format!("Failed to download model: HTTP {}", response.status())));
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
                log_info(LogServiceType::Source, format!("Downloading {}: {}%", dest, percent));
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
    steps: Vec<f32>,         // e.g. [8, 16, 32]
    variance: (f32, f32),    // e.g. (0.1, 0.2)
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
    pub landmarks: Vec<(f32, f32)>,  // 106 2D landmarks
    pub pose: (f32, f32, f32),       // (pitch, yaw, roll)
    pub confidence: f32,
    pub embedding: Vec<f32>,
    pub aligned_image: Option<DynamicImage>,
}

#[derive(Clone)]
pub struct FaceRecognitionService {
    detection_session: Arc<Session>,   // det_10g
    alignment_session: Arc<Session>,    // 2d106det
    recognition_session: Arc<Session>,  // w600k_r50
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
        image: DynamicImage
    ) -> RsResult<Vec<DetectedFace>> {
        let service = self.clone();
        // We need to clone the image data because DynamicImage is not Send/Sync efficiently if it shares underlying buffers, 
        // but normally it owns data. However, passing it to a closure requires move.
        // DynamicImage is Send + Sync.
        let image = image.clone(); 
        
        tokio::task::spawn_blocking(move || {
            service.detect_and_extract_faces_blocking(&image)
        })
        .await
        .map_err(|e| RsError::Error(format!("Join error: {}", e)))?
    }

    fn detect_and_extract_faces_blocking(&self, image: &DynamicImage) -> RsResult<Vec<DetectedFace>> {
        let detections = self.detect_faces_retinaface(image)?;
        
        let mut faces = Vec::new();
        for det in detections {
            let face_crop = self.crop_face_with_padding(image, &det.bbox, 0.3)?;
            let landmarks = self.extract_106_landmarks(&face_crop)?;
            let aligned = self.align_face_5points(&face_crop, &landmarks)?;
            let embedding = self.extract_embedding(&aligned)?;
            
            faces.push(DetectedFace {
                bbox: det.bbox.clone(),
                landmarks,
                pose: (0.0, 0.0, 0.0),
                confidence: det.bbox.confidence,
                embedding,
                aligned_image: Some(aligned),
            });
        }
        Ok(faces)
    }

    fn detect_faces_retinaface(&self, img: &DynamicImage) -> RsResult<Vec<Detection>> {
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
        
        let resized = img.resize_exact(new_w, new_h, FilterType::CatmullRom);
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
        let name = self.detection_session.inputs.iter().next().ok_or(RsError::Error("Unable to get input name".to_string()))?.name.clone();
        
        let outputs = self.detection_session.run(ort::inputs![name => input_tensor_value]?)?;
        let bbox_tensor = &outputs[0];
        let conf_tensor = &outputs[1];
        let bbox_array: ndarray::ArrayD<f32> = bbox_tensor.try_extract_tensor()?.to_owned();
        let conf_array: ndarray::ArrayD<f32> = conf_tensor.try_extract_tensor()?.to_owned();
        let softmax = softmax_last_dim(&conf_array);
        let num_detections = bbox_array.shape()[1];
        let mut detections = Vec::new();
        let anchors = prior_box(&cfg, (final_h as f32, final_w as f32));
    
        for i in 0..num_detections {
            let bbox_row = bbox_array.index_axis(ndarray::Axis(1), i);
            let conf_row = softmax.index_axis(ndarray::Axis(1), i);
            let confidence = conf_row[[0, 1]];
            
            if confidence < 0.5 { continue; }
            
            let ext_box = extract_bbox(bbox_row.view())?;
            let mut bbox = decode_retinaface_box(ext_box, anchors[i], cfg.variance, (final_w as f32, final_h as f32), (final_w as f32, final_h as f32), confidence);
            
            bbox.x1 = bbox.x1 / scale;
            bbox.y1 = bbox.y1 / scale;
            bbox.x2 = bbox.x2 / scale;
            bbox.y2 = bbox.y2 / scale;
            
            detections.push(Detection { bbox, landmarks: vec![] });
        }
    
        Ok(non_max_suppression(detections, 0.4))
    }

    fn crop_face_with_padding(&self, image: &DynamicImage, bbox: &BBox, padding: f32) -> RsResult<DynamicImage> {
        let (width, height) = image.dimensions();
        let w = bbox.x2 - bbox.x1;
        let h = bbox.y2 - bbox.y1;
        let pad_w = w * padding;
        let pad_h = h * padding;
        let x1 = (bbox.x1 - pad_w).max(0.0) as u32;
        let y1 = (bbox.y1 - pad_h).max(0.0) as u32;
        let x2 = (bbox.x2 + pad_w).min(width as f32) as u32;
        let y2 = (bbox.y2 + pad_h).min(height as f32) as u32;
        let crop_w = if x2 > x1 { x2 - x1 } else { 1 };
        let crop_h = if y2 > y1 { y2 - y1 } else { 1 };
        Ok(image.crop_imm(x1, y1, crop_w, crop_h))
    }

    fn extract_106_landmarks(&self, face_crop: &DynamicImage) -> RsResult<Vec<(f32, f32)>> {
        let resized = face_crop.resize_exact(192, 192, FilterType::Triangle);
        let input = preprocess_for_landmark(&resized);
        let tensor = Tensor::from_array(input)?;
        let name = self.alignment_session.inputs.iter().next().ok_or(RsError::Error("Unable to get input name".to_string()))?.name.clone();
        let outputs = self.alignment_session.run(ort::inputs![name => tensor]?)?;
        let landmarks: ArrayD<f32> = outputs[0].try_extract_tensor()?.to_owned();
        let mut pts = Vec::with_capacity(106);
        let shape = landmarks.shape();
        
        // Handle different output formats from 2d106det
        if shape.len() == 3 && shape[1] == 106 && shape[2] == 2 {
            // Shape: (1, 106, 2)
            for i in 0..106 { 
                pts.push((landmarks[[0, i, 0]], landmarks[[0, i, 1]])); 
            }
        } else if shape.len() == 2 && shape[1] >= 212 {
            // Shape: (1, 212) - flattened
            for i in 0..106 { 
                pts.push((landmarks[[0, i*2]], landmarks[[0, i*2+1]])); 
            }
        } else {
            return Err(RsError::Error(format!(
                "Unexpected landmark output shape: {:?}. Expected (1, 106, 2) or (1, 212)", 
                shape
            )));
        }
        
        // Check if landmarks are normalized to [-1, 1] range (check both x AND y)
        let is_normalized = pts.iter().all(|(x, y)| x.abs() <= 1.5 && y.abs() <= 1.5);
        if is_normalized {
            // Convert from [-1, 1] to [0, 192] pixel coordinates
            for (x, y) in pts.iter_mut() {
                *x = (*x + 1.0) * 96.0;
                *y = (*y + 1.0) * 96.0;
            }
        }
        Ok(pts)
    }

    fn align_face_5points(&self, face_crop: &DynamicImage, landmarks: &[(f32, f32)]) -> RsResult<DynamicImage> {
        let src_points = get_5_points_from_106(landmarks);
        let transform = estimate_similarity_transform(&src_points, &ARCFACE_DST);
        let inverse_transform = invert_transform(&transform);
        let mut out_img = RgbImage::new(112, 112);
        let (w, h) = face_crop.dimensions();
        
        // Safety: ensure we have valid dimensions for bilinear interpolation
        if w < 2 || h < 2 {
            return Ok(DynamicImage::ImageRgb8(out_img));
        }
        
        let max_x = (w - 2) as f32;
        let max_y = (h - 2) as f32;
        
        for out_y in 0..112u32 {
            for out_x in 0..112u32 {
                let src_x = inverse_transform[0] * out_x as f32 + inverse_transform[1] * out_y as f32 + inverse_transform[2];
                let src_y = inverse_transform[3] * out_x as f32 + inverse_transform[4] * out_y as f32 + inverse_transform[5];
                
                if src_x >= 0.0 && src_x <= max_x && src_y >= 0.0 && src_y <= max_y {
                    let x0 = src_x.floor() as u32;
                    let y0 = src_y.floor() as u32;
                    let x1 = (x0 + 1).min(w - 1);
                    let y1 = (y0 + 1).min(h - 1);
                    let dx = src_x - x0 as f32;
                    let dy = src_y - y0 as f32;
                    
                    let p00 = face_crop.get_pixel(x0, y0).0;
                    let p10 = face_crop.get_pixel(x1, y0).0;
                    let p01 = face_crop.get_pixel(x0, y1).0;
                    let p11 = face_crop.get_pixel(x1, y1).0;
                    
                    let mut pixel = [0u8; 3];
                    for c in 0..3 {
                        let val = (1.0-dx)*(1.0-dy)*p00[c] as f32 
                                + dx*(1.0-dy)*p10[c] as f32 
                                + (1.0-dx)*dy*p01[c] as f32 
                                + dx*dy*p11[c] as f32;
                        pixel[c] = val.clamp(0.0, 255.0) as u8;
                    }
                    out_img.put_pixel(out_x, out_y, Rgb(pixel));
                }
            }
        }
        Ok(DynamicImage::ImageRgb8(out_img))
    }

    fn extract_embedding(&self, aligned_face: &DynamicImage) -> RsResult<Vec<f32>> {
        // CRITICAL: ArcFace normalization
        let input = preprocess_for_arcface(aligned_face);
        let tensor = Tensor::from_array(input)?;
        
        // ArcFace input name usually "input" or "data"
        // Let's get the first input name dynamically
        let name = self.recognition_session.inputs.iter().next().ok_or(RsError::Error("Unable to get input name for recognition".to_string()))?.name.clone();

        let outputs = self.recognition_session.run(ort::inputs![name => tensor]?)?;
        let embedding: ArrayD<f32> = outputs[0].try_extract_tensor()?.to_owned();
        
        // L2 normalize for cosine similarity
        let embedding_vec: Vec<f32> = embedding.iter().copied().collect();
        let normalized = l2_normalize(&embedding_vec);
        
        Ok(normalized)
    }
}

// Helpers

fn compute_feature_maps(image_size: (f32, f32), steps: &[f32]) -> Vec<(usize, usize)> {
    let (img_h, img_w) = image_size;
    steps.iter().map(|&step| (((img_h / step).ceil()) as usize, ((img_w / step).ceil()) as usize)).collect()
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
                    let mut anchor = Anchor { cx, cy, w: s_kx, h: s_ky };
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

    BBox { x1, y1, x2, y2, confidence: conf }
}

struct Detection {
    bbox: BBox,
    landmarks: Vec<(f32, f32)>,
}

fn extract_bbox(bbox_tensor: ndarray::ArrayViewD<f32>) -> RsResult<[f32; 4]> {
    let slice = bbox_tensor.as_slice().ok_or("Could not get slice")?;
    slice.try_into().map_err(|_| RsError::Error("Slice length is not 4".to_string()))
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
    dets.sort_by(|a, b| b.bbox.confidence.partial_cmp(&a.bbox.confidence).unwrap_or(std::cmp::Ordering::Equal));
    let mut picked = Vec::new();

    // Process from highest confidence first
    while !dets.is_empty() {
        let current = dets.remove(0); // Take highest confidence
        dets.retain(|det| compute_iou(&current.bbox, &det.bbox) < iou_threshold);
        picked.push(current);
    }
    picked
}

// Landmark Preprocessing
fn preprocess_for_landmark(img: &DynamicImage) -> Array4<f32> {
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();
    let mut input = Array4::<f32>::zeros((1, 3, h as usize, w as usize));
    
    for y in 0..h {
        for x in 0..w {
            let pixel = rgb.get_pixel(x, y);
            input[[0, 0, y as usize, x as usize]] = (pixel[0] as f32 - 127.5) / 128.0;
            input[[0, 1, y as usize, x as usize]] = (pixel[1] as f32 - 127.5) / 128.0;
            input[[0, 2, y as usize, x as usize]] = (pixel[2] as f32 - 127.5) / 128.0;
        }
    }
    input
}

// ArcFace Preprocessing
fn preprocess_for_arcface(img: &DynamicImage) -> Array4<f32> {
    let rgb = img.to_rgb8();
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
    if landmarks.len() < 96 {
        // Return default centered points if landmarks are invalid
        return vec![
            (38.2946, 51.6963),
            (73.5318, 51.5014),
            (56.0252, 71.7366),
            (41.5493, 92.3655),
            (70.7299, 92.2041),
        ];
    }
    
    let left_eye = average_points(&landmarks[33..42]);
    let right_eye = average_points(&landmarks[87..96]);
    let nose = landmarks[54];
    let left_mouth = landmarks[76];
    let right_mouth = landmarks[82];
    vec![left_eye, right_eye, nose, left_mouth, right_mouth]
}

fn average_points(points: &[(f32, f32)]) -> (f32, f32) {
    let sum = points.iter().fold((0.0, 0.0), |acc, p| (acc.0 + p.0, acc.1 + p.1));
    (sum.0 / points.len() as f32, sum.1 / points.len() as f32)
}

// Standard ArcFace 5 points for 112x112
const ARCFACE_DST: [(f32, f32); 5] = [
    (38.2946, 51.6963),
    (73.5318, 51.5014),
    (56.0252, 71.7366),
    (41.5493, 92.3655),
    (70.7299, 92.2041),
];

fn estimate_similarity_transform(src: &[(f32, f32)], dst: &[(f32, f32)]) -> [f32; 6] {
    let n = src.len().min(dst.len());
    if n == 0 {
        // Return identity transform if no points
        return [1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    }
    
    let n_f = n as f32;
    let mut src_mean = (0.0, 0.0);
    let mut dst_mean = (0.0, 0.0);
    for p in src.iter().take(n) { src_mean.0 += p.0; src_mean.1 += p.1; }
    for p in dst.iter().take(n) { dst_mean.0 += p.0; dst_mean.1 += p.1; }
    src_mean.0 /= n_f; src_mean.1 /= n_f;
    dst_mean.0 /= n_f; dst_mean.1 /= n_f;
    
    let mut src_demean = Vec::new();
    let mut dst_demean = Vec::new();
    for p in src.iter().take(n) { src_demean.push((p.0 - src_mean.0, p.1 - src_mean.1)); }
    for p in dst.iter().take(n) { dst_demean.push((p.0 - dst_mean.0, p.1 - dst_mean.1)); }
    
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
    
    if sum_den.abs() < 1e-6 { sum_den = 1.0; }
    
    let a = sum_a_num / sum_den;
    let b = sum_b_num / sum_den;
    
    let tx = dst_mean.0 - (a * src_mean.0 - b * src_mean.1);
    let ty = dst_mean.1 - (b * src_mean.0 + a * src_mean.1);
    
    [a, -b, tx, b, a, ty]
}

fn invert_transform(t: &[f32; 6]) -> [f32; 6] {
    let a = t[0];
    let b = t[3]; 
    let det = a*a + b*b;
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
