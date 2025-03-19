use core::num;
use std::error::Error;

use image::{imageops::{crop_imm, FilterType}, DynamicImage, GenericImageView, Rgb, RgbImage};
use ndarray::{Array, Array4, ArrayD, Axis};
use ort::{inputs, Environment, GraphOptimizationLevel, Session, SessionBuilder, SessionInputs, SessionOutputs, Tensor};
use imageproc::geometric_transformations::{warp, Interpolation};
use crate::error::{RsError, RsResult};


#[derive(Debug, Clone)]
struct Config {
    name: String,
    min_sizes: Vec<Vec<f32>>, // e.g. [[16, 32], [64, 128], [256, 512]]
    steps: Vec<f32>,         // e.g. [8, 16, 32]
    variance: (f32, f32),    // e.g. (0.1, 0.2)
    clip: bool,
}

/// An anchor (prior) box with normalized coordinates.
#[derive(Debug, Clone, Copy)]
struct Anchor {
    /// Center x (normalized, 0..1)
    cx: f32,
    /// Center y (normalized, 0..1)
    cy: f32,
    /// Width (normalized)
    w: f32,
    /// Height (normalized)
    h: f32,
}

/// Computes the feature map sizes given an input size and steps.
/// Input: image_size as (height, width)
fn compute_feature_maps(image_size: (f32, f32), steps: &[f32]) -> Vec<(usize, usize)> {
    let (img_h, img_w) = image_size;
    steps
        .iter()
        .map(|&step| (((img_h / step).ceil()) as usize, ((img_w / step).ceil()) as usize))
        .collect()
}


