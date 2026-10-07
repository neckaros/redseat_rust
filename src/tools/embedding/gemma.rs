//! EmbeddingGemma 2 (`onnx-community/embeddinggemma-2-ONNX`), q4 weights, on CPU.
//!
//! An image goes through the vision encoder, whose features are then spliced into
//! the text backbone in place of image tokens. A text query only needs the backbone.

use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

use futures::StreamExt;
use image::imageops::FilterType;
use ort::{
    memory::Allocator,
    session::{builder::GraphOptimizationLevel, Session},
    value::Tensor,
};
use sha2::{Digest, Sha256};
use tokenizers::Tokenizer;
use tokio::io::AsyncWriteExt;

use super::{l2_normalize, Embedder, EmbedderSpec, EmbeddingError, EmbeddingResult};

/// Vision tokens per image. Part of the embedder id: another budget is another space.
pub const TOKEN_BUDGET: u32 = 140;

pub const SPEC: EmbedderSpec = EmbedderSpec {
    id: "embeddinggemma-2/q4/t140",
    dimension: 768,
    search_dimension: 512,
    // 127 / p, where p = 0.3108 is the 99.9th percentile of the absolute component
    // values over a sample of real image and text embeddings (see docs/EMBEDDINGS.md).
    // Changing it requires a new id.
    quantization_scale: 408.6,
};

/// Sub-directory of the models directory holding the files below.
pub const MODEL_DIRECTORY: &str = "embeddinggemma-2-q4";

/// Pinned to one commit of the repository: the embedder id promises that every stored
/// vector comes from the same weights and tokenizer. Another revision needs a new id.
const BASE_URL: &str = "https://huggingface.co/onnx-community/embeddinggemma-2-ONNX/resolve/daa72c51243991dfcaf9f9137d2c573d8f7790c0/";
const BACKBONE_FILE: &str = "model_q4.onnx";
const VISION_FILE: &str = "vision_encoder_q4.onnx";
const TOKENIZER_FILE: &str = "tokenizer.json";
/// (local name, path in the repository, SHA-256). Each `.onnx_data` must sit beside its `.onnx`.
const FILES: &[(&str, &str, &str)] = &[
    (
        "model_q4.onnx_data",
        "onnx/model_q4.onnx_data",
        "c3975f2d1ab7a1878ae31a7d7a9b7804a827aff3800b60dfceafce21cac3df49",
    ),
    (
        BACKBONE_FILE,
        "onnx/model_q4.onnx",
        "f9eeba97acddf139b8ee2ddf04bc30dceafa88de93fadf74d7644e0d61a477a9",
    ),
    (
        "vision_encoder_q4.onnx_data",
        "onnx/vision_encoder_q4.onnx_data",
        "0a9d6c927334f152a33dd90874f65d6ea5228999abe6a450d3f7813677fa704c",
    ),
    (
        VISION_FILE,
        "onnx/vision_encoder_q4.onnx",
        "7ea284226d4938f0ad921ab091f1d80a9ca699aa802984ef5cd5eec4f4761d96",
    ),
    (
        TOKENIZER_FILE,
        "tokenizer.json",
        "4d777ef5bdc1aa36227abdfb77c3e49e7b9c892d16e1b6bda41c393504828be4",
    ),
];

const QUERY_PREFIX: &str = "task: search result | query: ";

const PATCH_SIZE: u32 = 16;
const POOLING_KERNEL: u32 = 3;
const FEATURE_SIZE: usize = 512;

const TOKEN_BOS: i64 = 2;
const TOKEN_EOS: i64 = 1;
const TOKEN_BEGIN_IMAGE: i64 = 255999;
const TOKEN_IMAGE: i64 = 258880;
const TOKEN_END_IMAGE: i64 = 258882;

pub fn default_threads() -> usize {
    std::thread::available_parallelism()
        .map(|cores| cores.get())
        .unwrap_or(1)
        .min(4)
}

pub fn files_present(directory: &Path) -> bool {
    FILES
        .iter()
        .all(|(name, _, _)| directory.join(name).exists())
}

