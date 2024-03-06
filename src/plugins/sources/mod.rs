use std::{path::PathBuf, pin::Pin};

use axum::async_trait;
use hyper::{header, HeaderMap};
use mime::{Mime, APPLICATION_OCTET_STREAM};
use serde::{Deserialize, Serialize};
use tokio::{fs::File, io::{AsyncRead, AsyncWrite, BufReader}};
use crate::{domain::library::ServerLibrary, routes::mw_range::RangeDefinition};

use self::error::{SourcesError, SourcesResult};

pub mod path_provider;
pub mod virtual_provider;
pub mod error;

pub type AsyncReadPinBox = Pin<Box<dyn AsyncRead + Send>>;
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RangeResponse {
    pub total_size: Option<u64>,
    pub size: Option<u64>,
    pub start: Option<u64>,
    pub end: Option<u64>,
}

impl RangeResponse {
    pub fn header_value(&self) -> String  {
        format!("bytes {}-{}/{}", self.start.unwrap_or(0), self.end.unwrap_or(self.total_size.unwrap_or(0) - 1), self.total_size.unwrap_or(0))
    }
}
pub struct FileStreamResult<T: Sized + AsyncRead + Send> {
    pub stream: T,
    pub size: Option<u64>,
    pub range: Option<RangeResponse>,
    pub mime: Option<Mime>,
    pub name: Option<String>,
}

impl<T: Sized + AsyncRead + Send> FileStreamResult<T> {
    pub fn hearders(&self) -> SourcesResult<HeaderMap> {
        let mut headers = HeaderMap::new();
        let mime = self.mime.clone();
        headers.insert(header::CONTENT_TYPE, mime.unwrap_or(APPLICATION_OCTET_STREAM).to_string().parse().map_err(|_| SourcesError::Error)?);
        if let Some(name) = self.name.clone() {
            headers.insert(header::CONTENT_DISPOSITION, format!("attachment; filename={:?}", name).parse().map_err(|_| SourcesError::Error)?);
        }
        if let Some(size) = self.size.clone() {
            headers.insert(header::CONTENT_LENGTH, size.to_string().parse().map_err(|_| SourcesError::Error)?);
        }
        headers.append(header::ACCEPT_RANGES, "bytes".parse().map_err(|_| SourcesError::Error)?);
    
        if let Some(range) = &self.range {
            headers.append(header::CONTENT_RANGE, range.header_value().parse().map_err(|_| SourcesError::Error)?);
        }

        Ok(headers)
    }
}


#[async_trait]
pub trait Source: Send {
    async fn new(root: ServerLibrary) -> SourcesResult<Self> where Self: Sized;
    

    async fn exists(&self, name: &str) -> bool;
    async fn remove(&self, name: &str) -> SourcesResult<()>;
    async fn get_file_read_stream(&self, source: &str, range: Option<RangeDefinition>) -> SourcesResult<FileStreamResult<AsyncReadPinBox>>;
    async fn get_file_write_stream(&self, name: &str) -> SourcesResult<Pin<Box<dyn AsyncWrite + Send>>>;
    //async fn fill_file_information(&self, file: &mut ServerFile) -> SourcesResult<()>;
}

pub trait LocalSource: Send {
    fn get_gull_path(&self, source: String) -> PathBuf;

    //async fn fill_file_information(&self, file: &mut ServerFile) -> SourcesResult<()>;
}