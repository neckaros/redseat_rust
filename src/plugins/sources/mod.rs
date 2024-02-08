use tokio::io::{AsyncRead, AsyncWrite};


use crate::domain::file::ServerFile;

use self::error::SourcesResult;

mod path_provider;
mod error;


pub struct FileStreamResult {
    stream: Box<dyn AsyncRead>,
    size: Option<usize>,
}

trait Source {
    async fn new(root: String) -> SourcesResult<Self> where Self: Sized;
    async fn get_file_read_stream(&self, source: String) -> SourcesResult<FileStreamResult>;
    async fn get_file_write_stream(&self, name: &str) -> SourcesResult<Box<dyn AsyncWrite>>;
    //async fn fill_file_information(&self, file: &mut ServerFile) -> SourcesResult<()>;
}