
use serde::{Deserialize, Serialize};
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AuthMessage {
    pub token: Option<String>,
    pub sharetoken: Option<String>,
}
