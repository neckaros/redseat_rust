use std::{fs::File, io::BufReader, path::{Path, PathBuf}};
use flate2::read::GzDecoder;
use xz2::read::XzDecoder;
use tar::Archive;

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

