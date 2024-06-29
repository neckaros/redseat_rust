use std::path::Path;
use serde::Deserialize;
use tokio::fs::File;



#[derive(Deserialize, Debug)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize, Debug)]
struct GithubAsset {
    browser_download_url: String,
    name: String,
}

struct NewestRelease {
    url: String,
    tag: String,
}

#[derive(Deserialize, Debug)]
struct ExternalLibDownloader {
    pub name: String
}

impl ExternalLibDownloader {
    pub fn filename(&self) -> String {
        if cfg!(target_os = "windows") {
            format!("{}.exe", self.name)
        } else {
            self.name.to_string()
        }
    }

    pub fn release_name(&self) -> String {
        if cfg!(target_os = "windows") {
            format!("{}.exe", self.name)
        } else if cfg!(target_os = "linux") {
            format!("{}-linux", self.name)
        } else if cfg!(target_os = "macos") {
            format!("{}-macos", self.name)
        } else {
            self.name.to_string()
        }
    }

    pub fn has_binary(&self) -> bool {
        Path::new(&self.filename()).exists()
    }



    #[cfg(target_os = "windows")]
    async fn create_file(path: impl AsRef<Path>) -> tokio::io::Result<File> {
        

        File::create(&path).await
    }

    #[cfg(not(target_os = "windows"))]
    async fn create_file(path: impl AsRef<Path>) -> tokio::io::Result<File> {
        use tokio::fs::OpenOptions;

        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o744)
            .open(&path)
            .await
    }
}