/// Downloads the missing model files into `directory`, checking each against its checksum.
pub async fn ensure_files(
    directory: &Path,
    on_progress: &(dyn Fn(String) + Send + Sync),
) -> EmbeddingResult<()> {
    tokio::fs::create_dir_all(directory).await?;
    for (name, remote, sha256) in FILES {
        let destination = directory.join(name);
        if destination.exists() {
            continue;
        }
        on_progress(format!("Downloading embedding model file: {name}"));
        // Download to .tmp first so an interrupted transfer never looks complete.
        let temporary = directory.join(format!("{name}.tmp"));
        let url = format!("{BASE_URL}{remote}");
        match download_file(&url, &temporary, sha256, on_progress).await {
            Ok(()) => tokio::fs::rename(&temporary, &destination).await?,
            Err(error) => {
                let _ = tokio::fs::remove_file(&temporary).await;
                return Err(error);
            }
        }
    }
    Ok(())
}

async fn download_file(
    url: &str,
    destination: &Path,
    sha256: &str,
    on_progress: &(dyn Fn(String) + Send + Sync),
) -> EmbeddingResult<()> {
    let download_error =
        |error: reqwest::Error| EmbeddingError::Download(format!("{url}: {error}"));
    let response = reqwest::get(url).await.map_err(download_error)?;
    if !response.status().is_success() {
        return Err(EmbeddingError::Download(format!(
            "{url}: HTTP {}",
            response.status()
        )));
    }

    let total_size = response.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(destination).await?;
    let mut stream = response.bytes_stream();
    let mut downloaded: u64 = 0;
    let mut last_percent: u64 = 0;
    let mut hasher = Sha256::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(download_error)?;
        file.write_all(&chunk).await?;
        hasher.update(&chunk);
        downloaded += chunk.len() as u64;
        if total_size > 0 {
            let percent = downloaded * 100 / total_size;
            if percent >= last_percent + 10 {
                on_progress(format!("Downloading {}: {percent}%", destination.display()));
                last_percent = percent;
            }
        }
    }
    file.flush().await?;
    if total_size > 0 && downloaded != total_size {
        return Err(EmbeddingError::Download(format!(
            "{url}: received {downloaded} of {total_size} bytes"
        )));
    }
    let received = format!("{:x}", hasher.finalize());
    if received != sha256 {
        return Err(EmbeddingError::Download(format!(
            "{url}: checksum {received} does not match the expected {sha256}"
        )));
    }
    Ok(())
}

/// Resize target `(width, height)`: keeps the aspect ratio, both sides multiples of
/// `PATCH_SIZE * POOLING_KERNEL`, and about `budget` tokens after pooling.
pub fn target_size(width: u32, height: u32, budget: u32) -> (u32, u32) {
    let multiple = PATCH_SIZE * POOLING_KERNEL;
    let pixels = f64::from(budget) * f64::from(multiple * multiple);
    let factor = (pixels / (f64::from(width) * f64::from(height))).sqrt();
    let fit = |side: u32| {
        let scaled = (f64::from(side) * factor / f64::from(multiple)).floor() as u32;
        (scaled * multiple).max(multiple)
    };
    (fit(width), fit(height))
}

pub struct ImagePatches {
    /// `count * PATCH_SIZE * PATCH_SIZE * 3` values in `0..=1`, patches in row-major order.
    pub pixels: Vec<f32>,
    /// `[x, y]` (column, then row) of each patch, same order.
    pub positions: Vec<i64>,
    pub count: usize,
}

impl ImagePatches {
    /// Rows the vision encoder returns for these patches.
    pub fn tokens(&self) -> usize {
        self.count / (POOLING_KERNEL * POOLING_KERNEL) as usize
    }
}

