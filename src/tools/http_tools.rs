use std::path::Path;

use mime_guess::MimeGuess;
use nanoid::nanoid;
use reqwest::Client;
use serde::Deserialize;
use tokio::{fs::{self, File}, io::AsyncWriteExt};

use crate::{error::RsResult, tools::file_tools::get_extension_from_mime};

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


#[derive(Deserialize, Debug)]
pub struct GithubRelease {
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize, Debug)]
pub struct GithubAsset {
    name: String,
    browser_download_url: String,
}

pub async fn download_latest_wasm(repo_url: &str, download_dir: &str, filename: Option<&str>) -> RsResult<String> {
    // Manual parsing for GitHub repo URL (e.g., https://github.com/owner/repo)
    // Assumes standard format; trim trailing / or .git
    let url_without_scheme = repo_url
        .strip_prefix("https://")
        .or_else(|| repo_url.strip_prefix("http://"))
        .ok_or("Invalid GitHub URL: must start with http(s)://")?;
    
    if !url_without_scheme.starts_with("github.com/") {
        return Err("Invalid GitHub URL: must be github.com/owner/repo".into());
    }

    let path = &url_without_scheme["github.com/".len()..];
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() < 2 {
        return Err("Invalid GitHub repo URL: must be like https://github.com/owner/repo".into());
    }
    let owner = segments[0];
    let repo = segments[1].trim_end_matches('/')
        .trim_end_matches(".git");

    // Step 1: Fetch latest release info
    let client = Client::builder()
        .user_agent("RedseatRustApp/1.0")  // Required by GitHub API
        .build()?;
    let api_url = format!("https://api.github.com/repos/{}/{}/releases/latest", owner, repo);
    let release: GithubRelease = client
        .get(&api_url)
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .await?
        .json()
        .await?;

    // Step 2: Find first .wasm asset
    let wasm_asset = release.assets.iter()
        .find(|asset| asset.name.ends_with(".wasm"))
        .ok_or("No .wasm asset found in the latest release")?;

    // Step 3: Download the asset
    let wasm_response = client
        .get(&wasm_asset.browser_download_url)
        .send()
        .await?;
    let wasm_bytes = wasm_response.bytes().await?;

    // Step 4: Ensure download directory exists
    let dir_path = Path::new(download_dir);
    if !dir_path.exists() {
        fs::create_dir_all(dir_path).await?;
    }

    // Step 5: Save to file (use asset name)
    let file_path = dir_path.join(&filename.unwrap_or(&wasm_asset.name));
    let mut file = File::create(&file_path).await?;
    file.write_all(&wasm_bytes).await?;

    println!("Downloaded WASM to {}", file_path.display());
    Ok(file_path.to_string_lossy().to_string())
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_content() {
        assert!(parse_content_disposition("attachment; filename=\"test.avif\"") == Some("test.avif".to_string()));
    }

    
}
