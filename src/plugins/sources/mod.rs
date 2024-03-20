use std::{path::PathBuf, pin::Pin};
use async_recursion::async_recursion;
use axum::{async_trait, body::Body, response::IntoResponse};
use futures::{future::BoxFuture, Future};
use hyper::{header, HeaderMap};
use mime::{Mime, APPLICATION_OCTET_STREAM};
use plugin_request_interfaces::RsRequest;
use serde::{Deserialize, Serialize};
use tokio::{fs::File, io::{AsyncRead, AsyncWrite, BufReader}};
use tokio_util::io::ReaderStream;
use crate::{domain::{library::ServerLibrary, media::MediaForUpdate}, model::{error::Error, users::ConnectedUser, ModelController}, routes::mw_range::RangeDefinition};

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


pub enum SourceRead {
	Stream(FileStreamResult<AsyncReadPinBox>),
	Request(RsRequest)
}

type FuncResult = dyn Future<Output=crate::model::error::Result<RsRequest>>;

impl SourceRead { 
    

    
    #[async_recursion]
    pub async fn into_response(self, range: Option<RangeDefinition>, mc: Option<(ModelController, &ConnectedUser)>) -> Result<axum::response::Response, Error> {
        match self {
            SourceRead::Stream(reader) => {
                let headers = reader.hearders().map_err(|_| Error::UnableToFormatHeaders)?;
                let stream = ReaderStream::new(reader.stream);
                let body = Body::from_stream(stream);
                //println!("range req: {:?}", range);
                let status = if reader.range.is_some() { axum::http::StatusCode::PARTIAL_CONTENT } else { axum::http::StatusCode::OK };
                Ok((status, headers, body).into_response())
            },
            SourceRead::Request(request) => {

                match request.status {
                    plugin_request_interfaces::RsRequestStatus::Unprocessed => {
                        if let Some((mc, user)) = mc {
                            let new_request = mc.exec_request(request.clone(), None, user).await?.ok_or(Error::InvalidRsRequestStatus(request.status))?;
                            let read = SourceRead::Request(new_request);
                            read.into_response(range.clone(), Some((mc, user))).await
                        } else {
                            Err(Error::InvalidRsRequestStatus(request.status))
                        }
                    },
                    plugin_request_interfaces::RsRequestStatus::FinalPrivate => {
                        let mut headers = reqwest::header::HeaderMap::new();
                        for (key, value) in request.headers.unwrap_or(vec![]) {
                            headers.insert(reqwest::header::HeaderName::from_lowercase(key.to_lowercase().as_bytes()).map_err(|_| Error::UnableToFormatHeaders)?, reqwest::header::HeaderValue::from_str(&value).map_err(|_| Error::UnableToFormatHeaders)?);
                        }
                         if let Some(range) = range {
                             let (key, val) = range.header();
                             headers.insert(key, val);
                         }
                        
                        let client = reqwest::Client::new();
                        let r = client.get(request.url)
                            .headers(headers)
                            .send().await?;
                        
                        let result_headers: &reqwest::header::HeaderMap = r.headers();
                        let mut response_headers = axum::http::HeaderMap::new();
                        
                        if let Some(accept) = result_headers.get(reqwest::header::ACCEPT_RANGES) {
                            response_headers.append(axum::http::header::ACCEPT_RANGES, axum::http::header::HeaderValue::from_bytes(accept.as_bytes()).map_err(|_| Error::UnableToFormatHeaders)? );
                        }
                        if let Some(range) = result_headers.get(reqwest::header::CONTENT_RANGE) {
                            response_headers.append(axum::http::header::CONTENT_RANGE, axum::http::header::HeaderValue::from_bytes(range.as_bytes()).map_err(|_| Error::UnableToFormatHeaders)? );
                        }
                        if let Some(value) = result_headers.get(reqwest::header::CONTENT_LENGTH) {
                            response_headers.append(axum::http::header::CONTENT_LENGTH, axum::http::header::HeaderValue::from_bytes(value.as_bytes()).map_err(|_| Error::UnableToFormatHeaders)? );
                        }
                        if let Some(value) = result_headers.get(reqwest::header::CONTENT_TYPE) {
                            response_headers.append(axum::http::header::CONTENT_TYPE, axum::http::header::HeaderValue::from_bytes(value.as_bytes()).map_err(|_| Error::UnableToFormatHeaders)? );
                        }
                        if let Some(value) = result_headers.get(reqwest::header::CONTENT_DISPOSITION) {
                            response_headers.append(axum::http::header::CONTENT_DISPOSITION, axum::http::header::HeaderValue::from_bytes(value.as_bytes()).map_err(|_| Error::UnableToFormatHeaders)? );
                        }
                        let code = r.status().as_u16();
                        let status = axum::http::StatusCode::from_u16(code).map_err(|_| Error::UnableToFormatHeaders)?;

                        let body = Body::from_stream(r.bytes_stream());

                        Ok((status, response_headers, body).into_response())



                    },
                    plugin_request_interfaces::RsRequestStatus::FinalPublic => {
                        let mut headers = axum::http::HeaderMap::new();
                        headers.append(axum::http::header::LOCATION, request.url.parse().unwrap());
                        let status = axum::http::StatusCode::TEMPORARY_REDIRECT;
                        let body = Body::empty();
                        Ok((status, headers, body).into_response())
                    },
                    _ => Err(Error::InvalidRsRequestStatus(request.status))
                }

                
            },
        }
    }
}

#[async_trait]
pub trait Source: Send {
    async fn new(root: ServerLibrary, controller: ModelController) -> SourcesResult<Self> where Self: Sized;
    

    async fn exists(&self, name: &str) -> bool;
    async fn remove(&self, name: &str) -> SourcesResult<()>;
    async fn fill_infos(&self, source: &str, infos: &mut MediaForUpdate) -> SourcesResult<()>;
    async fn thumb(&self, source: &str) -> SourcesResult<Vec<u8>>;
    async fn get_file(&self, source: &str, range: Option<RangeDefinition>) -> SourcesResult<SourceRead>;
    async fn get_file_write_stream(&self, name: &str) -> SourcesResult<(String, Pin<Box<dyn AsyncWrite + Send>>)>;
    //async fn fill_file_information(&self, file: &mut ServerFile) -> SourcesResult<()>;
}

pub trait LocalSource: Send {
    fn get_gull_path(&self, source: String) -> PathBuf;

    //async fn fill_file_information(&self, file: &mut ServerFile) -> SourcesResult<()>;
}