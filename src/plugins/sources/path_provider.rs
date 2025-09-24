use std::{collections::HashSet, fs::read_dir, path::{Path, PathBuf}, pin::Pin, str::FromStr, time::{Duration, SystemTime}};


use axum::{async_trait, extract::path};
use bytes::Bytes;
use chrono::{Datelike, Utc};
use futures::Stream;
use human_bytes::human_bytes;
use query_external_ip::SourceError;
use sha256::try_async_digest;
use tokio::{fs::{create_dir_all, remove_file, File}, io::{copy, AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWrite, AsyncWriteExt, BufReader, BufWriter}};

use crate::{domain::{backup::Backup, library::ServerLibrary, media::MediaForUpdate}, error::{RsError, RsResult}, model::ModelController, routes::mw_range::RangeDefinition, tools::{file_tools::get_mime_from_filename, image_tools::resize_image_reader, log::{log_error, log_info, LogServiceType}}};

use super::{error::{SourcesError, SourcesResult}, AsyncReadPinBox, AsyncSeekableWrite, BoxedStringFuture, FileStreamResult, RangeResponse, Source, SourceRead};

pub struct PathProvider {
    root: PathBuf,
    for_local: bool
}

impl PathProvider {
    pub fn get_full_path(&self, source: &str) -> PathBuf {
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

    pub fn get_all_file_paths(dir: &Path, include_hidden: bool) -> HashSet<String> {
        let mut file_paths = HashSet::new();
    
        if dir.is_dir() {
            for entry in read_dir(dir).expect("Failed to read directory") {
                let entry = entry.expect("Failed to get directory entry");
                let path = entry.path();
                let filename = entry.file_name();
                
                if path.is_file() {
                    file_paths.insert(path.to_string_lossy().into_owned());
                } else if path.is_dir() && (include_hidden || !filename.to_string_lossy().starts_with(".")) {
                    file_paths.extend(PathProvider::get_all_file_paths(&path, include_hidden));
                }
            }
        }
    
        file_paths
    }
    
    pub fn move_to_trash<P: AsRef<Path>>(path: P) -> RsResult<()> {
        trash::delete(path)?;
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

    async fn new_from_backup(backup: Backup, _: ModelController) -> RsResult<Self> {
         Ok(PathProvider {
                root: PathBuf::from_str(&backup.path).map_err(|_| SourcesError::Error)?,
                for_local: true
            })
    }

    async fn init(&self) -> SourcesResult<()> {
        
        log_info(LogServiceType::LibraryCreation, format!("init libary {}", self.root.to_string_lossy()));
        let path_lib = self.get_full_path(".redseat");
        
        log_info(LogServiceType::LibraryCreation, format!("init libary {} - creating dir {}", self.root.to_string_lossy(), path_lib.to_string_lossy()));
        create_dir_all(path_lib).await?;

        let path_lib = self.get_full_path(".redseat/.thumbs");
        log_info(LogServiceType::LibraryCreation, format!("init libary {} - creating dir {}", self.root.to_string_lossy(), path_lib.to_string_lossy()));
        create_dir_all(path_lib).await?;
        let path_lib = self.get_full_path(".redseat/.portraits");
        log_info(LogServiceType::LibraryCreation, format!("init libary {} - creating dir {}", self.root.to_string_lossy(), path_lib.to_string_lossy()));
        create_dir_all(path_lib).await?;
        let path_lib = self.get_full_path(".redseat/.cache");
        log_info(LogServiceType::LibraryCreation, format!("init libary {} - creating dir {}", self.root.to_string_lossy(), path_lib.to_string_lossy()));
        create_dir_all(path_lib).await?;
        let path_lib = self.get_full_path(".redseat/.series");
        log_info(LogServiceType::LibraryCreation, format!("init libary {} - creating dir {}", self.root.to_string_lossy(), path_lib.to_string_lossy()));
        create_dir_all(path_lib).await?;
        Ok(())
    }

    async fn exists(&self, source: &str) -> bool {
        let path = self.get_full_path(&source);
        path.exists()
    }

    async fn remove(&self, source: &str) -> RsResult<()> {
        let path = self.get_full_path(&source);
        remove_file(path).await?;
        Ok(())
    }

    async fn fill_infos(&self, source: &str, infos: &mut MediaForUpdate) -> RsResult<()> {
        let path = self.get_full_path(&source);
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
        Some(self.get_full_path(&source))
    }



    async fn get_file(&self, source: &str, range: Option<RangeDefinition>) -> RsResult<SourceRead> {
        let path = self.get_full_path(&source);
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





    async fn writer(&self, name: &str, length: Option<u64>, mime: Option<String>) -> RsResult<(BoxedStringFuture, Pin<Box<dyn AsyncWrite + Send>>)> {
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

        let source = Box::pin(async {
            Ok::<RsResult<String>, RsError>(Ok::<String, RsError>(source))
        });
        Ok((source, Box::pin(file)))
    }



    async fn writerseek(&self, name: &str) -> RsResult<(String, Pin<Box<dyn AsyncSeekableWrite + Send>>)> {
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

    async fn clean(&self, sources: Vec<String>) -> RsResult<Vec<(String, u64)>> {
        let mut result = vec![];
        println!("CLEAN get paths");
        let existing = Self::get_all_file_paths(&self.root, false);
        println!("Got {} paths", existing.len());
        let mut total = 0u64;
        for existing_file in existing.iter() {
            let existing_as_source = existing_file.replace(&self.root.to_string_lossy().into_owned(), "")[1..].to_string();
            
            if !sources.contains(&existing_as_source) {
                let path = Path::new(&existing_file);
                let metadata = path.metadata()?;
                println!("{} TO DELETE {}", existing_as_source, metadata.len());
                result.push((path.to_string_lossy().into_owned(), metadata.len()));
                total += metadata.len();
                Self::move_to_trash(path)?;
            }
        }
        println!("Total clean: {} ({})", result.len(), human_bytes(total as f64));
        let existing_files_sources: Vec<String> = existing.into_iter().map(|existing_file| existing_file.replace(&self.root.to_string_lossy().into_owned(), "")[1..].to_string()).collect();
        for source in sources {
            if !existing_files_sources.contains(&source) {
                log_error(crate::tools::log::LogServiceType::Other, format!("Unable to find file {}", source));
            }
        }
        Ok(result)
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

    pub fn clean_temp(&self) -> RsResult<Vec<(String, u64)>> {
        let dest = self.get_full_path(".cache");
        let mut result = vec![];
        let paths = PathProvider::get_all_file_paths(dest.as_path(), false);
        println!("dest: {:?}", dest);
        let mut total = 0u64;

        let now = SystemTime::now();
        let one_day = Duration::from_secs(86400); // 86400 seconds in a day

        for path_string in paths {
            let path = Path::new(&path_string);
            let metadata = path.metadata()?;
            let modified = metadata.modified()?;

            if now.duration_since(modified)? >= one_day {
                println!("{} TO DELETE {}", path_string, metadata.len());
                
                result.push((path.to_string_lossy().into_owned(), metadata.len()));
                total += metadata.len();
            } else {
                println!("{} TOO YOUNG {}", path_string, metadata.len()); 
            }
            PathProvider::move_to_trash(path)?;   
        }
        println!("Total clean: {} ({})", result.len(), human_bytes(total as f64));

        Ok(result)
    }

}





#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{self, Request, StatusCode, header},
    };
    use http_body_util::BodyExt;
    // for `collect`
    use serde_json::{json, Value};
    use tower::ServiceExt; // for `call`, `oneshot`, and `ready`



}