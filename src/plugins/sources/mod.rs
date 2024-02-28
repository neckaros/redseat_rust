use std::sync::Arc;

use axum::async_trait;
use hyper::{header, HeaderMap};
use mime::{Mime, APPLICATION_OCTET_STREAM};
use tokio::{fs::File, io::{AsyncRead, AsyncWrite, BufReader}};
use crate::domain::library::ServerLibrary;

use self::{error::{SourcesError, SourcesResult}, path_provider::PathProvider, virtual_provider::VirtualProvider};

pub mod path_provider;
pub mod virtual_provider;
pub mod error;


pub struct FileStreamResult<T: Sized + AsyncRead> {
    pub stream: T,
    pub size: Option<u64>,
    pub mime: Option<Mime>,
    pub name: Option<String>,
}

impl<T: Sized + AsyncRead> FileStreamResult<T> {
    pub fn hearders(&self) -> SourcesResult<HeaderMap> {
        let mut headers = HeaderMap::new();
        let mime = self.mime.clone();
        headers.insert(header::CONTENT_TYPE, mime.unwrap_or(APPLICATION_OCTET_STREAM).to_string().parse().map_err(|e| SourcesError::Error)?);
        if let Some(name) = self.name.clone() {
            headers.insert(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{:?}\"", name).parse().map_err(|e| SourcesError::Error)?);
        }
        Ok(headers)
    }
}

pub struct SourceManager {
    pub source: Box<dyn Source>
}
impl SourceManager {
    pub async fn new(library: ServerLibrary) -> SourcesResult<Self> {
        let source: Box<dyn Source> = if library.source == "PathProvider" {
            let source = PathProvider::new(library).await?;
            Box::new(source)
        } else {
            let source = VirtualProvider::new(library).await?;
            Box::new(source)
        };
        Ok(SourceManager { source: source })
    }
}

#[async_trait]
pub trait Source: Send {
    async fn new(root: ServerLibrary) -> SourcesResult<Self> where Self: Sized;
    async fn get_file_read_stream(&self, source: String) -> SourcesResult<FileStreamResult<BufReader<File>>>;
    async fn get_file_write_stream(&self, name: &str) -> SourcesResult<Box<dyn AsyncWrite>>;
    //async fn fill_file_information(&self, file: &mut ServerFile) -> SourcesResult<()>;
}
