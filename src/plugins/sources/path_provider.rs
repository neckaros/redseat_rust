use std::{path::PathBuf, pin::Pin, str::FromStr};


use axum::async_trait;
use bytes::Bytes;
use chrono::{Datelike, Utc};
use futures::Stream;
use query_external_ip::SourceError;
use sha256::try_async_digest;
use tokio::{fs::{create_dir_all, remove_file, File}, io::{copy, AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWrite, AsyncWriteExt, BufReader, BufWriter}};

use crate::{domain::{library::ServerLibrary, media::MediaForUpdate}, error::RsResult, model::ModelController, routes::mw_range::RangeDefinition, tools::{file_tools::get_mime_from_filename, image_tools::resize_image_reader, log::log_info}};

use super::{error::{SourcesError, SourcesResult}, AsyncReadPinBox, AsyncSeekableWrite, FileStreamResult, RangeResponse, Source, SourceRead};

pub struct PathProvider {
    root: PathBuf,
    for_local: bool
}

impl PathProvider {
    pub fn get_gull_path(&self, source: &str) -> PathBuf {
        let mut path = self.root.clone();
        path.push(&source);
        path
    }

    pub async fn ensure_filepath(full_file_path: &PathBuf) -> SourcesResult<()> {
        if let Some(p) = full_file_path.parent() {
            create_dir_all(&p).await?;
        }
        Ok(())
    }
    

    pub async fn get_file_write_stream(&self, name: &str) -> SourcesResult<(String, Pin<Box<dyn AsyncWrite + Send>>)> {
        let path = self.root.clone();
        let mut sourcepath = PathBuf::new();

        if !self.for_local {
            let year = Utc::now().year().to_string();
            sourcepath.push(year);
            let month = Utc::now().month().to_string();
            sourcepath.push(month);
        }
        let mut folder = path.clone();
        folder.push(&sourcepath);
       

        let mut file_path = path.clone();
        let original_source = sourcepath.clone();
        sourcepath.push(&name);
        file_path.push(&sourcepath);
        
        if let Some(p) = file_path.parent() {
            create_dir_all(&p).await?;
        }
        

        let original_name = name;
        let mut i = 1;
        while file_path.exists() {
            i = i + 1;
            let extension = file_path.extension().and_then(|r| r.to_str());
            let new_name = if let Some(extension) = extension {
                original_name.replace(&format!(".{}", extension), &format!("-{}.{}", i, extension))
            } else {
                format!("{}-{}", original_name, i)
            };
            file_path = path.clone();
            sourcepath = original_source.clone();
            sourcepath.push(new_name);
            file_path.push(&sourcepath);
        }
    
    
        let file = BufWriter::new(File::create(&file_path).await?);
        let source = sourcepath.to_str().ok_or(SourcesError::Other("Unable to convert path to string".into()))?.to_string();
        Ok((source.to_string(), Box::pin(file)))
    }
}

impl PathProvider {
    
    pub fn new_for_local(path: PathBuf) -> Self {
        PathProvider {
            root: path,
            for_local: true
        }
    }
}

#[async_trait]
impl Source for PathProvider {
    async fn new(library: ServerLibrary, _: ModelController) -> RsResult<Self> {
        if let Some(root) = library.root {
            Ok(PathProvider {
                root: PathBuf::from_str(&root).map_err(|_| SourcesError::Error)?,
                for_local: false
            })
        } else {
            Err(SourcesError::Error.into())
        }
    }

    async fn exists(&self, source: &str) -> bool {
        let path = self.get_gull_path(&source);
        path.exists()
    }

    async fn remove(&self, source: &str) -> RsResult<()> {
        let path = self.get_gull_path(&source);
        remove_file(path).await?;
        Ok(())
    }

    async fn fill_infos(&self, source: &str, infos: &mut MediaForUpdate) -> RsResult<()> {
        let path = self.get_gull_path(&source);
        let metadata = path.metadata()?;
        infos.size = Some(metadata.len());

        let md5 = try_async_digest(&path).await;
        if let Ok(md5) = md5 {
            infos.md5 = Some(md5);
        } 
        let mime = get_mime_from_filename(source);
        if let Some(mime) = mime {
            infos.mimetype = Some(mime);
        }
        Ok(())
    }

