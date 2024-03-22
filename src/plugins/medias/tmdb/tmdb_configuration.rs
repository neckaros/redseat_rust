use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TmdbConfiguration {
    pub images: TmdbImageConfiguration,
    #[serde(rename = "change_keys")]
    pub change_keys: Vec<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TmdbImageConfiguration {
    #[serde(rename = "base_url")]
    pub base_url: String,
    #[serde(rename = "secure_base_url")]
    pub secure_base_url: String,
    #[serde(rename = "backdrop_sizes")]
    pub backdrop_sizes: Vec<String>,
    #[serde(rename = "logo_sizes")]
    pub logo_sizes: Vec<String>,
    #[serde(rename = "poster_sizes")]
    pub poster_sizes: Vec<String>,
    #[serde(rename = "profile_sizes")]
    pub profile_sizes: Vec<String>,
    #[serde(rename = "still_sizes")]
    pub still_sizes: Vec<String>,
}