use std::{path::PathBuf, pin::Pin};

use axum::async_trait;
use hyper::{header, HeaderMap};
use mime::{Mime, APPLICATION_OCTET_STREAM};
use tokio::{fs::File, io::{AsyncRead, AsyncWrite, BufReader}};
use crate::domain::library::ServerLibrary;

use self::error::{SourcesError, SourcesResult};

pub mod path_provider;
pub mod virtual_provider;
pub mod error;

pub type AsyncReadPinBox = Pin<Box<dyn AsyncRead + Send>>;
pub struct FileStreamResult<T: Sized + AsyncRead + Send> {
    pub stream: T,
    pub size: Option<u64>,
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
        Ok(headers)
    }
}


#[async_trait]
pub trait Source: Send {
    async fn new(root: ServerLibrary) -> SourcesResult<Self> where Self: Sized;
    

    async fn exists(&self, name: &str) -> bool;
    async fn remove(&self, name: &str) -> SourcesResult<()>;
    async fn get_file_read_stream(&self, source: &str) -> SourcesResult<FileStreamResult<AsyncReadPinBox>>;
    async fn get_file_write_stream(&self, name: &str) -> SourcesResult<Pin<Box<dyn AsyncWrite + Send>>>;
    //async fn fill_file_information(&self, file: &mut ServerFile) -> SourcesResult<()>;
}

pub trait LocalSource: Send {
    fn get_gull_path(&self, source: String) -> PathBuf;

    //async fn fill_file_information(&self, file: &mut ServerFile) -> SourcesResult<()>;
}