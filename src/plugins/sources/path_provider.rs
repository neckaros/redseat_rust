use std::{path::PathBuf, str::FromStr};


use chrono::{Datelike, Utc};
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
        let len = file.metadata().unwrap().len();
        Ok(FileStreamResult {
            stream: Box::new(file),
            size: Some(0)
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


#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{self, Request, StatusCode, header},
    };
    use http_body_util::BodyExt; // for `collect`
    use serde_json::{json, Value};
    use tokio::io::AsyncWriteExt;
    use tokio_util::io::ReaderStream;
    use tower::ServiceExt; // for `call`, `oneshot`, and `ready`

    #[tokio::test]
    async fn copy() {
        
        let mut read_file = tokio::fs::OpenOptions::new()
            .read(true)
            .write(false)
            .create(false)
            .open("/Users/arnaudjezequel/Downloads/IMG_4303.HEIC")
            .await.unwrap();

        let mut write_file = tokio::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open("/Users/arnaudjezequel/Downloads/IMG_4305.HEIC")
            .await.unwrap();
        let mut write_file2 = tokio::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open("/Users/arnaudjezequel/Downloads/IMG_4304.HEIC")
            .await.unwrap();

        let mut stream = ReaderStream::new(read_file);
        
        while let Some(v) = stream.next().await {
            println!("GOT = {:?}", v);
            let data = v.unwrap();
            //panic!("nooo");
            write_file.write(&data).await.unwrap();
            write_file2.write(&data).await.unwrap();
        }
        //tokio::io::copy(&mut read_file, &mut write_file).await.unwrap();
    }
}