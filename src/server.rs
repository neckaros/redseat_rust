use crate::{
    error::{Error, RsError, RsResult},
    model::{users::ConnectedUser, ModelController},
    plugins::url,
    tools::{
        image_tools::has_image_magick,
        log::{log_error, log_info, LogServiceType},
    },
    RegisterInfo, Result,
};
use axum::serve::Serve;
use clap::Parser;
use nanoid::nanoid;
use query_external_ip::Consensus;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::{
    cell::{OnceCell, RefCell},
    env,
    path::PathBuf,
    sync::OnceLock,
    time::Duration,
};
use tokio::{
    fs::{create_dir_all, metadata, read_to_string, File},
    io::AsyncWriteExt,
    sync::Mutex,
};
use tracing_subscriber::fmt::format;

static CONFIG: OnceLock<Mutex<ServerConfig>> = OnceLock::new();

const ENV_SERVERID: &str = "REDSEAT_SERVERID";
const ENV_HOME: &str = "REDSEAT_HOME";
const ENV_PORT: &str = "REDSEAT_PORT";
const ENV_EXP_PORT: &str = "REDSEAT_EXP_PORT";
const ENV_DIR: &str = "REDSEAT_DIR";
const ENV_DOMAIN: &str = "REDSEAT_DOMAIN";
const ENV_NOCERT: &str = "REDSEAT_NOCERT";
const ENV_SIGNALING_URL: &str = "REDSEAT_SIGNALING_URL";
const ENV_PORT_FORWARDED: &str = "REDSEAT_PORT_FORWARDED";
const ENV_LAN_IPS: &str = "REDSEAT_LAN_IPS";
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerConfig {
    pub id: Option<String>,
    #[serde(default = "default_home")]
    pub redseat_home: String,
    pub domain: Option<String>,
    #[serde(default = "default_false")]
    pub noCert: bool,
    pub port: Option<u16>,
    pub exp_port: Option<u16>,
    pub local: Option<String>,
    pub token: Option<String>,
    /// Optional development/test override for the cloud WebRTC rendezvous endpoint.
    #[serde(default, rename = "signalingUrl", alias = "signaling_url")]
    pub signaling_url: Option<String>,
    #[serde(default = "default_false")]
    pub imagesUseIm: bool,
    /// The port is forwarded manually on the router: report the public IPv4 for direct
    /// HTTPS even when UPnP can't open it.
    #[serde(default, rename = "portForwarded", alias = "port_forwarded")]
    pub port_forwarded: bool,
    /// LAN addresses to report for direct HTTPS instead of the discovered ones (for example
    /// the host's addresses when running in a container).
    #[serde(
        default,
        rename = "lanIps",
        alias = "lan_ips",
        skip_serializing_if = "Option::is_none"
    )]
    pub lan_ips: Option<Vec<String>>,
}

impl ServerConfig {
    pub fn get_signaling_url(&self) -> String {
        env::var(ENV_SIGNALING_URL)
            .ok()
            .or_else(|| self.signaling_url.clone())
            .unwrap_or_else(|| {
                format!(
                    "wss://{}/api/webrtc/signaling",
                    self.redseat_home
                        .trim_end_matches('/')
                        .trim_start_matches("https://")
                        .trim_start_matches("http://")
                )
            })
    }

    pub fn get_server_base_url(&self) -> RsResult<String> {
        return Ok(format!(
            "https://{}-srv.redseat.cloud:{}",
            self.id
                .clone()
                .ok_or(RsError::Error("No id set for this server".to_string()))?,
            self.get_port().to_string()
        ));
    }

    pub fn get_port(&self) -> u16 {
        let config_port = self.port;
        env::var(ENV_PORT)
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .or_else(|| config_port)
            .unwrap_or(8080)
    }

    /// Port clients connect to: the exposed port when it differs from the listening one
    /// (container port mapping), otherwise the listening port.
    pub fn get_exposed_port(&self) -> u16 {
        self.get_explicit_exposed_port()
            .unwrap_or_else(|| self.get_port())
    }
}