fn prior_box(cfg: &Config, image_size: (f32, f32)) -> Vec<Anchor> {
    let feature_maps = compute_feature_maps(image_size, &cfg.steps);
    let (img_h, img_w) = image_size; // image_size: (height, width)
    let mut anchors = Vec::new();

    for (k, &(fm_h, fm_w)) in feature_maps.iter().enumerate() {
        let min_sizes = &cfg.min_sizes[k];
        let step = cfg.steps[k];
        for i in 0..fm_h {
            for j in 0..fm_w {
                // Compute center for this grid cell
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

#[derive(Debug)]
struct BBox {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    confidence: f32,
}

/// Decodes the regression deltas into an absolute bounding box on the original image.
/// 
/// # Arguments
/// 
/// * `delta` - The regression output: [delta_x, delta_y, delta_w, delta_h].
/// * `anchor` - The corresponding anchor (normalized).
/// * `variance` - The variance parameters (e.g. (0.1, 0.2)).
/// * `net_input` - Network input dimensions as (width, height).
/// * `orig_size` - Original image dimensions as (width, height).
/// * `conf` - The detection confidence.
/// 
fn decode_retinaface_box(
    delta: [f32; 4],
    anchor: Anchor,
    variance: (f32, f32),
    net_input: (f32, f32),
    orig_size: (f32, f32),
    conf: f32,
) -> BBox {
    let (var0, var1) = variance;
    // Decode center and size in normalized coordinates.
    let cx = anchor.cx + delta[0] * var0 * anchor.w;
    let cy = anchor.cy + delta[1] * var0 * anchor.h;
    let w = anchor.w * (delta[2] * var1).exp();
    let h = anchor.h * (delta[3] * var1).exp();

    // Convert to network input coordinates.
    let x1_net = (cx - w / 2.0) * net_input.0;
    let y1_net = (cy - h / 2.0) * net_input.1;
    let x2_net = (cx + w / 2.0) * net_input.0;
    let y2_net = (cy + h / 2.0) * net_input.1;

    // Scale from network input to original image.
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

#[derive(Debug)]
struct Detection {
    bbox: BBox,
    // Five landmarks as a vector of (x, y) pairs (normalized to the detection input size).
    landmarks: Vec<(f32, f32)>,
}

pub fn pro(image: DynamicImage) -> RsResult<()> {

    // Load the best-in-class RetinaFace model for face detection and landmark extraction.
    // (This model expects an input image of size 128×128, RGB, normalized to [0,1].)
    let detection_session = Session::builder()?
    .with_optimization_level(GraphOptimizationLevel::Level3)?
        .commit_from_file("retinaface.onnx")?;

   /*let recognition_session = Session::builder()?
    .with_optimization_level(GraphOptimizationLevel::Level3)?
    .commit_from_file("arcface.onnx")?;
 */ 
println!("detect");
    let faces = detect_faces_with_landmarks(&image, &detection_session)?;
    let (img_width, img_height) = image.dimensions();
    let mut i = 0;
    for face in faces {
        i = i + 1;
        let bbox = face.bbox;
        let x1 = (bbox.x1.max(0.0) as u32).min(img_width);
        let y1 = (bbox.y1.max(0.0) as u32).min(img_height);
        let x2 = (bbox.x2.max(0.0) as u32).min(img_width);
        let y2 = (bbox.y2.max(0.0) as u32).min(img_height);

        // Ensure width and height are positive
        let width = if x2 > x1 { x2 - x1 } else { 1 };
        let height = if y2 > y1 { y2 - y1 } else { 1 };

        let cropped = image.crop_imm(x1, y1, width, height);

        cropped.save(format!("temp/{}.jpg", i));
    }


    Ok(())
}

fn extract_bbox(bbox_tensor: ndarray::ArrayViewD<f32>) -> RsResult<[f32; 4]> {
    // Flatten the array to 1D; if it's already 1D this is a no-op.
    let slice = bbox_tensor.as_slice().ok_or("Could not get slice")?;
    // Attempt to convert the slice to an array of length 4.
    slice.try_into().map_err(|_| RsError::Error("Slice length is not 4".to_string()))
}

/// Applies softmax along the last dimension of the input array.
/// This is equivalent to PyTorch's F.softmax(x, dim=-1).
fn softmax_last_dim(x: &ArrayD<f32>) -> ArrayD<f32> {
    // Determine the last axis (i.e. dim = -1)
    let last_axis = Axis(x.ndim() - 1);
    // Create a mutable copy of x.
    let mut result = x.clone();
    // Iterate over subviews along the last axis.
    result.map_axis_mut(last_axis, |mut subview| {
        // Compute the maximum value in the subview for numerical stability.
        let max_val = subview.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
        // Subtract max_val and take the exponential.
        subview.iter_mut().for_each(|v| *v = (*v - max_val).exp());
        // Sum over the last dimension.
        let sum: f32 = subview.iter().sum();
        // Normalize: divide each element by the sum.
        subview.iter_mut().for_each(|v| *v /= sum);
    });
    result
}

/// Runs face detection using the RetinaFace model.
/// The model is assumed to take a 1×3×128×128 float tensor (RGB, normalized to [0,1])
/// and output a tensor of shape [1, N, 15], where each detection is:
/// [x, y, w, h, confidence, l0x, l0y, l1x, l1y, l2x, l2y, l3x, l3y, l4x, l4y].
fn detect_faces_with_landmarks(img: &DynamicImage, session: &Session) -> RsResult<Vec<Detection>> {

    let cfg = Config {
        name: "mobilenet0.25".to_string(),
        min_sizes: vec![vec![16.0, 32.0], vec![64.0, 128.0], vec![256.0, 512.0]],
        steps: vec![8.0, 16.0, 32.0],
        variance: (0.1, 0.2),
        clip: false,
    };


    let input_width = 640;
    let input_height = 608;
    let img_width = img.width();
    let img_height = img.height();
    let resized = img.resize_exact(input_width, input_height, FilterType::CatmullRom);
    let mut input = Array::zeros((1, 608, 640, 3));
	for pixel in resized.pixels() {
		let x = pixel.0 as _;
		let y = pixel.1 as _;
		let [r, g, b, _] = pixel.2.0;
		input[[0, y, x, 0]] = (r as f32) - 104.;
		input[[0, y, x, 1]] = (g as f32) - 117.;
		input[[0, y, x, 2]] = (b as f32) -123.;
	}

    let input_tensor_value = Tensor::from_array(input)?;
    let name = session.inputs.iter().next().ok_or(RsError::Error("Unable to get input name".to_string()))?.name.clone();
    println!("name: {}", name);
    let outputs = session.run(ort::inputs![name => input_tensor_value]?)?;
    // Assume outputs[0] is "output0", outputs[1] is "819", outputs[2] is "818".
    let bbox_tensor = &outputs[0];
    let conf_tensor = &outputs[1];
    let landm_tensor = &outputs[2];



    // Extract the outputs as ndarrays.
    let bbox_array: ndarray::ArrayD<f32> = bbox_tensor.try_extract_tensor()?.to_owned();
    let conf_array: ndarray::ArrayD<f32> = conf_tensor.try_extract_tensor()?.to_owned();
    let landm_array: ndarray::ArrayD<f32> = landm_tensor.try_extract_tensor()?.to_owned();

    let softmax = softmax_last_dim(&conf_array);

    // All outputs have shape [1, N, C]. Retrieve the number of detections (N).
    let num_detections = bbox_array.shape()[1];
    println!("bbox: {:?}, conf {:?}, land {:?}", bbox_array.shape(), conf_array.shape(), landm_array.shape());
    let mut detections = Vec::new();
    let anchors = prior_box(&cfg, (608.0, 640.0));

    println!("anchors size: {}", anchors.len());
    for i in 0..num_detections {
        // Extract the i-th bounding box (shape: [1, 4]). We assume the box is [x, y, width, height].
        let bbox_row = bbox_array.index_axis(ndarray::Axis(1), i);


        let xc = bbox_row[[0, 0]] / (input_width as f32) * (img_width as f32);
		let yc = bbox_row[[0, 1]] / (input_height as f32) * (img_height as f32);
		let w = bbox_row[[0, 2]] / (input_width as f32) * (img_width as f32);
		let h = bbox_row[[0, 3]] / (input_height as f32) * (img_height as f32);

        /*let x = bbox_row[[0, 0]];
        let y = bbox_row[[0, 1]];
        let width_box = bbox_row[[0, 2]];
        let height_box = bbox_row[[0, 3]];**/

        // Extract confidence scores for the i-th detection (shape: [1, 2]).
        let conf_row = softmax.index_axis(ndarray::Axis(1), i);
        // Use the second channel as the face score.
        let confidence = conf_row[[0, 1]];
        if confidence < 0.5 {
            continue; // Skip low-confidence detections.
        }
        let ext_box = extract_bbox(bbox_row.view())?;
        println!("bboxrow {:?}", ext_box);
        let bbox = decode_retinaface_box(ext_box, anchors[i], cfg.variance, (input_width as f32, input_height as f32), (img_width as f32, img_height as f32), confidence);

        println!("bboc {:?}", bbox);
        

        // Extract landmarks for the i-th detection (shape: [1, 10]).
        let landm_row = landm_array.index_axis(ndarray::Axis(1), i);
        let mut landmarks = Vec::new();
        for j in 0..5 {
            let lx = landm_row[[0, j * 2]];
            let ly = landm_row[[0, j * 2 + 1]];
            landmarks.push((lx, ly));
        }
        

        detections.push(Detection { bbox, landmarks });
    }

    let detections = non_max_suppression(detections, 0.5);
    Ok(detections)
}






fn recognize(buffer_image: Vec<u8>) -> Result<(), Box<dyn std::error::Error>> {
    // Load ONNX models
    let detection_session = Session::builder()?
        .with_optimization_level(GraphOptimizationLevel::Level3)?
        .commit_from_file("retinaface.onnx")?;

    let recognition_session = Session::builder()?
        .with_optimization_level(GraphOptimizationLevel::Level3)?
        .commit_from_file("arcface.onnx")?;
    Ok(())

}

fn detect_faces(
    image: &RgbImage,
    detection_session: &Session,
) -> Result<Vec<(f32, f32, f32, f32)>, Box<dyn Error>> {
    let (width, height) = image.dimensions();

    // Normalize image and convert to tensor
    let img_tensor: Vec<f32> = image
        .pixels()
        .flat_map(|p| vec![p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0])
        .collect();

    let input_shape = &[1, 3, height as usize, width as usize];
    //let input_tensor = Array4::from_shape_vec(*input_shape, img_tensor)?.view();

    // Run inference
    //let outputs: SessionOutputs = detection_session.run(inputs!["images".to_string() => input_tensor]?)?;
    //let output = outputs["output0"].try_extract_tensor::<f32>()?.t().into_owned();

    // Parse detections (assuming [num_boxes, 5] format)
    let mut boxes = Vec::new();
    /*for i in (0..detections.len()).step_by(5) {
        let confidence = detections[i + 4];
        if confidence > 0.5 {
            let x1 = detections[i] * width as f32;
            let y1 = detections[i + 1] * height as f32;
            let x2 = detections[i + 2] * width as f32;
            let y2 = detections[i + 3] * height as f32;
            boxes.push((x1, y1, x2, y2));
        }
    }*/
    Ok(boxes)
}


/// Computes the Intersection over Union (IoU) of two bounding boxes.
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

/// Applies non-maximum suppression (NMS) on a list of detections.
/// `detections`: A vector of Detection objects.
/// `iou_threshold`: Detections with IoU greater than this threshold will be suppressed.
/// Returns a filtered vector of Detection.
fn non_max_suppression(detections: Vec<Detection>, iou_threshold: f32) -> Vec<Detection> {
    let mut dets = detections;
    // Sort detections by confidence descending.
    dets.sort_by(|a, b| b.bbox.confidence.partial_cmp(&a.bbox.confidence).unwrap());
    let mut picked = Vec::new();

    while let Some(current) = dets.pop() {
        // Remove any detection that overlaps significantly with `current`
        dets.retain(|det| compute_iou(&current.bbox, &det.bbox) < iou_threshold);
        picked.push(current);
    }
    picked
}



// alignment

/// Computes a 2×3 affine transformation matrix mapping three source points to three target points.
/// The returned matrix is in the form [[a, b, c], [d, e, f]].
fn compute_affine_transform(
    src: &[(f32, f32); 3],
    dst: &[(f32, f32); 3],
) -> Result<[[f32; 3]; 2], Box<dyn Error>> {
    // Build the 3×3 source matrix A with rows [x, y, 1].
    let a = [
        [src[0].0, src[0].1, 1.0],
        [src[1].0, src[1].1, 1.0],
        [src[2].0, src[2].1, 1.0],
    ];
    // Build the destination vectors.
    let bx = [dst[0].0, dst[1].0, dst[2].0];
    let by = [dst[0].1, dst[1].1, dst[2].1];
    
    // Compute the inverse of A.
    let inv_a = invert_3x3(&a)?;
    
    // Compute the affine parameters by multiplying inv_a with bx and by.
    let mut row1 = [0.0; 3];
    let mut row2 = [0.0; 3];
    for i in 0..3 {
        row1[i] = inv_a[i][0] * bx[0] + inv_a[i][1] * bx[1] + inv_a[i][2] * bx[2];
        row2[i] = inv_a[i][0] * by[0] + inv_a[i][1] * by[1] + inv_a[i][2] * by[2];
    }
    Ok([row1, row2])
}

/// Inverts a 3×3 matrix represented as [[f32; 3]; 3].
fn invert_3x3(m: &[[f32; 3]; 3]) -> Result<[[f32; 3]; 3], Box<dyn Error>> {
    let det = m[0][0]*(m[1][1]*m[2][2]-m[1][2]*m[2][1])
            - m[0][1]*(m[1][0]*m[2][2]-m[1][2]*m[2][0])
            + m[0][2]*(m[1][0]*m[2][1]-m[1][1]*m[2][0]);
    if det.abs() < 1e-6 {
        return Err("Matrix is singular".into());
    }
    let inv_det = 1.0 / det;
    let mut inv = [[0.0; 3]; 3];
    inv[0][0] =  (m[1][1]*m[2][2] - m[1][2]*m[2][1]) * inv_det;
    inv[0][1] = -(m[0][1]*m[2][2] - m[0][2]*m[2][1]) * inv_det;
    inv[0][2] =  (m[0][1]*m[1][2] - m[0][2]*m[1][1]) * inv_det;
    inv[1][0] = -(m[1][0]*m[2][2] - m[1][2]*m[2][0]) * inv_det;
    inv[1][1] =  (m[0][0]*m[2][2] - m[0][2]*m[2][0]) * inv_det;
    inv[1][2] = -(m[0][0]*m[1][2] - m[0][2]*m[1][0]) * inv_det;
    inv[2][0] =  (m[1][0]*m[2][1] - m[1][1]*m[2][0]) * inv_det;
    inv[2][1] = -(m[0][0]*m[2][1] - m[0][1]*m[2][0]) * inv_det;
    inv[2][2] =  (m[0][0]*m[1][1] - m[0][1]*m[1][0]) * inv_det;
    Ok(inv)
}

/// Applies the computed affine transformation to the face region (given by the bounding box)
/// and produces an aligned face image of size `out_width`×`out_height`.
fn align_face_affine(
    cropped_image: &DynamicImage,
    bbox: &BBox,
    affine: &[[f32; 3]; 2],
    out_width: u32,
    out_height: u32,
) -> DynamicImage {
    // Extract the face ROI using the detection box.
    let face_roi = cropped_image;
    let face_rgb: RgbImage = face_roi.to_rgb8();
    // Convert our 2×3 affine matrix to a full 3×3 matrix (with the last row [0, 0, 1]).
    let affine_full = [
        [affine[0][0], affine[0][1], affine[0][2]],
        [affine[1][0], affine[1][1], affine[1][2]],
        [0.0,          0.0,          1.0],
    ];

    // Warp the face image using bilinear interpolation.
    //let warped_img = warp_affine(&cropped_image.as_rgb8().ok_or(RsError::Error("Oops".to_string())), affine_full, out_width, out_height);
    //DynamicImage::ImageRgb8(warped_img)
    cropped_image.clone()
}

/// Applies an affine transformation defined by a 3x3 matrix to a point (x, y).
/// Assumes the matrix is of the form:
/// [ a  b  c ]
/// [ d  e  f ]
/// [ 0  0  1 ]
fn affine_transform(x: f32, y: f32, matrix: [[f32; 3]; 3]) -> (f32, f32) {
    let new_x = matrix[0][0] * x + matrix[0][1] * y + matrix[0][2];
    let new_y = matrix[1][0] * x + matrix[1][1] * y + matrix[1][2];
    (new_x, new_y)
}


/// Inverts a 3x3 affine matrix (last row is assumed to be [0, 0, 1]).
/// Returns None if the matrix is singular.
fn invert_affine(matrix: [[f32; 3]; 3]) -> Option<[[f32; 3]; 3]> {
    let a = matrix[0][0];
    let b = matrix[0][1];
    let c = matrix[0][2];
    let d = matrix[1][0];
    let e = matrix[1][1];
    let f = matrix[1][2];
    let det = a * e - b * d;
    if det.abs() < 1e-6 {
        return None;
    }
    let inv_det = 1.0 / det;
    let inv_a = e * inv_det;
    let inv_b = -b * inv_det;
    let inv_d = -d * inv_det;
    let inv_e = a * inv_det;
    let inv_c = -(inv_a * c + inv_b * f);
    let inv_f = -(inv_d * c + inv_e * f);
    Some([
        [inv_a, inv_b, inv_c],
        [inv_d, inv_e, inv_f],
        [0.0,   0.0,   1.0],
    ])
}
/* 
/// Warps an image using an affine transformation matrix.
/// `matrix` is the 3x3 affine transform matrix.
/// `out_width` and `out_height` specify the desired output size.
fn warp_affine(img: &RgbImage, matrix: [[f32; 3]; 3], out_width: u32, out_height: u32) -> RgbImage {
    // Compute the inverse of the affine transformation.
    let inv_matrix = invert_affine(matrix).expect("Matrix must be invertible");
    // The closure maps destination (x,y) to source coordinates.
    warp(img, |x, y| {
        // Note: x and y come in as f32 pixel coordinates in the output image.
        affine_transform(x as f32, y as f32, inv_matrix)
    },
    Interpolation::Bilinear,
    Rgb([0, 0, 0]))
}*/