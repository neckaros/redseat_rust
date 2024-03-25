use std::path::Path;

use youtube_dl::{download_yt_dlp, YoutubeDl};

use crate::{error::RsResult, tools::log::log_info};

const FILE_NAME: &str = if cfg!(target_os = "windows") {
    "yt-dlp.exe"
} else {
    "yt-dlp"
};


#[derive(Debug, Clone, Default)]
pub struct YydlContext {
    update_checked: bool,
}

impl YydlContext {
    pub async fn new() -> RsResult<Self> {
        if !Self::has_binary() {
            log_info(crate::tools::log::LogServiceType::Other, "Downloading YT-DLP".to_owned());
            let yt_dlp_path = download_yt_dlp(".").await?;
            
            log_info(crate::tools::log::LogServiceType::Other, format!("downloaded YT-DLP at {:?}", yt_dlp_path));
        }
        Ok(YydlContext { update_checked: false})
    }

    pub async fn update_binary() -> RsResult<()> {
        log_info(crate::tools::log::LogServiceType::Other, "Downloading YT-DLP".to_owned());
        let yt_dlp_path = download_yt_dlp(".").await?;
        
        log_info(crate::tools::log::LogServiceType::Other, format!("downloaded YT-DLP at {:?}", yt_dlp_path));
        Ok(())
    }


    pub fn has_binary() -> bool {
        Path::new(FILE_NAME).exists()
    }

    pub async fn url(&self, url: &str) -> RsResult<()> {
            let output = YoutubeDl::new(url)
            .socket_timeout("15")
            .run_async()
            .await?;
        let title = output.into_single_video().unwrap().title;
        println!("Video title: {:?}", title);
        Ok(())
    }
}


#[cfg(test)]
mod tests {
    use crate::domain::library::LibraryRole;

    use super::*;

    #[tokio::test]
    async fn test_role() {
        let ctx = YydlContext::new().await.unwrap();
        ctx.url("https://twitter.com/LouisFrenchyy/status/1772286083398594605").await.unwrap();

    }
}