pub fn image_patches(image: &image::DynamicImage, budget: u32) -> ImagePatches {
    let (width, height) = target_size(image.width().max(1), image.height().max(1), budget);
    // CatmullRom is the bicubic kernel the reference pipeline uses. No mean/std step.
    let resized = image
        .resize_exact(width, height, FilterType::CatmullRom)
        .to_rgb8();
    let patch = PATCH_SIZE as usize;
    let (columns, rows) = (width as usize / patch, height as usize / patch);
    let stride = width as usize * 3;
    let raw = resized.as_raw();

    let count = rows * columns;
    let mut pixels = Vec::with_capacity(count * patch * patch * 3);
    let mut positions = Vec::with_capacity(count * 2);
    for row in 0..rows {
        for column in 0..columns {
            for y in 0..patch {
                let start = (row * patch + y) * stride + column * patch * 3;
                pixels.extend(
                    raw[start..start + patch * 3]
                        .iter()
                        .map(|value| f32::from(*value) / 255.0),
                );
            }
            positions.push(column as i64);
            positions.push(row as i64);
        }
    }
    ImagePatches {
        pixels,
        positions,
        count,
    }
}

/// BOS, begin-of-image, one image token per feature row, end-of-image, EOS.
pub fn image_token_ids(tokens: usize) -> Vec<i64> {
    let mut ids = Vec::with_capacity(tokens + 4);
    ids.extend([TOKEN_BOS, TOKEN_BEGIN_IMAGE]);
    ids.extend(std::iter::repeat(TOKEN_IMAGE).take(tokens));
    ids.extend([TOKEN_END_IMAGE, TOKEN_EOS]);
    ids
}

pub struct EmbeddingGemma {
    directory: PathBuf,
    threads: usize,
    tokenizer: Tokenizer,
    backbone: Mutex<Session>,
    /// About 2.7 s per image and several hundred MB: loaded on first image,
    /// dropped by `release_image_resources` once a batch is over.
    vision: Mutex<Option<Session>>,
}

fn load_session(path: PathBuf, threads: usize) -> EmbeddingResult<Session> {
    Ok(Session::builder()?
        .with_optimization_level(GraphOptimizationLevel::Level3)?
        .with_intra_threads(threads)?
        .commit_from_file(path)?)
}

/// `[rows, FEATURE_SIZE]` tensor. `Tensor::from_array` refuses a zero-length
/// dimension, so an empty one is allocated by the runtime instead.
fn features_tensor(features: Vec<f32>) -> EmbeddingResult<Tensor<f32>> {
    let rows = features.len() / FEATURE_SIZE;
    Ok(if rows == 0 {
        Tensor::new(&Allocator::default(), [0usize, FEATURE_SIZE])?
    } else {
        Tensor::from_array(([rows, FEATURE_SIZE], features))?
    })
}

fn poisoned<T>(_: T) -> EmbeddingError {
    EmbeddingError::Model("embedding session lock poisoned".to_string())
}

impl EmbeddingGemma {
    /// Loads the tokenizer and the backbone. Blocking.
    pub fn load(directory: &Path, threads: usize) -> EmbeddingResult<Self> {
        let tokenizer = Tokenizer::from_file(directory.join(TOKENIZER_FILE))
            .map_err(|error| EmbeddingError::Tokenizer(error.to_string()))?;
        let backbone = load_session(directory.join(BACKBONE_FILE), threads)?;
        Ok(Self {
            directory: directory.to_path_buf(),
            threads,
            tokenizer,
            backbone: Mutex::new(backbone),
            vision: Mutex::new(None),
        })
    }

