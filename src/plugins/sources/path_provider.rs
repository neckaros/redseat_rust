use std::{path::PathBuf, pin::Pin, str::FromStr};


use axum::async_trait;
use chrono::{Datelike, Utc};
use query_external_ip::SourceError;
use sha256::try_async_digest;
use tokio::{fs::{create_dir_all, remove_file, File}, io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWrite, BufReader, BufWriter}};

use crate::{domain::{library::ServerLibrary, media::MediaForUpdate}, model::ModelController, routes::mw_range::RangeDefinition, tools::{file_tools::get_mime_from_filename, image_tools::resize_image_reader, log::log_info}};

use super::{error::{SourcesError, SourcesResult}, AsyncReadPinBox, FileStreamResult, RangeResponse, Source, SourceRead};

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
    async fn new(library: ServerLibrary, _: ModelController) -> SourcesResult<Self> {
        if let Some(root) = library.root {
            Ok(PathProvider {
                root: PathBuf::from_str(&root).map_err(|_| SourcesError::Error)?,
                for_local: false
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

    async fn thumb(&self, source: &str) -> SourcesResult<Vec<u8>> {
        let reader = self.get_file(source, None).await?;
        if let SourceRead::Stream(mut reader) = reader {
        let image = resize_image_reader(&mut reader.stream, 512).await?;
        Ok(image)
        } else {
            Err(SourcesError::Error)
        }
    }

    fn local_path(&self, source: &str) -> Option<PathBuf> {
        Some(self.get_gull_path(&source))
    }

    async fn fill_infos(&self, source: &str, infos: &mut MediaForUpdate) -> SourcesResult<()> {
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

    async fn get_file(&self, source: &str, range: Option<RangeDefinition>) -> SourcesResult<SourceRead> {
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

        let mut range_response = RangeResponse { total_size: Some(total_size.clone()), size: None, start: None, end: None };
        
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
                    range_response.size = Some(size.clone());
                    //println!("range: {}", &range_response.header_value());
                    return Ok(SourceRead::Stream(FileStreamResult {
                        stream: Box::pin(taken),
                        size: Some(size),
                        range: Some(range_response),
                        mime: guess.first(),
                        name: filename
                    }))
                }
            }
        }

        Ok(SourceRead::Stream(FileStreamResult {
            stream: Box::pin(filereader),
            size: Some(size),
            range: if range.is_some() { Some(range_response) } else {None},
            mime: guess.first(),
            name: filename
        }))
    }

    async fn get_file_write_stream(&self, name: &str) -> SourcesResult<(String, Pin<Box<dyn AsyncWrite + Send>>)> {
        let mut path = self.root.clone();
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
