use std::{collections::VecDeque, fs::read_to_string, io::Read, path::PathBuf, str::FromStr};

use ort::{inputs, GraphOptimizationLevel, Session, SessionOutputs, Tensor, ValueType};
use ndarray::{s, Array, Array4, Axis};
use image::{imageops::{self, FilterType}, DynamicImage, GenericImageView, ImageBuffer, Rgb, Rgba};
use serde::{Deserialize, Serialize};
use crate::{Error, Result};

pub enum PreductionModelType {
    WD,
    ImageNet,
}

/* 
pub fn predict(buffer_image: Vec<u8>) -> Result<Vec<PredictionTagResult>> {
    let target2 = PathBuf::from_str("models/wd-v1-4-tags.onnx").unwrap();
    let target = PathBuf::from_str("models/efficientnet-lite4-11-int8.onnx").expect("unable to set path");
    if target2.exists() {
        predict_net(target2, true, false, buffer_image)
    } else if target.exists() {
        predict_wd(target, buffer_image)
    } else {
        Err(crate::Error::NoModelFound)
    }
    
}*/

pub fn prepare_image(buffer_image: Vec<u8>, width: u32, height: u32) -> Result<DynamicImage> {
    let img = image::load_from_memory(&buffer_image)?;
    //let resized = img.resize_exact(width, height, image::imageops::FilterType::Nearest);
    let resized = resize_center_crop(&img, width, height);
    Ok(resized)
}

fn resize_center_crop(img: &DynamicImage, width: u32, height: u32) -> DynamicImage {
    // Calculez les dimensions de la zone de recadrage
    let (src_width, src_height) = img.dimensions();
    let min_dim = src_width.min(src_height);
    let crop_width = min_dim * width / height;
    let crop_height = min_dim * height / width;

    // Recadrez l'image au centre
    let cropped_img = imageops::crop_imm(
        img,
        (src_width - crop_width) / 2,
        (src_height - crop_height) / 2,
        crop_width,
        crop_height,
    ).to_image();
    let dynamic_cropped_img = DynamicImage::ImageRgba8(cropped_img);
    // Redimensionnez l'image à la taille souhaitée
    dynamic_cropped_img.resize_exact(width, height, FilterType::Lanczos3)
}

pub fn rgb_to_bgr(image: DynamicImage) -> ImageBuffer<Rgb<u8>, Vec<u8>> {
    let (width, height) = image.dimensions();
    let mut img_bgr = image::ImageBuffer::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let Rgba([r, g, b, _a]) = image.get_pixel(x, y);
            img_bgr.put_pixel(x, y, Rgb([b, g, r]));
        }
    }
    img_bgr
}

pub fn preload_model(path: &PathBuf) -> Result<Session> {
    Ok(Session::builder()?
            .with_optimization_level(GraphOptimizationLevel::Level3)?
            .with_intra_threads(4)?
            .commit_from_file(path)?)
}

pub fn predict_net(path: PathBuf, bgr: bool, normalize: bool, buffer_image: Vec<u8>, model: Option<&Session>) -> Result<Vec<PredictionTagResult>> {
    let mut mapping_path = path.clone();
    mapping_path.set_extension("csv");
    let tags = mapping(mapping_path)?;
    

    let loaded_session = if model.is_none() {
        Some(Session::builder()?
        .with_optimization_level(GraphOptimizationLevel::Level3)?
        .with_intra_threads(4)?
        .commit_from_file(path)?)
    } else {
        None
    };
    let ref_session = loaded_session.as_ref();

    let model = match model {
        Some(m) => m,
        None => {
            ref_session.unwrap()
        },
    };

    let output_info = model.outputs.first().ok_or(Error::Error("Prediction model does not have outputs".into()))?;

    let input_info = model.inputs.first().ok_or(Error::Error("Prediction model does not have outputs".into()))?;

       let size = match &input_info.input_type {

        ValueType::Tensor { ty: _, dimensions } => dimensions.get(1).map(|i| *i as u32),
        ValueType::Sequence(_) => None,
        ValueType::Map { key: _, value: _ } => None,
        ValueType::Optional(value_type) => None,
    };
    let size = size.ok_or(Error::Error("Unable to get dimensions".into()))?;
    let resized = prepare_image(buffer_image, size, size)?;
    let image_rgb = if bgr {
        rgb_to_bgr(resized)
    } else {
        resized.to_rgb8()
    };

    //let img_bgr = rgb_to_bgr(resized);

    // Convertir l'image en un tableau 1D
    let image_data: Vec<f32> = image_rgb.iter().map(|&v| {
                if normalize {
                    (v as f32 -127.0) / 128.0
                } else {
                    v as f32
                }
            
        }).collect();

    // Convertir le tableau 1D en un tableau 4D
    let input = Array4::from_shape_vec((1_usize, size as usize, size as usize, 3_usize), image_data)?;
    let input_tensor = input.view();

    //let outputs: SessionOutputs = model.run(inputs!["images" => input.view()]?)?;
    let outputs: SessionOutputs = model.run(inputs![input_info.name.to_string() => input_tensor]?)?;

    let binding = outputs[output_info.name.clone()].try_extract_tensor::<f32>()?;
    let output = binding.view();
    let a = output.axis_iter(Axis(0)).next().ok_or(crate::Error::NotFound)?;
    //println!("{:?}", a);
        let row: Vec<_> = a.iter().copied().enumerate().filter(|(_i, p)| p > &(0.3_f32)).filter_map(|(index, proba)| {
            //println!("{:?}", index);
            let element = tags.get(index);
            if let Some(element) = element {
                let tag = PredictionTagResult {
                    tag: element.clone(),
                    probability: proba * 100.0,
                };
                Some(tag)
            } else {
                None
            }
        }).collect();
        Ok(row)
}

