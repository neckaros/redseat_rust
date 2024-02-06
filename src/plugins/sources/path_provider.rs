use std::{path::PathBuf, str::FromStr};

use tokio::{fs::File, io::{AsyncWrite, BufReader, BufWriter}};

use super::{error::{SourcesError, SourcesResult}, FileStreamResult, Source};

pub struct PathProvider {
    root: PathBuf
}

impl Source for PathProvider {
    async fn new(root: String) -> SourcesResult<Self> {
        Ok(PathProvider {
            root: PathBuf::from_str(&root).map_err(|_| SourcesError::Error)?
        })
    }

    async fn get_file_read_stream(&self, source: String) -> SourcesResult<FileStreamResult> {
        let mut path = self.root.clone();
        path.push(source);
        
        let file = BufReader::new(File::open(path).await.map_err(|_| SourcesError::Error)?);

        Ok(FileStreamResult {
            stream: Box::new(file),
            size: Some(0)
        })
    }

    async fn get_file_write_stream(&self, source: String) -> SourcesResult<Box<dyn AsyncWrite>> {
        let mut path = self.root.clone();
        path.push(source);
        
        let file = BufWriter::new(File::create(path).await.map_err(|_| SourcesError::Error)?);
        
        Ok(Box::new(file))
    }
}