use std::error::Error;

use image::RgbImage;
use ndarray::{Array4, Axis};
use ort::{inputs, Environment, GraphOptimizationLevel, Session, SessionBuilder, SessionOutputs};





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
    let input_tensor = Array4::from_shape_vec(*input_shape, img_tensor)?.view();

    // Run inference
    let outputs: SessionOutputs = detection_session.run(inputs!["images".to_string() => input_tensor]?)?;
    let output = outputs["output0"].try_extract_tensor::<f32>()?.t().into_owned();

    // Parse detections (assuming [num_boxes, 5] format)
    let mut boxes = Vec::new();
    for i in (0..detections.len()).step_by(5) {
        let confidence = detections[i + 4];
        if confidence > 0.5 {
            let x1 = detections[i] * width as f32;
            let y1 = detections[i + 1] * height as f32;
            let x2 = detections[i + 2] * width as f32;
            let y2 = detections[i + 3] * height as f32;
            boxes.push((x1, y1, x2, y2));
        }
    }
    Ok(boxes)
}