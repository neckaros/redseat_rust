use std::{fs::read_to_string, io::Read, path::PathBuf, str::FromStr};

use ort::{inputs, GraphOptimizationLevel, Session, SessionOutputs};
use ndarray::{s, Array4, Axis};
use image::{GenericImageView, Rgb, Rgba};
use serde::{Deserialize, Serialize};
use crate::Result;


pub fn predict(buffer_image: Vec<u8>) -> Result<Vec<PredictionTagResult>> {
    let target = PathBuf::from_str("models/wd-v1-4-tags.onnx").expect("unable to set path");
    if target.exists() {
        predict_wd(target, buffer_image)
    } else {
        Err(crate::Error::NoModelFound)
    }
    
}
pub fn predict_wd(path: PathBuf, buffer_image: Vec<u8>) -> Result<Vec<PredictionTagResult>> {
    let mut mapping_path = path.clone();
    mapping_path.set_extension("csv");
    let tags = mapping_wd(mapping_path)?;
    
    let model = Session::builder()?
    .with_optimization_level(GraphOptimizationLevel::Level3)?
    .with_intra_threads(4)?
    .with_model_from_file(path)?;


    let img = image::load_from_memory(&buffer_image)?;
    let resized = img.resize_exact(448, 448, image::imageops::FilterType::Nearest);
    let (width, height) = resized.dimensions();
    let mut img_bgr = image::ImageBuffer::new(width, height);


    for y in 0..height {
        for x in 0..width {
            let Rgba([r, g, b, _a]) = resized.get_pixel(x, y);
            img_bgr.put_pixel(x, y, Rgb([b, g, r]));
        }
    }

    // Convertir l'image en un tableau 1D
    let image_data: Vec<f32> = img_bgr.into_raw().iter().map(|&v| v as f32).collect();

    // Convertir le tableau 1D en un tableau 4D
    let input = Array4::from_shape_vec((1, 448, 448, 3), image_data).unwrap();
    let input_tensor = input.view();

    //let outputs: SessionOutputs = model.run(inputs!["images" => input.view()]?)?;
    let outputs: SessionOutputs = model.run(inputs!["input_1:0" => input_tensor]?)?;

    let binding = outputs["predictions_sigmoid"].extract_tensor::<f32>()?;
    let output = binding.view();
    let a = output.axis_iter(Axis(0)).next().ok_or(crate::Error::NotFound)?;

        let row: Vec<_> = a.iter().copied().enumerate().filter(|(i, p)| p > &(0.7 as f32)).map(|(index, proba)| {
            let element = tags.get(index);
            if let Some(element) = element {
                let tag = PredictionTagResult {
                    tag: element.clone(),
                    probability: proba,
                };
                Some(tag)
            } else {
                None
            }
        }).flatten().collect();
        Ok(row)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PredictionTag {
    pub index: usize,
    pub id: String,
    pub name: String,
    pub kind: PredictionTagKind
}
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PredictionTagResult {
    pub probability: f32,
    pub tag: PredictionTag,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")] 
pub enum PredictionTagKind {
	Category,
	Character,
	Tag,
}


impl  PredictionTag {
    pub fn from_csv(line: (usize, &str)) -> Self {
        let (index, tag) = line;
        let splitted = tag.split(",");
        let elements: Vec<&str> = splitted.collect();
        let kind_value = elements[2];
        let kind = if kind_value == "9" {
            PredictionTagKind::Category
        } else if kind_value == "4" {
            PredictionTagKind::Character
        } else {
            PredictionTagKind::Tag
        };
        let preduction = PredictionTag { index, id: format!("wd-v1-4-tags:{}", elements[0]), name: elements[1].replace("_", " "), kind};
        preduction
    }
}

fn mapping_wd(path: PathBuf) -> Result<Vec<PredictionTag>> {
    if path.exists() {
        let tags: Vec<PredictionTag> = read_to_string(path)?.lines().skip(1).enumerate().map(PredictionTag::from_csv).collect();
        Ok(tags)
    } else {
        Err(crate::Error::ModelMappingNotFound)
    }
    
}