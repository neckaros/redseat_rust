use std::{fmt::{self, Debug}, fs::{remove_dir, remove_dir_all, remove_file}, io, path::PathBuf, pin::Pin, str::FromStr, sync::Arc};
use async_recursion::async_recursion;
use axum::{async_trait, body::Body, response::IntoResponse};
use futures::{future::BoxFuture, Future, Stream, StreamExt, TryStreamExt};
use hyper::{header, HeaderMap};
use mime::{Mime, APPLICATION_OCTET_STREAM};
use nanoid::nanoid;
use rs_plugin_common_interfaces::request::{RsRequest, RsRequestStatus};
use serde::{Deserialize, Serialize};
use tokio::{fs::File, io::{AsyncRead, AsyncWrite, BufReader}, sync::{mpsc::Sender, Mutex}};

use tokio_util::io::{ReaderStream, StreamReader};
use crate::{domain::{library::ServerLibrary, media::MediaForUpdate, progress::{RsProgress, RsProgressCallback}}, error::RsResult, model::{error::Error, users::ConnectedUser, ModelController}, routes::mw_range::RangeDefinition, tools::video_tools::ytdl::ProgressStreamItem};

use self::error::{SourcesError, SourcesResult};

pub mod path_provider;
pub mod virtual_provider;
pub mod error;
pub mod async_reader_progress;

pub type AsyncReadPinBox = Pin<Box<dyn AsyncRead + Send>>;
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RangeResponse {
    pub size: Option<u64>,
    pub start: Option<u64>,
    pub end: Option<u64>,
}
impl FromStr for RangeResponse {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let string = s.replace("bytes ", "");
        let splits: Vec<_> = string.split("/").collect();
        let range = splits.get(0).ok_or(crate::Error::Error("Unable to parse header range".to_owned()))?.split("-").collect::<Vec<&str>>();
        let start: Option<u64> = range.get(0).and_then(|s| s.parse().ok());
        let end: Option<u64> = range.get(1).and_then(|s| s.parse().ok());
        
        let size: Option<u64> = splits.get(1).and_then(|s| s.parse().ok());

        Ok(RangeResponse {
            size,
            start,
            end

        })
    }
}

impl RangeResponse {
    pub fn header_value(&self) -> String  {
        format!("bytes {}-{}/{}", self.start.unwrap_or(0), self.end.unwrap_or(self.size.unwrap_or(1) - 1), self.size.unwrap_or(0))
    }
}

#[derive(Debug)]
pub struct FileStreamResult<T: Sized + AsyncRead + Send> {
    pub stream: T,
    pub size: Option<u64>,
    pub accept_range: bool,
    pub range: Option<RangeResponse>,
    pub mime: Option<String>,
    pub name: Option<String>,
    pub cleanup: Option<Box<dyn Cleanup>>
}

#[async_trait]
pub trait Cleanup: Send + Debug {

}

#[derive(Debug)]
pub struct CleanupFiles {
    pub paths: Vec<PathBuf>
}
impl Cleanup for CleanupFiles{}
impl Drop for CleanupFiles {
    fn drop(&mut self) {
        for path in &self.paths {
            if path.is_dir() {
                let d = remove_dir_all(path);
                if let Err(error) = d {
                    println!("error: {:?}", error);
                }
                
            } else {
                let _ = remove_file(path);
            }
        }
    }
}


// impl<T: Sized + AsyncRead + Send> FileStreamResult<T> {
//     pub async fn from_path(path: &PathBuf) -> RsResult<Self> {
//         let source = tokio::fs::File::open(&path).await?;

//         Ok(FileStreamResult {
//             stream: source,
//             size: todo!(),
//             range: todo!(),
//             mime: todo!(),
//             name: todo!(),
//         })
//     }
// }