#[derive(Parser, Debug, Default)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Force server id
    #[arg(short, long)]
    serverid: Option<String>,

    // Use docker specific settings
    #[arg(short = 'k', long)]
    docker: bool,

    // Use image magick if installed for images conversion
    #[arg(short = 'm', long)]
    imagesUseIm: bool,

    // Don't use certificate creation (if your domain already has ssl via proxy)
    #[arg(short = 'c', long)]
    noCert: Option<bool>,

    /// set domain name (ex redseat.myserver.com)
    #[arg(short = 'u', long)]
    domain: Option<String>,

    // Server local folder
    #[arg(short, long)]
    dir: Option<String>,
}

pub async fn initialize_config() -> ServerConfig {
    let local_path = get_server_local_path()
        .await
        .expect("Unable to create local library path");
    log_info(
        LogServiceType::Register,
        format!("LocalPath: {:?}", local_path),
    );
    let config = get_config_with_overrides().await.unwrap();
    let _ = CONFIG.set(Mutex::new(config.clone()));
    return config;
}

pub async fn get_server_local_path() -> Result<PathBuf> {
    let args = Args::try_parse().unwrap_or_default();

    let dir_path = if let Some(argdir) = args.dir {
        PathBuf::from(&argdir)
    } else if let Ok(val) = env::var(ENV_DIR) {
        PathBuf::from(&val)
    } else if args.docker {
        PathBuf::from("/config")
    } else {
        let Some(mut dir_path) = dirs::config_local_dir() else {
            return Err(Error::ServerUnableToAccessServerLocalFolder);
        };
        dir_path.push("redseat");
        dir_path
    };

    let Ok(_) = create_dir_all(&dir_path).await else {
        return Err(Error::ServerUnableToAccessServerLocalFolder);
    };

    return Ok(dir_path);
}

