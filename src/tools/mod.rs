use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub mod array_tools;
pub mod auth;
pub mod convert;
pub mod encryption;
pub mod file_tools;
pub mod http_tools;
pub mod image_tools;
pub mod log;
pub mod prediction;
pub mod recognition;
pub mod scheduler;
pub mod serialization;
pub mod video_tools;

pub mod text_tools;

pub mod serialization_tools;

pub mod clock;
pub mod compression;
pub mod download_external_libs;
pub mod test_sample;

pub fn get_time() -> Duration {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn theic() {
        assert_eq!(4284, 4284);
    }
}
