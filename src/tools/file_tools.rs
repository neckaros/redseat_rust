use std::{env, path::PathBuf};

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