use std::{path::PathBuf, pin::Pin, str::FromStr};


use axum::async_trait;
use chrono::{Datelike, Utc};
use tokio::{fs::{remove_file, File}, io::{AsyncRead, AsyncWrite, BufReader, BufWriter}};

use crate::domain::library::ServerLibrary;

use super::{error::{SourcesError, SourcesResult}, AsyncReadPinBox, FileStreamResult, Source};

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

    async fn get_file_read_stream(&self, source: &str) -> SourcesResult<FileStreamResult<AsyncReadPinBox>> {
        let path = self.get_gull_path(&source);
        let guess = mime_guess::from_path(&source);
        let filename = path.file_name().map(|f| f.to_string_lossy().into_owned());

        let file = File::open(&path).await.map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                SourcesError::NotFound(path.to_str().map(|a| a.to_string()))
            } else {
                SourcesError::Io(err)
            }
        })?;

        let len = file.metadata().await?;
        let filereader = BufReader::new(file);
        Ok(FileStreamResult {
            stream: Box::pin(filereader),
            size: Some(len.len()),
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