    /// The vision encoder, loaded if it is not in memory.
    fn vision_session<'a>(
        &self,
        vision: &'a mut Option<Session>,
    ) -> EmbeddingResult<&'a mut Session> {
        Ok(match vision {
            Some(session) => session,
            empty => empty.insert(load_session(
                self.directory.join(VISION_FILE),
                self.threads,
            )?),
        })
    }

    fn image_features(&self, patches: ImagePatches) -> EmbeddingResult<Vec<f32>> {
        let patch_values = (PATCH_SIZE * PATCH_SIZE * 3) as usize;
        let pixel_values = Tensor::from_array(([1, patches.count, patch_values], patches.pixels))?;
        let position_ids = Tensor::from_array(([1, patches.count, 2], patches.positions))?;

        let mut vision = self.vision.lock().map_err(poisoned)?;
        let session = self.vision_session(&mut vision)?;
        let outputs = session.run(ort::inputs![
            "pixel_values" => pixel_values,
            "pixel_position_ids" => position_ids,
        ])?;
        let (_, features) = outputs["image_features"].try_extract_tensor::<f32>()?;
        Ok(features.to_vec())
    }

    fn sentence_embedding(
        &self,
        input_ids: Vec<i64>,
        image_features: Vec<f32>,
    ) -> EmbeddingResult<Vec<f32>> {
        let length = input_ids.len();
        let attention_mask = Tensor::from_array(([1, length], vec![1i64; length]))?;
        let input_ids = Tensor::from_array(([1, length], input_ids))?;
        let image_features = features_tensor(image_features)?;
        // The graph requires the video and audio inputs even when there is nothing in them.
        let video_features = features_tensor(Vec::new())?;
        let audio_features = features_tensor(Vec::new())?;

        let mut session = self.backbone.lock().map_err(poisoned)?;
        let outputs = session.run(ort::inputs![
            "input_ids" => input_ids,
            "attention_mask" => attention_mask,
            "image_features" => image_features,
            "video_features" => video_features,
            "audio_features" => audio_features,
        ])?;
        let (_, embedding) = outputs["sentence_embedding"].try_extract_tensor::<f32>()?;
        if embedding.len() != SPEC.dimension {
            return Err(EmbeddingError::Model(format!(
                "unexpected embedding length {}",
                embedding.len()
            )));
        }
        let mut embedding = embedding.to_vec();
        l2_normalize(&mut embedding);
        if embedding.iter().any(|value| !value.is_finite()) {
            return Err(EmbeddingError::Model(
                "embedding contains non-finite values".to_string(),
            ));
        }
        Ok(embedding)
    }
}

impl Embedder for EmbeddingGemma {
    fn spec(&self) -> &EmbedderSpec {
        &SPEC
    }

    fn embed_image(&self, bytes: &[u8]) -> EmbeddingResult<Vec<f32>> {
        let image = image::load_from_memory(bytes)?;
        let features = self.image_features(image_patches(&image, TOKEN_BUDGET))?;
        let tokens = features.len() / FEATURE_SIZE;
        self.sentence_embedding(image_token_ids(tokens), features)
    }

    fn embed_text(&self, query: &str) -> EmbeddingResult<Vec<f32>> {
        // The tokenizer adds <bos> and <eos> itself.
        let encoding = self
            .tokenizer
            .encode(format!("{QUERY_PREFIX}{query}"), true)
            .map_err(|error| EmbeddingError::Tokenizer(error.to_string()))?;
        let input_ids = encoding.get_ids().iter().map(|id| i64::from(*id)).collect();
        self.sentence_embedding(input_ids, Vec::new())
    }

    fn prepare_image_resources(&self) -> EmbeddingResult<()> {
        let mut vision = self.vision.lock().map_err(poisoned)?;
        self.vision_session(&mut vision)?;
        Ok(())
    }

