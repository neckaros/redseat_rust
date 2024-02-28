use std::{path::PathBuf, str::FromStr};


use axum::async_trait;
use chrono::{Datelike, Utc};
use tokio::{fs::File, io::{AsyncWrite, BufReader, BufWriter}};

use crate::domain::library::ServerLibrary;

use super::{error::{SourcesError, SourcesResult}, FileStreamResult, Source};

pub struct VirtualProvider {
    root: PathBuf
}

#[async_trait]
impl Source for VirtualProvider {
    async fn new(library: ServerLibrary) -> SourcesResult<Self> {
        if let Some(root) = library.root {
            Ok(VirtualProvider {
                root: PathBuf::from_str(&root).map_err(|_| SourcesError::Error)?
            })
        } else {
            Err(SourcesError::Error)
        }
    }

    async fn get_file_read_stream(&self, source: String) -> SourcesResult<FileStreamResult<BufReader<File>>> {
        let mut path = self.root.clone();
        path.push(source);
        
        let file = File::open(path).await.map_err(|_| SourcesError::Error)?;
        let len = file.metadata().await.map_err(|_| SourcesError::Error)?;
        let filereader = BufReader::new(file);
        Ok(FileStreamResult {
            stream: filereader,
            size: Some(len.len()),
            mime: None,
            name: None
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


       /* 
        while let Some(v) = read_file.next().await {
            println!("GOT = {:?}", v);
            let data = v.unwrap();
            //panic!("nooo");
            write_file.write(&data).await.unwrap();
            write_file2.write(&data).await.unwrap();
        }
        */
        //tokio::io::copy(&mut read_file, &mut write_file).await.unwrap();
    }
}