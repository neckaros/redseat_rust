use std::{env, path::PathBuf, sync::OnceLock};
use query_external_ip::Consensus;
use tokio::{fs::{create_dir_all, metadata, read_to_string, File}, io::AsyncWriteExt, sync::Mutex};
use serde::{Deserialize, Serialize};
use nanoid::nanoid;
use clap::Parser;
use crate::{error::{Error, RsResult}, tools::log::{log_info, LogServiceType}, RegisterInfo, Result};


static CONFIG: OnceLock<Mutex<ServerConfig>> = OnceLock::new();

const ENV_SERVERID: &str = "REDSEAT_SERVERID";
const ENV_HOME: &str = "REDSEAT_HOME";
const ENV_PORT: &str = "REDSEAT_PORT";
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerConfig {
    #[serde(default = "default_serverid")]
    pub id: String,
    #[serde(default = "default_home")]
    pub redseat_home: String,
    pub port: Option<u16>,
    pub local: Option<String>,
    pub domain: Option<String>,
    pub duck_dns: Option<String>,
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Name of the person to greet
    #[arg(short, long)]
    serverid: Option<String>,
}

pub async fn initialize_config() {
    let local_path = get_server_local_path().await.expect("Unable to create local library path");
    log_info(LogServiceType::Register, format!("LocalPath: {:?}", local_path));
    let config = get_config_with_overrides().await.unwrap();
    let _ = CONFIG.set(Mutex::new(config));
}

pub async fn get_server_local_path() -> Result<PathBuf> {
    let Some(mut dir_path) = dirs::config_local_dir() else { return Err(Error::ServerUnableToAccessServerLocalFolder); };
    dir_path.push("redseat");
    

    let Ok(_) = create_dir_all(&dir_path).await else { return Err(Error::ServerUnableToAccessServerLocalFolder); };

    return Ok(dir_path);
}

pub async fn get_server_port() -> u16 {
    let config_port = get_config().await.port;
    env::var(ENV_PORT).ok().and_then(|p| p.parse::<u16>().ok()).or_else(|| config_port).unwrap_or(8080)
}

fn default_serverid() -> String {
    let new_id = nanoid!();
    if let Some(id) = get_config_override_serverid() {
        return id;
    } else {
        return new_id;
    } 
} 

fn get_config_override_serverid() -> Option<String> {
    if let Ok(val) =env::var(ENV_SERVERID) {
        return Some(val);
    } else {
        //let args = Args::parse();
        //return args.serverid;
        return None;
    } 
}

pub async fn get_server_id() -> String {
    get_config().await.id
}

fn default_home() -> String {
    let new_id = "www.redseat.cloud".to_owned();
    if let Some(id) = get_config_override_home() {
        return id;
    } else {
        return new_id;
    } 
} 

fn get_config_override_home() -> Option<String> {
    if let Ok(val) =env::var(ENV_HOME) {
        return Some(val);
    } else {
        //let args = Args::parse();
        //return args.serverid;
        return None;
    } 
}

pub async fn get_home() -> String {
    get_config().await.redseat_home
}
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PublicServerInfos {
    pub url: String,
    pub port: u16,
    pub cert: String,
    pub id: String,
    pub local: Option<String>, 
}

impl PublicServerInfos {
    pub async fn get(public_cert_path: &PathBuf, url: &str) -> RsResult<Self> {
        let cert = read_to_string(public_cert_path).await?;
        let config = get_config().await;
        Ok(PublicServerInfos {
            url: url.to_owned(),
            port: get_server_port().await,
            cert,
            id: get_server_id().await,
            local: config.local,
        })
    }
}


pub async fn get_config() -> ServerConfig {
    if let Some(config) = CONFIG.get() {
        let guard = config.lock().await;
        let config = guard.clone();
        return config;
    } else {
        let config = get_config_with_overrides().await.unwrap();
        let _ = CONFIG.set(Mutex::new(config));
        return CONFIG.get().unwrap().lock().await.clone();
    }
}

pub async fn get_config_with_overrides() -> Result<ServerConfig> {

    let mut config = get_raw_config().await?;

    if let Some(id) = get_config_override_serverid() {
        config.id = id;
    }

    return Ok(config)
}

