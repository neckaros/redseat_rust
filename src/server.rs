use std::{cell::{OnceCell, RefCell}, env, path::PathBuf, sync::OnceLock, time::Duration};
use axum::serve::Serve;
use query_external_ip::Consensus;
use reqwest::Client;
use tokio::{fs::{create_dir_all, metadata, read_to_string, File}, io::AsyncWriteExt, sync::Mutex};
use serde::{Deserialize, Serialize};
use nanoid::nanoid;
use clap::Parser;
use tracing_subscriber::fmt::format;
use crate::{error::{Error, RsResult}, model::{users::ConnectedUser, ModelController}, tools::{image_tools::has_image_magick, log::{log_info, LogServiceType}}, RegisterInfo, Result};


static CONFIG: OnceLock<Mutex<ServerConfig>> = OnceLock::new();


const ENV_SERVERID: &str = "REDSEAT_SERVERID";
const ENV_HOME: &str = "REDSEAT_HOME";
const ENV_PORT: &str = "REDSEAT_PORT";
const ENV_EXP_PORT: &str = "REDSEAT_EXP_PORT";
const ENV_DIR: &str = "REDSEAT_DIR";
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerConfig {
    pub id: Option<String>,
    #[serde(default = "default_home")]
    pub redseat_home: String,
    pub port: Option<u16>,
    pub exp_port: Option<u16>,
    pub local: Option<String>,
    pub token: Option<String>,
    #[serde(default = "default_false")]
    pub noIM: bool
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Name of the person to greet
    #[arg(short, long)]
    serverid: Option<String>,


    #[arg(short = 'k', long)]
    docker: bool,

    #[arg(short = 'm', long)]
    noIM: bool,
    
    #[arg(short, long)]
    dir: Option<String>,
}

pub async fn initialize_config() -> ServerConfig {
    let local_path = get_server_local_path().await.expect("Unable to create local library path");
    log_info(LogServiceType::Register, format!("LocalPath: {:?}", local_path));
    let config = get_config_with_overrides().await.unwrap();
    let _ = CONFIG.set(Mutex::new(config.clone()));
    return config
}

pub async fn get_server_local_path() -> Result<PathBuf> {
    let args = Args::parse();
    
    let dir_path = if let Some(argdir) = args.dir {
        PathBuf::from(&argdir)
    } else if let Ok(val) =env::var(ENV_DIR) {
        PathBuf::from(&val)
    } else if args.docker {
        PathBuf::from("/config")
    } else {
        let Some(mut dir_path) = dirs::config_local_dir() else { return Err(Error::ServerUnableToAccessServerLocalFolder); };
        dir_path.push("redseat");
        dir_path    
    };

    

    let Ok(_) = create_dir_all(&dir_path).await else { return Err(Error::ServerUnableToAccessServerLocalFolder); };

    return Ok(dir_path);
}

