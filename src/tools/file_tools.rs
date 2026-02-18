use std::{env, path::{PathBuf, Path}};
use std::fs;
use std::io;
use tokio::task;
use zip::ZipArchive;
use tokio::io::AsyncRead;

use mime_guess::get_mime_extensions_str;
use nanoid::nanoid;
use crate::{domain::media::FileType, error::RsResult};

pub fn filename_from_path(path: &str) -> Option<String> {
    let escaped = path.replace('\\', "/");
    escaped.split('/').last().and_then(|last| {
        last.split('?').next().map(|c| c.to_owned())
    })
}

pub fn remove_extension(filename: &str) -> &str {
    match filename.rfind('.') {
        Some(index) => &filename[..index],
        None => filename,
    }
}


pub fn get_mime_from_filename(path: &str) -> Option<String> {
    let mime = mime_guess::from_path(&path);
    if let Some(mime) = mime.first() {
        Some(mime.to_string())
    } else if path.ends_with(".heic") {
        Some("image/heic".to_string())
    } else {
        None
    }
}

pub fn get_extension_from_mime(mime: &str) -> String {

    if mime == "image/heic" {
        return "heic".to_string();
    }
    let suffix = get_mime_extensions_str(mime).and_then(|f| f.first()).unwrap_or(&"bin").to_string();

    match suffix.as_str() {
        "jpe" => "jpeg",
        _ => &suffix
    }.to_string()
}

pub fn file_type_from_mime(mime: &str) -> FileType {
    if mime.starts_with("image") {
        FileType::Photo
    } else if mime.starts_with("video") {
        FileType::Video
    } else if mime == "application/zip" {
        FileType::Album
    } else if mime == "application/vnd.comicbook+cbz" || mime == "application/x-cbr" {
        FileType::Album
    } else if mime == "application/epub+zip" {
        FileType::Book
    } else {
        FileType::Other
    }
}

pub fn executable_dir() -> RsResult<PathBuf> {
    let exe_path = env::current_exe()?;
    
    // Get the directory containing the executable
    let exe_dir = exe_path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::Other, "Failed to get parent directory of executable")
    })?;

    return Ok(exe_dir.to_path_buf())
}



// Add this function to your module (e.g., impl or lib.rs)
pub async fn extract_zip(
    reader: &mut (dyn AsyncRead + Unpin + Send),
    target_dir: &Path,
) -> RsResult<()> {
    // Create temp file path
    let temp_dir = std::env::temp_dir();
    let temp_filename = format!("plugin_upload_{}.zip", nanoid::nanoid!());
    let temp_path = temp_dir.join(temp_filename);

    // Stream the ZIP to temp file (async, bounded memory)
    {
        let mut temp_file = tokio::fs::File::create(&temp_path).await?;
        tokio::io::copy(reader, &mut temp_file).await?;
    }

    // Clone for async cleanup
    let temp_path_for_cleanup = temp_path.clone();
    let target_dir = target_dir.to_path_buf();

    // Spawn blocking task for sync extraction, using io::Error for consistency
    let extract_result = task::spawn_blocking(move || -> Result<(), io::Error> {
        let file = fs::File::open(&temp_path)?;
        let mut archive = ZipArchive::new(file)?;

        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            let entry_name = entry.name().to_string();

            // Security: Block path traversal or absolute paths
            if entry_name.contains("..") || Path::new(&entry_name).is_absolute() {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "Path traversal detected in ZIP entry"));
            }

            let mut outpath = target_dir.clone();
            outpath.push(&entry_name);

            if entry_name.ends_with('/') || (entry.unix_mode().map_or(false, |mode| (mode & 0o40000) != 0)) {
                // Directory
                fs::create_dir_all(&outpath)?;
            } else {
                // File
                if let Some(parent) = outpath.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut outfile = fs::File::create(&outpath)?;
                io::copy(&mut entry, &mut outfile)?;  // This fully consumes the entry and automatically verifies CRC checksum
            }
            // No explicit finish() needed: io::copy handles full consumption and CRC verification via the Read impl
        }

        Ok(())
    })
    .await
    .map_err(|join_err| io::Error::new(io::ErrorKind::Other, format!("Spawn blocking failed: {}", join_err)))?;


    // Cleanup temp file (async)
    if let Err(e) = tokio::fs::remove_file(temp_path_for_cleanup).await {
        eprintln!("Failed to remove temp file: {}", e);
    }

    Ok(extract_result?)
}