impl<T: Sized + AsyncRead + Send> FileStreamResult<T> {
    pub fn hearders(&self) -> SourcesResult<HeaderMap> {
        let mut headers = HeaderMap::new();
        let mime = self.mime.clone();
        headers.insert(header::CONTENT_TYPE, mime.unwrap_or("application/octet-stream".to_owned()).parse().map_err(|_| SourcesError::Error)?);
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
	Request(RsRequest),
}


type FuncResult = dyn Future<Output=crate::model::error::Result<RsRequest>>;

impl SourceRead { 
    #[async_recursion]
    pub async fn into_reader(self, library_id: &str, range: Option<RangeDefinition>, progress: Option<Sender<RsProgress>>, mc: Option<(ModelController, &ConnectedUser)>) -> RsResult<FileStreamResult<AsyncReadPinBox>> {
        match self {
            SourceRead::Stream(reader) => {
                Ok(reader)
            },
            SourceRead::Request(request) => {

                match request.status {
                    RsRequestStatus::Unprocessed | RsRequestStatus::NeedParsing => {
                        if let Some((mc, user)) = mc {
                            let new_request = mc.exec_request(request.clone(), Some(library_id.to_string()), false, progress, user).await?;
                            new_request.into_reader(library_id, range.clone(), None, Some((mc, user))).await
                        } else {
                            Err(Error::InvalidRsRequestStatus(request.status).into())
                        }
                    },
                    RsRequestStatus::FinalPrivate | RsRequestStatus::FinalPublic => {
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
                        let accept_range = if result_headers.get(reqwest::header::ACCEPT_RANGES).is_some() {
                            true
                        } else {
                            false
                        };
                        let range_response = if let Some(range) = result_headers.get(reqwest::header::CONTENT_RANGE) {
                            RangeResponse::from_str(range.to_str().map_err(|_| crate::Error::Error("Unable to get range header".to_owned()))?).ok()  
                        } else {
                            None
                        };
                        let size = if let Some(value) = result_headers.get(reqwest::header::CONTENT_LENGTH).and_then(|l| l.to_str().ok()).and_then(|s| s.parse::<u64>().ok()) {
                            Some(value)
                        } else {
                            request.size
                        };
                        let mime = if let Some(value) = result_headers.get(reqwest::header::CONTENT_TYPE).and_then(|h| h.to_str().ok()) {
                            Some(value.to_owned())
                        } else {
                            request.mime
                        };

                        let stream = r.bytes_stream();
                        let body_with_io_error = stream.map_err(|err| io::Error::new(io::ErrorKind::Other, err));
                        let body_reader = StreamReader::new(body_with_io_error);
                        let pinned: AsyncReadPinBox = Box::pin(body_reader);

                        


                        Ok(FileStreamResult {stream:pinned, accept_range, size, range: range_response, mime, name: request.filename, cleanup: None })



                    },
                    _ => Err(Error::InvalidRsRequestStatus(request.status).into())
                }

                
            },
        }
    }

    
    #[async_recursion]
    pub async fn into_response(self, library_id: &str, range: Option<RangeDefinition>, progress: RsProgressCallback, mc: Option<(ModelController, &ConnectedUser)>) -> RsResult<axum::response::Response> {
        match self {
            SourceRead::Stream(reader) => {
                let headers = reader.hearders().map_err(|_| Error::UnableToFormatHeaders)?;
                let stream = ReaderStream::new(reader.stream);
                let body = Body::from_stream(stream);
                let status = if reader.range.is_some() { axum::http::StatusCode::PARTIAL_CONTENT } else { axum::http::StatusCode::OK };
                Ok((status, headers, body).into_response())
            },
            SourceRead::Request(request) => {
                match request.status {
                    RsRequestStatus::Unprocessed | RsRequestStatus::NeedParsing => {
                        if let Some((mc, user)) = mc {
                            let new_request = mc.exec_request(request.clone(), Some(library_id.to_string()), false, progress, user).await?;
                            new_request.into_response(library_id, range.clone(), None, Some((mc, user))).await
                        } else {
                            Err(Error::InvalidRsRequestStatus(request.status).into())
                        }
                    },
                    RsRequestStatus::FinalPrivate => {
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
                    RsRequestStatus::FinalPublic => {
                        let mut headers = axum::http::HeaderMap::new();
                        headers.append(axum::http::header::LOCATION, request.url.parse().unwrap());
                        let status = axum::http::StatusCode::TEMPORARY_REDIRECT;
                        let body = Body::empty();
                        Ok((status, headers, body).into_response())
                    },
                    _ => Err(Error::InvalidRsRequestStatus(request.status).into())
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
    fn local_path(&self, source: &str) -> Option<PathBuf>;
    async fn get_file(&self, source: &str, range: Option<RangeDefinition>) -> SourcesResult<SourceRead>;
    async fn get_file_write_stream(&self, name: &str) -> SourcesResult<(String, Pin<Box<dyn AsyncWrite + Send>>)>;
    //async fn fill_file_information(&self, file: &mut ServerFile) -> SourcesResult<()>;
}

pub trait LocalSource: Send {
    fn get_gull_path(&self, source: String) -> PathBuf;

    //async fn fill_file_information(&self, file: &mut ServerFile) -> SourcesResult<()>;
}




#[cfg(test)]
mod tests {
    use super::*;

 
    #[test]
    fn test_header() -> Result<(), crate::Error> {
        let parsed = RangeResponse::from_str("bytes 0-1023/146515")?;
        println!("parsed: {:?}", parsed);
        assert_eq!(parsed.size, Some(146515), "test size parsing");
        assert_eq!(parsed.start, Some(0), "test start parsing");
        assert_eq!(parsed.end, Some(1023), "test end parsing");
        Ok(())
    }
}