use crate::domain::media::FileType;

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

pub fn file_type_from_mime(mime: &str) -> FileType {
    if mime.starts_with("image") {
        FileType::Photo
    } else if mime.starts_with("video") {
        FileType::Video
    } else {
        FileType::Other
    }
}