    fn release_image_resources(&self) {
        if let Ok(mut vision) = self.vision.lock() {
            *vision = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{index::VectorIndex, quantize};
    use super::*;

    #[test]
    fn target_sizes_match_reference() {
        assert_eq!(target_size(640, 480, 280), (912, 672));
        assert_eq!(target_size(640, 480, 140), (624, 480));
        assert_eq!(target_size(640, 480, 70), (432, 336));
        // Extreme ratios never collapse a side below one pooled token.
        assert_eq!(target_size(4000, 10, 140).1, 48);
    }

    #[test]
    fn patch_and_token_counts_match_reference() {
        let image = image::DynamicImage::new_rgb8(640, 480);
        for (budget, patches, tokens) in [(280, 2394, 266), (140, 1170, 130), (70, 567, 63)] {
            let result = image_patches(&image, budget);
            assert_eq!(result.count, patches);
            assert_eq!(result.tokens(), tokens);
            assert_eq!(result.pixels.len(), patches * 768);
            assert_eq!(result.positions.len(), patches * 2);
        }
    }

    #[test]
    fn patches_are_row_major_with_x_before_y() {
        // 96x48 already fits a budget of 2: 6 columns by 3 rows of 16px patches.
        let mut image = image::RgbImage::new(96, 48);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            *pixel = image::Rgb([(x / 16) as u8 * 40, (y / 16) as u8 * 100, 255]);
        }
        let result = image_patches(&image::DynamicImage::ImageRgb8(image), 2);
        assert_eq!(result.count, 18);
        assert_eq!(&result.positions[..6], &[0, 0, 1, 0, 2, 0]);
        // Patch 7 is column 1 of row 1.
        assert_eq!(&result.positions[14..16], &[1, 1]);
        let patch = &result.pixels[7 * 768..8 * 768];
        assert!((patch[0] - 40.0 / 255.0).abs() < 1e-6);
        assert!((patch[1] - 100.0 / 255.0).abs() < 1e-6);
        assert!((patch[2] - 1.0).abs() < 1e-6);
        assert!((patch[765] - 40.0 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn image_token_sequence_is_wrapped() {
        assert_eq!(
            image_token_ids(2),
            vec![2, 255999, 258880, 258880, 258882, 1]
        );
    }

    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b).map(|(a, b)| a * b).sum()
    }

    fn test_model_directory() -> PathBuf {
        std::env::var("REDSEAT_EMBEDDING_TEST_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("redseat-embedding-test"))
    }

    fn percentile(values: &mut [f32], percent: f64) -> f32 {
        values.sort_by(f32::total_cmp);
        values[((values.len() - 1) as f64 * percent / 100.0).round() as usize]
    }

    fn integer_cosine(a: &[i8], b: &[i8]) -> f32 {
        let dot = |a: &[i8], b: &[i8]| -> f32 {
            a.iter()
                .zip(b)
                .map(|(a, b)| i32::from(*a) * i32::from(*b))
                .sum::<i32>() as f32
        };
        dot(a, b) / (dot(a, a).sqrt() * dot(b, b).sqrt())
    }

    const CALIBRATION_QUERIES: &[&str] = &[
        "cats sleeping on a couch",
        "a dog running on the beach",
        "kids at the beach",
        "a plate of food on a table",
        "people riding bicycles in a city street",
        "a man wearing a suit and tie",
        "a living room with a television",
        "snow covered mountains",
        "a bathroom with a sink and a mirror",
        "a train at a station",
        "an airplane in the sky",
        "a baseball player swinging a bat",
        "a kitchen with wooden cabinets",
        "a group of people around a dinner table",
        "a bear in the forest",
        "a woman holding an umbrella",
        "a stop sign at an intersection",
        "sunset over the ocean",
        "a birthday party",
        "a person skiing down a slope",
        "a bowl of fruit",
        "a bus on a busy road",
        "flowers in a vase",
        "a laptop on a desk",
        "elephants walking in the grass",
        "a child playing with a toy",
        "a wedding",
        "a president's speech about serving your country",
        "a turtle swimming in the ocean",
        "night city skyline",
        "a tennis match",
        "a horse in a field",
        "a bedroom",
        "a pizza",
        "people at a concert",
        "a boat on a lake",
        "a street market",
        "a giraffe",
        "someone surfing a wave",
        "a clock tower",
    ];

    /// How `SPEC.quantization_scale` was derived, and the check that searching the
    /// first 512 values of the stored int8 vector equals quantising a re-normalised
    /// 512 prefix. Embeds every image of `REDSEAT_EMBEDDING_CALIBRATION_DIR`.
    #[tokio::test]
    #[ignore = "downloads the EmbeddingGemma 2 model and needs a directory of images"]
    async fn embeddinggemma_calibration() {
        let directory = test_model_directory();
        ensure_files(&directory, &|message| println!("{message}"))
            .await
            .unwrap();
        let images_directory = std::env::var("REDSEAT_EMBEDDING_CALIBRATION_DIR")
            .expect("REDSEAT_EMBEDDING_CALIBRATION_DIR must point to a directory of images");
        let mut paths: Vec<PathBuf> = std::fs::read_dir(images_directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        paths.sort();

        let embedder = EmbeddingGemma::load(&directory, default_threads()).unwrap();
        let images: Vec<Vec<f32>> = paths
            .iter()
            .map(|path| embedder.embed_image(&std::fs::read(path).unwrap()).unwrap())
            .collect();
        let texts: Vec<Vec<f32>> = CALIBRATION_QUERIES
            .iter()
            .map(|query| embedder.embed_text(query).unwrap())
            .collect();

        let absolute = |vectors: &[&Vec<f32>], length: usize| -> Vec<f32> {
            vectors
                .iter()
                .flat_map(|vector| vector[..length].iter().map(|value| value.abs()))
                .collect()
        };
        let all: Vec<&Vec<f32>> = images.iter().chain(&texts).collect();
        let p = percentile(&mut absolute(&all, SPEC.dimension), 99.9);
        let image_p = percentile(
            &mut absolute(&images.iter().collect::<Vec<_>>(), SPEC.dimension),
            99.9,
        );
        let text_p = percentile(
            &mut absolute(&texts.iter().collect::<Vec<_>>(), SPEC.dimension),
            99.9,
        );
        let mut everything = absolute(&all, SPEC.dimension);
        println!(
            "|v| percentiles: p50 {:.5}, p99 {:.5}, p99.9 {:.5}, p99.99 {:.5}, max {:.5}",
            percentile(&mut everything, 50.0),
            percentile(&mut everything, 99.0),
            percentile(&mut everything, 99.9),
            percentile(&mut everything, 99.99),
            percentile(&mut everything, 100.0)
        );
        // Position of the largest components, to spot dimensions that dominate every vector.
        let mut mean_by_position = vec![0f32; SPEC.dimension];
        for vector in &all {
            for (mean, value) in mean_by_position.iter_mut().zip(vector.iter()) {
                *mean += value / all.len() as f32;
            }
        }
        let mut positions: Vec<usize> = (0..SPEC.dimension).collect();
        positions.sort_by(|a, b| {
            mean_by_position[*b]
                .abs()
                .total_cmp(&mean_by_position[*a].abs())
        });
        println!(
            "largest mean components: {:?}",
            positions[..6]
                .iter()
                .map(|position| (*position, mean_by_position[*position]))
                .collect::<Vec<_>>()
        );
        println!(
            "{} images, {} texts: p99.9 |v| = {p:.5} (images {image_p:.5}, texts {text_p:.5}) -> scale 127/p = {:.1}",
            images.len(),
            texts.len(),
            127.0 / p
        );

        // A: truncate to the search dimension, re-normalise, then quantise.
        let prefix = |vector: &Vec<f32>| {
            let mut prefix = vector[..SPEC.search_dimension].to_vec();
            l2_normalize(&mut prefix);
            prefix
        };
        let prefixes: Vec<Vec<f32>> = all.iter().map(|vector| prefix(vector)).collect();
        let prefix_p = percentile(
            &mut absolute(&prefixes.iter().collect::<Vec<_>>(), SPEC.search_dimension),
            99.9,
        );
        let truncate_first = |vector: &Vec<f32>| quantize(&prefix(vector), 127.0 / prefix_p);
        // B (stored design): quantise everything, search the leading values.
        let quantize_first = |vector: &Vec<f32>| {
            quantize(vector, SPEC.quantization_scale)[..SPEC.search_dimension].to_vec()
        };

        let (mut max_difference, mut sum_difference, mut pairs) = (0f32, 0f32, 0);
        let (mut max_float_difference, mut same_best, mut top5_overlap) = (0f32, 0, 0);
        let mut clipped = 0;
        for vector in &all {
            clipped += quantize(vector, SPEC.quantization_scale)
                .iter()
                .filter(|value| value.unsigned_abs() == 127)
                .count();
        }
        for text in &texts {
            let (a_text, b_text) = (truncate_first(text), quantize_first(text));
            let float_text = prefix(text);
            let mut a_scores = Vec::new();
            let mut b_scores = Vec::new();
            for (position, image) in images.iter().enumerate() {
                let a = integer_cosine(&a_text, &truncate_first(image));
                let b = integer_cosine(&b_text, &quantize_first(image));
                let float = cosine(&float_text, &prefix(image));
                max_difference = max_difference.max((a - b).abs());
                max_float_difference = max_float_difference.max((float - b).abs());
                sum_difference += (a - b).abs();
                pairs += 1;
                a_scores.push((position, a));
                b_scores.push((position, b));
            }
            a_scores.sort_by(|x, y| y.1.total_cmp(&x.1));
            b_scores.sort_by(|x, y| y.1.total_cmp(&x.1));
            if a_scores[0].0 == b_scores[0].0 {
                same_best += 1;
            }
            let top = |scores: &[(usize, f32)]| -> Vec<usize> {
                scores
                    .iter()
                    .take(5)
                    .map(|(position, _)| *position)
                    .collect()
            };
            let a_top = top(&a_scores);
            top5_overlap += top(&b_scores)
                .iter()
                .filter(|position| a_top.contains(position))
                .count();
        }
        println!(
            "truncate-then-quantise vs quantise-then-truncate over {pairs} pairs: max |cos diff| {max_difference:.5}, mean {:.5}; same best image for {same_best}/{} queries; top-5 overlap {top5_overlap}/{}",
            sum_difference / pairs as f32,
            texts.len(),
            texts.len() * 5
        );
        let (mut max_full_difference, mut sum_full_difference) = (0f32, 0f32);
        for text in &texts {
            for image in &images {
                let difference =
                    (cosine(text, image) - cosine(&prefix(text), &prefix(image))).abs();
                max_full_difference = max_full_difference.max(difference);
                sum_full_difference += difference;
            }
        }
        println!(
            "float cosine, 768 values vs 512 prefix: max |diff| {max_full_difference:.5}, mean {:.5}",
            sum_full_difference / pairs as f32
        );
        println!(
            "stored design vs float cosine on the 512 prefix: max |cos diff| {max_float_difference:.5}; values clipped at +-127: {clipped} of {}",
            all.len() * SPEC.dimension
        );
        assert!(max_difference < 0.02);
    }

    /// Downloads about 316 MB into `REDSEAT_EMBEDDING_TEST_DIR` (or the temp dir) and
    /// embeds the reference image against the three reference queries.
    /// Run with `cargo test --bin redseat-rust embeddinggemma_reference -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "downloads the EmbeddingGemma 2 model"]
    async fn embeddinggemma_reference() {
        let directory = test_model_directory();
        ensure_files(&directory, &|message| println!("{message}"))
            .await
            .unwrap();
        let image = reqwest::get(
            "https://huggingface.co/datasets/Xenova/transformers.js-docs/resolve/main/cats.jpg",
        )
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();

        let embedder = EmbeddingGemma::load(&directory, default_threads()).unwrap();
        let started = std::time::Instant::now();
        let image = embedder.embed_image(&image).unwrap();
        println!("image embedded in {:?}", started.elapsed());
        assert_eq!(image.len(), SPEC.dimension);
        assert!((cosine(&image, &image) - 1.0).abs() < 1e-3);

        let queries = [
            "cats sleeping on a couch",
            "a president's speech about serving your country",
            "a turtle swimming in the ocean",
        ];
        let mut index = VectorIndex::new(SPEC.search_dimension);
        index.upsert("cats", &quantize(&image, SPEC.quantization_scale));
        let mut scores = Vec::new();
        for query in queries {
            let started = std::time::Instant::now();
            let text = embedder.embed_text(query).unwrap();
            let elapsed = started.elapsed();
            let float = cosine(&image, &text);
            let prefix = |vector: &[f32]| {
                let mut prefix = vector[..SPEC.search_dimension].to_vec();
                l2_normalize(&mut prefix);
                prefix
            };
            let float_prefix = cosine(&prefix(&image), &prefix(&text));
            // Same comparison as production search: int8, leading values only.
            let stored = index.rank(&quantize(&text, SPEC.quantization_scale), 1)[0].1;
            println!(
                "{query}: float 768 {float:.4}, float 512 {float_prefix:.4}, int8 512 {stored:.4} ({elapsed:?})"
            );
            assert!((float_prefix - stored).abs() < 0.04);
            scores.push((float, stored));
        }

        // Model card (q4, budget 280): 0.740, 0.462, 0.508.
        assert!(scores[0].0 > 0.65, "matching query scored {}", scores[0].0);
        for (float, stored) in &scores[1..] {
            assert!(scores[0].0 > float + 0.1);
            assert!(scores[0].1 > stored + 0.1);
        }

        embedder.release_image_resources();
        assert!(embedder.vision.lock().unwrap().is_none());
    }
}
