use std::{io, path::PathBuf, pin::Pin, str::FromStr};


use axum::async_trait;
use chrono::{Datelike, Utc};
use query_external_ip::SourceError;
use tokio::{fs::File, io::{AsyncRead, AsyncWrite, BufReader, BufWriter}};

use crate::{domain::library::ServerLibrary, routes::mw_range::RangeDefinition, server::get_server_file_path_array};

use super::{error::{SourcesError, SourcesResult}, AsyncReadPinBox, FileStreamResult, Source};

pub struct VirtualProvider {
    root: PathBuf,
    library: ServerLibrary
}

#[async_trait]
impl Source for VirtualProvider {
    async fn new(library: ServerLibrary) -> SourcesResult<Self> {
        if let Some(root) = &library.root {
            Ok(VirtualProvider {
                root: PathBuf::from_str(&root).map_err(|_| SourcesError::Error)?,
                library
            })
        } else {
            Err(SourcesError::Error)
        }
    }

    async fn exists(&self, source: &str) -> bool {
        true
    }
    async fn remove(&self, source: &str) -> SourcesResult<()> {

        Ok(())
    }
    async fn get_file_read_stream(&self, source: &str, range: Option<RangeDefinition>) -> SourcesResult<FileStreamResult<AsyncReadPinBox>> {
        println!("Virtual {}", &source);
        let mut path = self.root.clone();
        path.push(source);
        
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
            range: None,
            mime: None,
            name: None
        })
    }

    async fn get_file_write_stream(&self, name: &str) -> SourcesResult<Pin<Box<dyn AsyncWrite + Send>>> {
        let mut path = self.root.clone();
        let year = Utc::now().year().to_string();
        path.push(year);
        path.push(name);
        
        let file = BufWriter::new(File::create(path).await.map_err(|_| SourcesError::Error)?);
        
        Ok(Box::pin(file))
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