pub fn predict_wd(path: PathBuf, buffer_image: Vec<u8>) -> Result<Vec<PredictionTagResult>> {
    let mut mapping_path = path.clone();
    mapping_path.set_extension("csv");
    let tags = mapping(mapping_path)?;
    
    let model = Session::builder()?
    .with_optimization_level(GraphOptimizationLevel::Level3)?
    .with_intra_threads(4)?
    .commit_from_file(path)?;

    let width = 448;
    let height = 448;
    let resized = prepare_image(buffer_image, width, height)?;

    let img_bgr = rgb_to_bgr(resized);

    // Convertir l'image en un tableau 1D
    let image_data: Vec<f32> = img_bgr.into_raw().iter().map(|&v| v as f32).collect();

    // Convertir le tableau 1D en un tableau 4D
    let input = Array4::from_shape_vec((1, 448, 448, 3), image_data).unwrap();
    let input_tensor = input.view();

    //let outputs: SessionOutputs = model.run(inputs!["images" => input.view()]?)?;
    let outputs: SessionOutputs = model.run(inputs!["input_1:0" => input_tensor]?)?;

    let binding = outputs["predictions_sigmoid"].try_extract_tensor::<f32>()?;
    let output = binding.view();
    let a = output.axis_iter(Axis(0)).next().ok_or(crate::Error::NotFound)?;

        let row: Vec<_> = a.iter().copied().enumerate().filter(|(_i, p)| p > &0.3_f32).filter_map(|(index, proba)| {
            let element: Option<&PredictionTag> = tags.get(index);
            if let Some(element) = element {
                let tag = PredictionTagResult {
                    tag: element.clone(),
                    probability: proba,
                };
                Some(tag)
            } else {
                None
            }
        }).collect();
        Ok(row)
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct PredictionTag {
    pub index: usize,
    pub id: String,
    pub name: String,
    pub alts: Vec<String>,
    pub kind: PredictionTagKind
}

impl PredictionTag {
    pub fn all_names(&self) -> Vec<String> {
        let mut all_names = vec![self.name.clone()];
        let mut alts = self.alts.clone();
        all_names.append(&mut alts);
        all_names
    }
}


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PredictionTagResult {
    pub probability: f32,
    pub tag: PredictionTag,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub enum PredictionTagKind {
	Category,
	Character,
    #[default]
	Tag,
}


impl  PredictionTag {
    pub fn from_csv(line: (usize, &str)) -> Self {
        let (index, tag) = line;
        let splitted = tag.split(',');
        let elements: Vec<&str> = splitted.collect();
        let kind_value = elements.get(2).unwrap_or(&"0").to_string();
        let kind = if kind_value == "9" {
            PredictionTagKind::Category
        } else if kind_value == "4" {
            PredictionTagKind::Character
        } else {
            PredictionTagKind::Tag
        };
        let name = elements[1].replace('_', " ").replace('\"', "");
        let mut names: VecDeque<String> = VecDeque::from_iter(name.split('|').map(|t| t.trim().to_string()));
        let name = names.pop_front().unwrap_or(name);
        let preduction = PredictionTag { index, id: format!("wd-v1-4-tags:{}", elements[0]), name, kind, alts: names.make_contiguous().to_vec()};
        preduction
    }
}

fn mapping(path: PathBuf) -> Result<Vec<PredictionTag>> {
    if path.exists() {
        let tags: Vec<PredictionTag> = read_to_string(path)?.lines().enumerate().map(PredictionTag::from_csv).collect();
        Ok(tags)
    } else {
        Err(crate::Error::ModelMappingNotFound)
    } 
}