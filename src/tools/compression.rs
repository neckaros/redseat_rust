use std::{fs::File, io::BufReader, path::{Path, PathBuf}};
use flate2::read::GzDecoder;
use futures::AsyncReadExt;
use xz2::read::XzDecoder;
use tar::Archive;
use async_zip::tokio::read::fs::ZipFileReader;
use tokio::io::AsyncWriteExt;

use crate::error::{RsError, RsResult};

pub async fn unpack_tar_gz(path: impl AsRef<Path>, dest: PathBuf) -> Result<(), std::io::Error> {
    let file = File::open(path)?;
    tokio::task::spawn_blocking(move || {
    
    let buf_reader = BufReader::new(file);
    let tar = GzDecoder::new(buf_reader);
    let mut archive = Archive::new(tar);
    archive.unpack(dest)
    }).await?
}

pub async fn unpack_tar_xz(path: impl AsRef<Path>, dest: PathBuf) -> Result<(), std::io::Error> {
    let file = File::open(path)?;
    tokio::task::spawn_blocking(move || {
    
    let buf_reader = BufReader::new(file);
    let tar = XzDecoder::new(buf_reader);
    let mut archive = Archive::new(tar);
    archive.unpack(dest)
    }).await?
}

pub async fn unpack_7z(path: PathBuf, dest: PathBuf) -> RsResult<()> {
    tokio::task::spawn_blocking(move || {
        sevenz_rust::decompress_file(&path, &dest).map_err(|_| RsError::Error("unable to uncompress 7Zip file".to_string()))
    }).await?
}


pub async fn unpack_zip(path: PathBuf, dest: PathBuf) -> RsResult<()> {
    let mut reader = ZipFileReader::new(&path)
        .await
        .map_err(|e| RsError::Error(format!("unable to open zip file ({:?}): {:?}", path, e)))?;
    
    for index in 0..reader.file().entries().len() {
        let entry = reader.file().entries().get(index).unwrap();
        let entry_name = entry.filename().as_str()
            .map_err(|_| RsError::Error("invalid filename encoding".to_string()))?;
        
        let entry_path = dest.join(entry_name);
        
        // Create parent directories if needed
        if let Some(parent) = entry_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|_| RsError::Error("unable to create directories".to_string()))?;
        }
        
        // Skip directories
        if entry_name.ends_with('/') {
            continue;
        }
        
        // Extract file
        let mut entry_reader = reader.reader_with_entry(index)
            .await
            .map_err(|_| RsError::Error("unable to read zip entry".to_string()))?;
        
        let mut output_file = tokio::fs::File::create(&entry_path)
            .await
            .map_err(|_| RsError::Error("unable to create output file".to_string()))?;
        
        // Manually copy data since entry_reader does not implement AsyncRead
        let mut buffer = [0u8; 8192];
        loop {
            let read_bytes = entry_reader.read(&mut buffer).await
                .map_err(|_| RsError::Error("unable to read from zip entry".to_string()))?;
            if read_bytes == 0 {
                break;
            }
            output_file.write_all(&buffer[..read_bytes]).await
                .map_err(|_| RsError::Error("unable to write to output file".to_string()))?;
        }
    }
    
    Ok(())
}