pub async fn get_server_port() -> u16 {
    let config_port = get_config().await.port;
    env::var(ENV_PORT)
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .or_else(|| config_port)
        .unwrap_or(8080)
}
pub async fn get_server_exposed_port() -> u16 {
    let config_port = get_config().await.exp_port;
    env::var(ENV_EXP_PORT)
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .or_else(|| config_port)
        .unwrap_or(8080)
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
    if let Ok(val) = env::var(ENV_SERVERID) {
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
    if let Ok(val) = env::var(ENV_HOME) {
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
    let args = Args::try_parse().unwrap_or_default();
    let mut config = get_raw_config().await?;

    if let Some(id) = get_config_override_serverid() {
        config.id = Some(id);
    }

    if let Some(domain) = args.domain.or_else(|| env::var(ENV_DOMAIN).ok()) {
        config.domain = Some(domain);
    }
    if let Some(noCert) = args.noCert.or_else(|| {
        if let Ok(val) = env::var(ENV_NOCERT) {
            val.parse::<bool>().ok()
        } else {
            None
        }
    }) {
        config.noCert = noCert;
    }
    if let Some(port_forwarded) = env::var(ENV_PORT_FORWARDED)
        .ok()
        .and_then(|val| val.parse::<bool>().ok())
    {
        config.port_forwarded = port_forwarded;
    }
    if let Ok(lan_ips) = env::var(ENV_LAN_IPS) {
        config.lan_ips = Some(
            lan_ips
                .split(',')
                .map(|ip| ip.trim().to_string())
                .filter(|ip| !ip.is_empty())
                .collect(),
        );
    }

    if args.imagesUseIm {
        if has_image_magick() {
            config.imagesUseIm = true;
        } else {
            config.imagesUseIm = false;
            log_error(
                LogServiceType::Other,
                "Trying to use ImageMagick but not found on computer".to_string(),
            );
        }
    } else {
        config.imagesUseIm = false
    }

    return Ok(config);
}

pub async fn get_raw_config() -> Result<ServerConfig> {
    let mut dir_path: PathBuf = get_server_local_path().await?;
    dir_path.push("config.json");

    if let Ok(data) = read_to_string(dir_path.clone()).await {
        let Ok(config) = serde_json::from_str::<ServerConfig>(&data) else {
            return Err(Error::ServerMalformatedConfigFile);
        };
        return Ok(config);
    } else {
        let new_config: ServerConfig = serde_json::from_str(r#"{}"#).unwrap();
        let new_config_string = serde_json::to_string(&new_config).unwrap();

        let Ok(mut file) = File::create(dir_path).await else {
            return Err(Error::ServerNoServerId);
        };
        if file.write_all(new_config_string.as_bytes()).await.is_err() {
            return Err(Error::ServerNoServerId);
        }
        return Ok(new_config);
    }
}

pub async fn update_config(config: ServerConfig) -> Result<()> {
    let mut dir_path: PathBuf = get_server_local_path().await?;
    dir_path.push("config.json");
    let new_config_string = serde_json::to_string(&config).unwrap();
    let Ok(mut file) = File::create(dir_path).await else {
        return Err(Error::ServerUnableToAccessServerLocalFolder);
    };
    file.write_all(new_config_string.as_bytes()).await?;

    let mut guard = CONFIG.get().unwrap().lock().await;
    *guard = config;
    return Ok(());
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
    if let Some(domain) = config.domain {
        params.push(format!("domain={}", domain));
    }

    if config.noCert {
        params.push(format!("noCert={}", config.noCert.to_string()));
    }

    Ok(format!(
        "https://{}/install?{}",
        config.redseat_home,
        params.join("&")
    ))
}

pub async fn write_server_file(name: &str, data: &[u8]) -> Result<()> {
    let mut dir_path: PathBuf = get_server_local_path().await?;
    dir_path.push(name);
    let Ok(mut file) = File::create(dir_path).await else {
        return Err(Error::ServerUnableToAccessServerLocalFolder);
    };
    if file.write_all(&data).await.is_err() {
        return Err(Error::ServerNoServerId);
    } else {
        return Ok(());
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
        return false;
    }
}

pub async fn get_server_file_string(name: &str) -> Result<Option<String>> {
    let mut dir_path: PathBuf = get_server_local_path().await?;
    dir_path.push(name);
    match read_to_string(dir_path).await {
        Ok(data) => return Ok(Some(data)),
        Err(e) => match e.kind() {
            std::io::ErrorKind::NotFound => {
                return Ok(None);
            }
            _ => {
                return Err(Error::ServerFileNotFound);
            }
        },
    };
}

pub async fn get_ipv4() -> Result<String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(1))
        .build()
        .unwrap();

    let ip = client.get("https://v4.ident.me/").send().await;
    if let Ok(ip) = ip {
        if let Ok(ip) = ip.text().await {
            return Ok(ip);
        }
    }
    let ip = client.get("https://api.ipify.org/").send().await;
    match ip {
        Ok(ip) => match ip.text().await {
            Ok(ip) => return Ok(ip),
            _ => Err(Error::Error("Unable to get IPV4 no text found".to_string())),
        },
        Err(e) => Err(Error::Error(format!("Unable to get IPV4: {:?}", e))),
    }
}

/// How clients reach a server with a custom domain, reported to the cloud (`PATCH
/// /api/servers/<id>`). `domain: null` tells the cloud there is none, so a stale one is cleared.
#[derive(Debug, Serialize, PartialEq)]
pub struct DomainReport {
    pub domain: Option<String>,
    /// Public port of the domain; absent means 443.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}

impl ServerConfig {
    /// Exposed port set explicitly (`REDSEAT_EXP_PORT` or `exp_port`), if any.
    pub fn get_explicit_exposed_port(&self) -> Option<u16> {
        env::var(ENV_EXP_PORT)
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .or(self.exp_port)
    }

    /// The domain endpoint: a port written in the domain (`host:8443`), else the explicit
    /// exposed port, else none (443). Never the listening port: behind a reverse proxy it
    /// isn't what clients connect to.
    pub fn domain_report(&self) -> DomainReport {
        let Some(domain) = self.domain.as_deref() else {
            return DomainReport {
                domain: None,
                port: None,
            };
        };
        let domain = domain
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/');
        let (host, port) = match domain.rsplit_once(':') {
            Some((host, port)) if port.parse::<u16>().is_ok() => (host, port.parse::<u16>().ok()),
            _ => (domain, None),
        };
        DomainReport {
            domain: Some(host.to_string()),
            port: port
                .or_else(|| self.get_explicit_exposed_port())
                .filter(|port| *port != 443),
        }
    }
}

const DOMAIN_REPORT_REFRESH: Duration = Duration::from_secs(24 * 60 * 60);
const DOMAIN_REPORT_FIRST_RETRY: Duration = Duration::from_secs(60);
const DOMAIN_REPORT_MAX_RETRY: Duration = Duration::from_secs(60 * 60);

/// Delay before retrying after `failures` consecutive failed reports: 1 min, doubling up to 1 h.
fn domain_report_retry_delay(failures: u32) -> Duration {
    DOMAIN_REPORT_FIRST_RETRY
        .saturating_mul(1u32 << failures.saturating_sub(1).min(16))
        .min(DOMAIN_REPORT_MAX_RETRY)
}

/// Reports the custom domain in the background. The domain only changes with the config, which
/// is read at startup, so one successful report is enough: failures are retried with backoff,
/// then it's re-sent daily in case the cloud's copy was lost.
pub fn spawn_domain_reporter() {
    tokio::spawn(async {
        let mut failures = 0u32;
        loop {
            let delay = match report_domain().await {
                Ok(report) => {
                    log_info(LogServiceType::Register, describe_domain_report(&report));
                    failures = 0;
                    DOMAIN_REPORT_REFRESH
                }
                Err(error) => {
                    failures = failures.saturating_add(1);
                    let delay = domain_report_retry_delay(failures);
                    log_error(
                        LogServiceType::Register,
                        format!(
                            "Unable to report the custom domain (retrying in {}s): {:?}",
                            delay.as_secs(),
                            error
                        ),
                    );
                    delay
                }
            };
            tokio::time::sleep(delay).await;
        }
    });
}

fn describe_domain_report(report: &DomainReport) -> String {
    match &report.domain {
        Some(domain) => format!(
            "Reported custom domain https://{}{}",
            domain,
            report
                .port
                .map(|port| format!(":{port}"))
                .unwrap_or_default()
        ),
        None => "Reported no custom domain".to_string(),
    }
}

/// Reports the custom domain (or its absence) to the cloud.
pub async fn report_domain() -> Result<DomainReport> {
    let config = get_config().await;
    let id = config.id.clone().ok_or(crate::Error::ServerNoServerId)?;
    let token = config
        .token
        .clone()
        .ok_or(crate::Error::ServerNotYetRegistered)?;
    let report = config.domain_report();

    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?
        .patch(format!(
            "https://{}/api/servers/{}",
            config
                .redseat_home
                .trim_end_matches('/')
                .trim_start_matches("https://")
                .trim_start_matches("http://"),
            id
        ))
        .header("Authorization", format!("Token {}", token))
        .json(&report)
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(Error::Error(format!(
            "Domain report failed ({status}): {}",
            body.chars().take(200).collect::<String>()
        )));
    }
    Ok(report)
}

#[cfg(test)]
mod domain_report_tests {
    use super::*;

    fn config(domain: Option<&str>, exp_port: Option<u16>) -> ServerConfig {
        let mut config: ServerConfig = serde_json::from_str("{}").unwrap();
        config.domain = domain.map(str::to_string);
        config.exp_port = exp_port;
        config.port = Some(8080);
        config
    }

    #[test]
    fn domain_report_never_uses_the_listening_port() {
        // Behind a reverse proxy (Traefik…): 443, not the container's 8080.
        assert_eq!(
            config(Some("nseat.example.org"), None).domain_report(),
            DomainReport {
                domain: Some("nseat.example.org".into()),
                port: None
            }
        );
        assert_eq!(
            config(Some("https://nseat.example.org:8443/"), None).domain_report(),
            DomainReport {
                domain: Some("nseat.example.org".into()),
                port: Some(8443)
            }
        );
        assert_eq!(
            config(Some("nseat.example.org"), Some(9443))
                .domain_report()
                .port,
            Some(9443)
        );
        assert_eq!(
            config(Some("nseat.example.org:443"), None)
                .domain_report()
                .port,
            None
        );
    }

    #[test]
    fn retries_back_off_up_to_an_hour() {
        let minutes = |failures| domain_report_retry_delay(failures).as_secs() / 60;
        assert_eq!(
            [1, 2, 3, 4, 5, 6, 7, 8].map(minutes),
            [1, 2, 4, 8, 16, 32, 60, 60]
        );
        assert_eq!(minutes(u32::MAX), 60);
    }

    #[test]
    fn missing_domain_is_reported_as_null() {
        let report = config(None, Some(9443)).domain_report();
        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            serde_json::json!({ "domain": null })
        );
    }
}