pub async fn get_server_port() -> u16 {
    let config_port = get_config().await.port;
    env::var(ENV_PORT).ok().and_then(|p| p.parse::<u16>().ok()).or_else(|| config_port).unwrap_or(8080)
}
pub async fn get_server_exposed_port() -> u16 {
    let config_port = get_config().await.exp_port;
    env::var(ENV_EXP_PORT).ok().and_then(|p| p.parse::<u16>().ok()).or_else(|| config_port).unwrap_or(8080)
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

pub async fn get_server_id() -> Option<String> {
    get_config().await.id
}

fn default_false() -> bool {
    false
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
    pub port: u16,
    pub cert: Option<String>,
    pub id: Option<String>,
    pub local: Option<String>, 
}

impl PublicServerInfos {
    pub async fn get(public_cert_path: &PathBuf, _url: &str) -> RsResult<Self> {
        let cert = read_to_string(public_cert_path).await?;
        let config = get_config().await;
        Ok(PublicServerInfos {
            port: get_server_port().await,
            cert: Some(cert),
            id: get_server_id().await,
            local: config.local,
        })
    }

    pub async fn current() -> RsResult<Self> {
	    let public_cert_path = get_server_file_path("cert_chain.pem").await?;
        let cert = read_to_string(public_cert_path).await.ok();
        let config = get_config().await;
        
        Ok(PublicServerInfos {
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

pub async fn check_unregistered() -> Result<()> {
    let id = get_config().await.id;
    if id.is_some() {
        Err(crate::Error::ServerAlreadyRegistered)
    } else {
        Ok(())
    }
}

pub async fn get_config_with_overrides() -> Result<ServerConfig> {
    let args = Args::parse();
    let mut config = get_raw_config().await?;

    if let Some(id) = get_config_override_serverid() {
        config.id = Some(id);
    }
    if args.noIM {
        config.noIM = true
    } else if !has_image_magick() {
        config.noIM = true
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
    file.write_all(new_config_string.as_bytes()).await?;
    
    let mut guard = CONFIG.get().unwrap().lock().await;
    *guard = config;
    return Ok(())
}

pub async fn get_install_local_url() -> Result<String> {
	Ok(format!("https://127.0.0.1:{}/infos/install", get_server_port().await))
}


pub async fn get_own_url() -> Result<String> {
	let config = get_config().await;

	let mut params = vec![];
	if let Some(port) = config.port {
		params.push(format!("port={}", port));
	}
	if let Some(local) = config.local {
		params.push(format!("local={}", local));
	}
	
	Ok(format!("https://{}/install?{}", config.redseat_home, params.join("&")))
}

pub async fn get_web_url() -> Result<String> {
	let config = get_config().await;
    if let Some(id) = config.id {
	    Ok(format!("https://{}/servers/{}", config.redseat_home, id))
    } else {
	    Err(crate::Error::Error("Server not registered".to_owned()))
    }
}

pub async fn get_install_url() -> Result<String> {
	let config = get_config().await;

	let mut params = vec![];
	if let Some(port) = config.port {
		params.push(format!("port={}", port));
	}
	if let Some(local) = config.local {
		params.push(format!("local={}", local));
	}
   
	
	Ok(format!("https://{}/install?{}", config.redseat_home, params.join("&")))
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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerIpInfo {
    pub ipv4: Option<String>,
    pub ipv6: Option<String>,
}

pub async fn get_ipv4() -> Result<String> {
    let client = Client::builder()
    .timeout(Duration::from_secs(1))
    .build()
    .unwrap();

    let ip =client.get("https://v4.ident.me/").send().await;
    if let Ok(ip) = ip {
        if let Ok(ip) = ip.text().await {
            return Ok(ip);
        }
    }
    let ip = client.get("https://api.ipify.org/").send().await;
    if let Ok(ip) = ip {
        if let Ok(ip) = ip.text().await {
            return Ok(ip);
        }
    }
    Err(Error::Error("Unable to get IPV4".to_string()))
}

pub async fn get_ipv6() -> Result<String> {
    let client = Client::builder()
    .timeout(Duration::from_secs(1))
    .build()
    .unwrap();

    let ip = client.get("https://v6.ident.me/").send().await;
    if let Ok(ip) = ip {
        if let Ok(ip) = ip.text().await {
            return Ok(ip);
        }
    }
    let ip = client.get("https://api64.ipify.org/").send().await;
    if let Ok(ip) = ip {
        if let Ok(ip) = ip.text().await {
            return Ok(ip);
        }
    }
    Err(Error::Error("Unable to get IPV4".to_string()))
}


pub async fn update_ip() -> Result<Option<String>> {
    log_info(LogServiceType::Register, "Checking public IPs".to_string());
    let config = get_config().await;
    let id = config.id.ok_or(crate::Error::ServerNoServerId)?;
    let token = config.token.ok_or(crate::Error::ServerNotYetRegistered)?;

    let ipv4 = get_ipv4().await?;
    //let ipv6 = get_ipv6().await?;

    

    log_info(LogServiceType::Register, format!("Updating ip: {}", ipv4));


    let client = reqwest::Client::new();
        
    let request = ServerIpInfo {
        ipv4: Some(ipv4.clone()),
        ipv6: None,
    };

    log_info(LogServiceType::Register, format!("Calling: https://{}/servers/{}/register", config.redseat_home, id));
    log_info(LogServiceType::Register, format!("With content: {:?}", request));
    let result = client.patch(format!("https://{}/servers/{}/register", config.redseat_home, id))
    .header("Authorization", format!("Token {}", token))
        .json(&request)
        .send()
        .await?;

    log_info(LogServiceType::Register, format!("Result: {:?}", result.text().await?));


    
    Ok(Some(ipv4))

}