    fn local_path(&self, source: &str) -> Option<PathBuf> {
        Some(self.get_gull_path(&source))
    }

    async fn get_file(&self, source: &str, range: Option<RangeDefinition>) -> RsResult<SourceRead> {
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
        let metadata = file.metadata().await?;
        
        let mut size = metadata.len();

        let total_size = metadata.len();

        let mut filereader = BufReader::new(file);

        let mut range_response = RangeResponse { size: Some(total_size.clone()), start: None, end: None };
        
        if let Some(range) = &range {

            if let Some(start) = range.start {
                if start < size { 
                    filereader.seek( std::io::SeekFrom::Start(start)).await?;
                    range_response.start = Some(start.clone());
                    size = size - start;
                    range_response.size = Some(size.clone());

                }

            }
            if let Some(end) = range.end {
                if end <= size {
                    let start = range.start.unwrap_or(0);
                    let taken = filereader.take(end - start + 1);
                    size = end - start + 1;
                    range_response.end = Some(end.clone());
                    range_response.size = Some(total_size.clone());
                    //println!("range: {}", &range_response.header_value());
                    return Ok(SourceRead::Stream(FileStreamResult {
                        stream: Box::pin(taken),
                        size: Some(size),
                        accept_range: true,
                        range: Some(range_response),
                        mime: guess.first().and_then(|g| Some(g.to_string())),
                        name: filename,
                        cleanup: None
                    }))
                }
            } else {
                range_response.end = Some(total_size - 1);
                range_response.size = Some(total_size.clone());
            }
        }

        Ok(SourceRead::Stream(FileStreamResult {
            stream: Box::pin(filereader),
            size: Some(size),
            accept_range: true,
            range: if range.is_some() { Some(range_response) } else {None},
            mime: guess.first().and_then(|g| Some(g.to_string())),
            name: filename,
            cleanup: None
        }))
    }



    async fn write<'a>(&self, name: &str, mut read: Pin<Box<dyn AsyncRead + Send + 'a>>) -> RsResult<String> {
        
        let (source, mut file) = self.writer(name).await?;
        copy(&mut read, &mut file).await?;
        file.flush().await?;
        Ok(source.to_string())
    }

    async fn writer<'a>(&self, name: &str) -> RsResult<(String, Pin<Box<dyn AsyncSeekableWrite + 'a>>)> {
        let path = self.root.clone();
        let mut sourcepath = PathBuf::new();

        if !self.for_local {
            let year = Utc::now().year().to_string();
            sourcepath.push(year);
            let month = Utc::now().month().to_string();
            sourcepath.push(month);
        }
        let mut folder = path.clone();
        folder.push(&sourcepath);
       

        let mut file_path = path.clone();
        let original_source = sourcepath.clone();
        sourcepath.push(name);
        file_path.push(&sourcepath);
        
        if let Some(p) = file_path.parent() {
            create_dir_all(&p).await?;
        }
        

        let original_name = name;
        let mut i = 1;
        while file_path.exists() {
            i += 1;
            let extension = file_path.extension().and_then(|r| r.to_str());
            let new_name = if let Some(extension) = extension {
                original_name.replace(&format!(".{}", extension), &format!("-{}.{}", i, extension))
            } else {
                format!("{}-{}", original_name, i)
            };
            file_path = path.clone();
            sourcepath = original_source.clone();
            sourcepath.push(new_name);
            file_path.push(&sourcepath);
        }
    
    
        let source = sourcepath.to_str().ok_or(SourcesError::Other("Unable to convert path to string".into()))?.to_string();
        let file = BufWriter::new(File::create(&file_path).await?);

        Ok((source, Box::pin(file)))
    }

}




impl PathProvider {
    /// Will replace existing library file
    pub async fn get_file_write_library_overwrite(&self, name: &str) -> SourcesResult<BufWriter<File>> {
        let mut path = self.root.clone();
        path.push(&name);
        if let Some(p) = path.parent() {
            create_dir_all(&p).await?;
        }
        let file = BufWriter::new(File::create(&path).await?);
        Ok(file)
    }
    pub async fn get_file_library(&self, name: &str) -> SourcesResult<BufReader<File>> {
        let mut path = self.root.clone();
        path.push(&name);
        let file = BufReader::new(File::open(&path).await?);
        Ok(file)
    }

}