pub async fn get_raw_config() -> Result<ServerConfig> {
    
    let mut dir_path: PathBuf = get_server_local_path().await?;
    dir_path.push("config.json");

    if let Ok(data) = read_to_string(dir_path.clone()).await {
        let Ok(config) = serde_json::from_str::<ServerConfig>(&data) else { return Err(Error::ServerMalformatedConfigFile); };
        return Ok(config)
    } else {

        let new_config: ServerConfig = serde_json::from_str(r#"{}"#).unwrap();
        let new_config_string = serde_json::to_string(&new_config).unwrap();
        
        let Ok(mut file) = File::create(dir_path).await else { return Err(Error::ServerNoServerId); };
        if file.write_all(new_config_string.as_bytes()).await.is_err() {
            return Err(Error::ServerNoServerId);
        }
        return Ok(new_config)
    } 
}


pub async fn update_config(config: ServerConfig) -> Result<()> {
    let mut dir_path: PathBuf = get_server_local_path().await?;
    dir_path.push("config.json");
    let new_config_string = serde_json::to_string(&config).unwrap();
    let Ok(mut file) = File::create(dir_path).await else { return Err(Error::ServerUnableToAccessServerLocalFolder); };
    if file.write_all(new_config_string.as_bytes()).await.is_err() {
        return Err(Error::ServerNoServerId);
    } else {
        let mut guard = CONFIG.get().unwrap().lock().await;
        *guard = config;
        return Ok(())
    }
}


pub async fn write_server_file(name: &str, data: &[u8]) -> Result<()> {
    let mut dir_path: PathBuf = get_server_local_path().await?;
    dir_path.push(name);
    let Ok(mut file) = File::create(dir_path).await else { return Err(Error::ServerUnableToAccessServerLocalFolder); };
    if file.write_all(&data).await.is_err() {
        return Err(Error::ServerNoServerId);
    } else {
        return Ok(())
    }
}
pub async fn get_server_file_path(name: &str) -> Result<PathBuf> {
    let mut dir_path: PathBuf = get_server_local_path().await?;
    dir_path.push(name);
    return Ok(dir_path);
}

pub async fn get_server_temp_file_path() -> Result<PathBuf> {
    get_server_file_path_array(vec![".cache", &nanoid!()]).await
}


pub async fn get_server_file_path_array(mut names: Vec<&str>) -> Result<PathBuf> {
    let mut dir_path: PathBuf = get_server_local_path().await?;
    if let Some(last) = names.pop() {
        for name in names {
            dir_path.push(name);
        }
        create_dir_all(&dir_path).await?;
        dir_path.push(last);
    }
    return Ok(dir_path);
}
pub async fn get_server_folder_path_array(names: Vec<&str>) -> Result<PathBuf> {
    let mut dir_path: PathBuf = get_server_local_path().await?;
    for name in names {
        dir_path.push(name);
    }
    create_dir_all(&dir_path).await?;
    return Ok(dir_path);
}


pub async fn has_server_file(name: &str) -> bool {
    if let Ok(path) = get_server_file_path(name).await {
        match metadata(path).await {
            Ok(_) => true,
            Err(_) => false,
        }
    } else {
        return false
    }
    
}

pub async fn get_server_file_string(name: &str) -> Result<Option<String>> {
    let mut dir_path: PathBuf = get_server_local_path().await?;
    dir_path.push(name);
    match read_to_string(dir_path).await {
        Ok(data) => return Ok(Some(data)),
        Err(e) => match e.kind() {
            std::io::ErrorKind::NotFound =>  {
                return Ok(None);
            },
            _ => {
                return Err(Error::ServerFileNotFound);
            },
        }
    };

}


pub async fn update_ip() -> Result<Option<(String, String)>> {
    log_info(LogServiceType::Register, "Checking public IPs".to_string());
    let config = get_config().await;

    let Some(domain) = config.domain else {
        log_info(LogServiceType::Register, format!("No Domain"));

        return Ok(None);
    };

    if let Some(duck_dns) = config.duck_dns {
        log_info(LogServiceType::Register, "Updating public ip for duckdns".to_string());
        let ips = Consensus::get().await.or_else(|_| Err(Error::Error("Unable to get external IPs".to_string())))?;
    
        let ipv4 = {
            if let Some(ip) = ips.v4() {
                ip.to_string()
            } else {
                "".to_string()
            }
        };
        let ipv6 = {
            if let Some(ip) = ips.v6() {
                ip.to_string()
            } else {
                "".to_string()
            }
        };
        log_info(LogServiceType::Register, format!("Updating ips: {} {}", ipv4, ipv6));

        let duck_url = format!("https://www.duckdns.org/update?domains={}&token={}&ip={}&ipv6={}&verbose=true", domain.replace(".duckdns.org", ""), duck_dns, ipv4, ipv6);

        let _ = reqwest::get(duck_url)
            .await.map_err(|_| Error::Error("Unable to update duckdns".to_string()))?
            .text()
            .await.map_err(|_| Error::Error("Unable to read duckdns response".to_string()))?;
        return Ok(Some((ipv4, ipv6)));
    }
    Ok(None)

}
