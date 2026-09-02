use async_zip::base::read1::{seek::ZipArchiveReader, ZipOptions};
use flate2::read::GzDecoder;
use std::{
    fs::File,
    io::BufReader,
    path::{Component, Path, PathBuf},
};
use tar::Archive;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use xz2::read::XzDecoder;

use crate::error::{RsError, RsResult};

pub async fn unpack_tar_gz(path: impl AsRef<Path>, dest: PathBuf) -> Result<(), std::io::Error> {
    let file = File::open(path)?;
    tokio::task::spawn_blocking(move || {
        let buf_reader = BufReader::new(file);
        let tar = GzDecoder::new(buf_reader);
        let mut archive = Archive::new(tar);
        archive.unpack(dest)
    })
    .await?
}

pub async fn unpack_tar_xz(path: impl AsRef<Path>, dest: PathBuf) -> Result<(), std::io::Error> {
    let file = File::open(path)?;
    tokio::task::spawn_blocking(move || {
        let buf_reader = BufReader::new(file);
        let tar = XzDecoder::new(buf_reader);
        let mut archive = Archive::new(tar);
        archive.unpack(dest)
    })
    .await?
}

pub async fn unpack_7z(path: PathBuf, dest: PathBuf) -> RsResult<()> {
    tokio::task::spawn_blocking(move || {
        sevenz_rust::decompress_file(&path, &dest)
            .map_err(|_| RsError::Error("unable to uncompress 7Zip file".to_string()))
    })
    .await?
}

pub async fn unpack_zip(path: PathBuf, dest: PathBuf) -> RsResult<()> {
    let file = tokio::fs::File::open(&path).await?;
    let reader = tokio::io::BufReader::with_capacity(64 * 1024, file).compat();
    let mut archive = ZipArchiveReader::open_with_options(reader, ZipOptions::untrusted())
        .await
        .map_err(|e| RsError::Error(format!("unable to open zip file ({:?}): {:?}", path, e)))?;

    for index in 0..archive.cdrs().len() {
        let entry_name =
            String::from_utf8_lossy(archive.cdrs()[index].insecure_file_name.as_bytes())
                .into_owned();
        let is_directory = entry_name.ends_with('/') || entry_name.ends_with('\\');
        let normalized = entry_name.replace('\\', "/");
        let mut relative_path = PathBuf::new();
        for component in Path::new(&normalized).components() {
            match component {
                Component::Normal(component) => relative_path.push(component),
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(RsError::Error(format!(
                        "unsafe path in ZIP archive: {entry_name}"
                    )))
                }
            }
        }
        if relative_path.as_os_str().is_empty() {
            continue;
        }
        let entry_path = dest.join(relative_path);

        // Create parent directories if needed
        if let Some(parent) = entry_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|_| RsError::Error("unable to create directories".to_string()))?;
        }

        // Materialize directory entries.
        if is_directory {
            tokio::fs::create_dir_all(&entry_path)
                .await
                .map_err(|_| RsError::Error("unable to create directory".to_string()))?;
            continue;
        }

        // Extract file
        let mut entry_reader = archive
            .file(index)
            .await
            .map_err(|_| RsError::Error("unable to read zip entry".to_string()))?;

        let output_file = tokio::fs::File::create(&entry_path)
            .await
            .map_err(|_| RsError::Error("unable to create output file".to_string()))?;
        let mut output_file = output_file.compat_write();
        futures::io::copy(&mut entry_reader, &mut output_file)
            .await
            .map_err(|error| RsError::Error(format!("unable to extract zip entry: {error}")))?;
        futures::AsyncWriteExt::close(&mut output_file)
            .await
            .map_err(|error| RsError::Error(format!("unable to close extracted file: {error}")))?;
    }

    Ok(())
}
