use mime_guess::get_mime_extensions_str;
use nanoid::nanoid;
use crate::domain::media::FileType;

pub fn filename_from_path(path: &str) -> Option<String> {
    let escaped = path.replace('\\', "/");
    escaped.split('/').last().and_then(|last| {
        last.split('?').next().map(|c| c.to_owned())
    })
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
    } else if mime == "application/vnd.comicbook+cbz" {
        FileType::Album
    } else {
        FileType::Other
    }
}