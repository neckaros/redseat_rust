use serde::{Deserialize, Serialize};

use crate::tools::image_tools::{ImageSize, ImageType};

pub mod ping;
pub mod libraries;
pub mod users;
pub mod mw_auth;
pub mod socket;
pub mod credentials;
pub mod backups;

pub mod tags;
pub mod people;
pub mod series;
pub mod episodes;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ImageRequestOptions {
    size: Option<ImageSize>,
    kind: Option<ImageType>
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ImageUploadOptions {
    kind: ImageType
}