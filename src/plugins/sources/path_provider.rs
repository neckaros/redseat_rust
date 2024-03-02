use std::{path::PathBuf, str::FromStr};


use axum::async_trait;
use chrono::{Datelike, Utc};
use tokio::{fs::File, io::{AsyncWrite, BufReader, BufWriter}};

use crate::domain::library::ServerLibrary;

use super::{error::{SourcesError, SourcesResult}, FileStreamResult, Source};

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
    async fn get_file_read_stream(&self, source: &str) -> SourcesResult<FileStreamResult<BufReader<File>>> {
        let mut path = self.get_gull_path(&source);
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
            stream: filereader,
            size: Some(len.len()),
            mime: guess.first(),
            name: filename
        })
    }

    async fn get_file_write_stream(&self, name: &str) -> SourcesResult<Box<dyn AsyncWrite>> {
        let mut path = self.root.clone();
        let year = Utc::now().year().to_string();
        path.push(year);
        path.push(name);
        
        let file = BufWriter::new(File::create(path).await.map_err(|_| SourcesError::Error)?);
        
        Ok(Box::new(file))
    }

}
