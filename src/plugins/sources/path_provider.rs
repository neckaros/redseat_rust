use std::{path::PathBuf, pin::Pin, str::FromStr};


use axum::async_trait;
use chrono::{Datelike, Utc};
use tokio::{fs::{remove_file, File}, io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWrite, BufReader, BufWriter}};

use crate::{domain::library::ServerLibrary, routes::mw_range::RangeDefinition};

use super::{error::{SourcesError, SourcesResult}, AsyncReadPinBox, FileStreamResult, RangeResponse, Source};

pub struct PathProvider {
    root: PathBuf
}

impl PathProvider {
    pub fn get_gull_path(&self, source: &str) -> PathBuf {
        let mut path = self.root.clone();
        path.push(&source);
        path
    }
}


#[async_trait]
impl Source for PathProvider {
    async fn new(library: ServerLibrary) -> SourcesResult<Self> {
        if let Some(root) = library.root {
            Ok(PathProvider {
                root: PathBuf::from_str(&root).map_err(|_| SourcesError::Error)?
            })
        } else {
            Err(SourcesError::Error)
        }
    }


    async fn exists(&self, source: &str) -> bool {
        let path = self.get_gull_path(&source);
        path.exists()
    }

    async fn remove(&self, source: &str) -> SourcesResult<()> {
        let path = self.get_gull_path(&source);
        remove_file(path).await?;
        Ok(())
    }

    async fn get_file_read_stream(&self, source: &str, range: Option<RangeDefinition>) -> SourcesResult<FileStreamResult<AsyncReadPinBox>> {
        let path = self.get_gull_path(&source);
        let guess = mime_guess::from_path(&source);
        let filename = path.file_name().map(|f| f.to_string_lossy().into_owned());

        let mut file = File::open(&path).await.map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                SourcesError::NotFound(path.to_str().map(|a| a.to_string()))
            } else {
                SourcesError::Io(err)
            }
        })?;
        let metadata = file.metadata().await?;
        
        let mut size = metadata.len();

        let mut total_size = metadata.len();

        let mut filereader = BufReader::new(file);

        let mut range_response = RangeResponse { total_size: Some(total_size.clone()), size: None, start: None, end: None };
        
        if let Some(range) = &range {

            if let Some(start) = range.start {
                filereader.seek( std::io::SeekFrom::Start(start)).await?;
                range_response.start = Some(start.clone());
                size = size - start;
                range_response.size = Some(size.clone());


            }
            if let Some(end) = range.end {
                let start = range.start.unwrap_or(0);
                let taken = filereader.take(end - start + 1);
                size = end - start + 1;
                range_response.end = Some(end.clone());
                range_response.size = Some(size.clone());
                //println!("range: {}", &range_response.header_value());
                return Ok(FileStreamResult {
                    stream: Box::pin(taken),
                    size: Some(size),
                    range: Some(range_response),
                    mime: guess.first(),
                    name: filename
                })
            }
        }

        
        println!("range: {}", &range_response.header_value());
        Ok(FileStreamResult {
            stream: Box::pin(filereader),
            size: Some(size),
            range: if range.is_some() { Some(range_response) } else {None},
            mime: guess.first(),
            name: filename
        })
    }

    async fn get_file_write_stream(&self, name: &str) -> SourcesResult<Pin<Box<dyn AsyncWrite + Send>>> {
        let mut path = self.root.clone();
        //let year = Utc::now().year().to_string();
        //path.push(year);
        path.push(name);
        
        let file = BufWriter::new(File::create(path).await?);
        
        Ok(Box::pin(file))
    }

}
