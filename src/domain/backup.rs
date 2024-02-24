use serde::{Deserialize, Serialize};
use serde_json::Value;


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")] 
pub struct Backup {
    pub id: String,
	pub source: String,
    pub credentials: String,
    pub library: String,
    pub path: String,
    pub schedule: Option<String>,
    pub filter: Option<Value>,
    pub last: Option<u64>,
    pub password: Option<String>,
    pub size: u64,
}



#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")] 
pub struct BackupFile {
    pub backup: String,
	pub library: String,
    pub file: String,
    pub id: String,
    pub path: String,
    pub hash: String,
    pub sourcehash: String,
    pub size: u64,
    pub date: u64,
    pub iv: Option<String>,
    pub info_size: Option<u64>,
    pub error: Option<String>,
}