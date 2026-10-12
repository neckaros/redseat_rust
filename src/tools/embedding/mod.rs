//! Media embeddings: turns an image or a text query into a vector in a shared space.
//!
//! This module is deliberately self-contained: it must not import `crate::model`,
//! the routes or the store, so another binary can re-declare it (there is no `lib.rs`).
//! Everything downstream only sees vectors and an [`EmbedderSpec`], never model specifics.
//! See `docs/EMBEDDINGS.md`.

use std::{fmt, path::Path, sync::Arc};

pub mod gemma;
pub mod index;

pub type EmbeddingResult<T> = Result<T, EmbeddingError>;

#[derive(Debug)]
pub enum EmbeddingError {
    Io(std::io::Error),
    /// Model files could not be fetched.
    Download(String),
    /// The runtime failed to load or run a graph.
    Model(String),
    Tokenizer(String),
    Image(String),
    InvalidInput(String),
}

impl fmt::Display for EmbeddingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "embedding io error: {error}"),
            Self::Download(message) => write!(f, "embedding model download failed: {message}"),
            Self::Model(message) => write!(f, "embedding model error: {message}"),
            Self::Tokenizer(message) => write!(f, "embedding tokenizer error: {message}"),
            Self::Image(message) => write!(f, "embedding image error: {message}"),
            Self::InvalidInput(message) => write!(f, "embedding invalid input: {message}"),
        }
    }
}

impl std::error::Error for EmbeddingError {}

impl From<std::io::Error> for EmbeddingError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl<R> From<ort::Error<R>> for EmbeddingError {
    fn from(error: ort::Error<R>) -> Self {
        Self::Model(error.to_string())
    }
}

impl From<image::ImageError> for EmbeddingError {
    fn from(error: image::ImageError) -> Self {
        Self::Image(error.to_string())
    }
}

/// Everything storage and search need to know about an embedder.
///
/// Vectors produced under different ids are never comparable, so the id must change
/// whenever the model, its weight precision, its input budget or `quantization_scale`
/// changes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EmbedderSpec {
    pub id: &'static str,
    /// Length of the vectors returned by the embedder, and of the stored int8 vectors.
    pub dimension: usize,
    /// Number of leading values used for search. Equal to `dimension` for models
    /// that do not support truncation.
    pub search_dimension: usize,
    /// Fixed factor applied before rounding to int8, see [`quantize`].
    pub quantization_scale: f32,
    /// Search score under which an image is not a match for a text. Only a search
    /// setting: changing it does not need a new id.
    pub text_cutoff: f32,
}

/// Blocking interface: callers run it off the async executor.
pub trait Embedder: Send + Sync {
    fn spec(&self) -> &EmbedderSpec;

    fn id(&self) -> &str {
        self.spec().id
    }

    fn dimension(&self) -> usize {
        self.spec().dimension
    }

    fn search_dimension(&self) -> usize {
        self.spec().search_dimension
    }

    /// `bytes` is an encoded image (PNG, JPEG, WebP...). Returns an L2-normalised vector.
    fn embed_image(&self, bytes: &[u8]) -> EmbeddingResult<Vec<f32>>;

    /// Returns an L2-normalised vector in the same space as [`Embedder::embed_image`].
    fn embed_text(&self, query: &str) -> EmbeddingResult<Vec<f32>>;

    /// Loads what is only needed for images, so a model that cannot load fails here
    /// and not on the first image.
    fn prepare_image_resources(&self) -> EmbeddingResult<()> {
        Ok(())
    }

    /// Frees what is only needed for images. Text embedding must stay available.
    fn release_image_resources(&self) {}
}

/// The embedder new vectors are computed with. Switching model means changing these
/// three functions; stored vectors of the previous id are simply ignored.
pub fn active_spec() -> EmbedderSpec {
    gemma::SPEC
}

/// True when the active embedder can be loaded without downloading anything.
pub fn active_files_present(models_dir: &Path) -> bool {
    gemma::files_present(&models_dir.join(gemma::MODEL_DIRECTORY))
}

/// Downloads the active embedder's files if needed, then loads it.
pub async fn load_active(
    models_dir: &Path,
    on_progress: &(dyn Fn(String) + Send + Sync),
) -> EmbeddingResult<Arc<dyn Embedder>> {
    let directory = models_dir.join(gemma::MODEL_DIRECTORY);
    gemma::ensure_files(&directory, on_progress).await?;
    let embedder = tokio::task::spawn_blocking(move || {
        gemma::EmbeddingGemma::load(&directory, gemma::default_threads())
    })
    .await
    .map_err(|error| EmbeddingError::Model(format!("Join error: {error}")))??;
    Ok(Arc::new(embedder))
}

pub fn l2_normalize(vector: &mut [f32]) {
    let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in vector.iter_mut() {
            *value /= norm;
        }
    }
}

/// Mean of normalised vectors, normalised again. Used to fold video frames into one vector.
pub fn average_normalized(vectors: &[Vec<f32>]) -> Option<Vec<f32>> {
    let first = vectors.first()?;
    let mut average = vec![0f32; first.len()];
    for vector in vectors {
        if vector.len() != average.len() {
            return None;
        }
        for (sum, value) in average.iter_mut().zip(vector) {
            *sum += value;
        }
    }
    l2_normalize(&mut average);
    Some(average)
}

/// `clamp(round(v * scale), -127, 127)`.
pub fn quantize(vector: &[f32], scale: f32) -> Vec<i8> {
    vector
        .iter()
        .map(|value| (value * scale).round().clamp(-127.0, 127.0) as i8)
        .collect()
}

/// Stored blob layout: one byte per value, two's complement.
pub fn to_bytes(vector: &[i8]) -> Vec<u8> {
    vector.iter().map(|value| *value as u8).collect()
}

pub fn from_bytes(bytes: &[u8]) -> Vec<i8> {
    bytes.iter().map(|value| *value as i8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantize_rounds_and_clamps() {
        let scale = 100.0;
        let quantized = quantize(&[0.0, 0.004, 0.005, -0.005, 0.5, 2.0, -2.0, -1.27], scale);
        assert_eq!(quantized, vec![0, 0, 1, -1, 50, 127, -127, -127]);
    }

    #[test]
    fn quantized_bytes_roundtrip() {
        let vector: Vec<i8> = vec![-127, -1, 0, 1, 127];
        let bytes = to_bytes(&vector);
        assert_eq!(bytes, vec![129, 255, 0, 1, 127]);
        assert_eq!(from_bytes(&bytes), vector);
    }

    #[test]
    fn average_is_renormalized() {
        let average = average_normalized(&[vec![1.0, 0.0], vec![0.0, 1.0]]).unwrap();
        let expected = std::f32::consts::FRAC_1_SQRT_2;
        assert!((average[0] - expected).abs() < 1e-6);
        assert!((average[1] - expected).abs() < 1e-6);
        assert!(average_normalized(&[]).is_none());
        assert!(average_normalized(&[vec![1.0], vec![1.0, 0.0]]).is_none());
    }

    #[test]
    fn active_spec_searches_within_its_dimension() {
        let spec = active_spec();
        assert!(spec.search_dimension > 0 && spec.search_dimension <= spec.dimension);
        assert!(spec.quantization_scale > 0.0);
    }
}
