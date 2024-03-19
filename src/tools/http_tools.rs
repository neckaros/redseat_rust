use mime_guess::MimeGuess;
use nanoid::nanoid;

use crate::tools::file_tools::get_extension_from_mime;

pub fn extract_header(headers: &http::HeaderMap, name: http::HeaderName) -> Option<&str>{
    headers.get(name).and_then(|l| l.to_str().ok())
}

pub fn guess_filename(url: &str, mime: &Option<String>) -> String {
    let last_path = url.split("/").last().and_then(|p| p.split("?").next());
    if let Some(last_path) = last_path {
        if last_path.split(".").last().and_then(|e| if e.len() < 6 {Some(e)} else {None}).is_some() {
            last_path.to_string()
        } else if let Some(mime) = &mime {
            let ext = get_extension_from_mime(mime);
            format!("{}.{}", last_path, ext)
        } else {
            nanoid!()
        }
    } else if let Some(mime) = &mime {
        let ext = get_extension_from_mime(mime);
        format!("{}.{}", nanoid!(), ext)
    } else {
        nanoid!()
    }
}

pub fn parse_content_disposition(disposition: &str) -> Option<String> {
    let parsed: Vec<_> = disposition.split("filename=").collect();
    if parsed.len() != 2 {
        None
    } else {
        parsed.last().and_then(|p| Some(p.replace("\"", "")))
    }
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_content() {
        assert!(parse_content_disposition("attachment; filename=\"test.avif\"") == Some("test.avif".to_string()));
    }

    
}
