use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub mod auth;
pub mod video_tools;
pub mod image_tools;
pub mod log;
pub mod array_tools;
pub mod serialization;
pub mod prediction;
pub mod file_tools;
pub mod http_tools;
pub mod scheduler;


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