use youtube_dl::download_yt_dlp;



#[derive(Debug, Clone)]
pub struct YydlContext {
    update_checked: bool,
}

impl YydlContext {
    pub async fn new() {
        let yt_dlp_path = download_yt_dlp(".").await.unwrap();
    }
}


#[cfg(test)]
mod tests {
    use crate::domain::library::LibraryRole;

    use super::*;

    #[tokio::test]
    async fn test_role() {
        let ctx = YydlContext::new().await;

    }
}