//! Direct HTTPS (redseat-svelte issue #62): browsers reach this server at
//! `https://<ip-encoded>.<label>.servers.redseat.cloud:<port>`.
//!
//! The background loop keeps the cloud up to date and the certificate installed:
//! - reports candidate addresses (`PATCH /api/servers/<id>`) when they change,
//! - reports the users this server accepts (`PUT …/members`) at startup and on change,
//! - polls `GET …/certificate`, sends a CSR when `csrNeeded` (the key is generated here and
//!   never leaves the server) and hot-reloads TLS when a new chain is issued.
//!
//! See `docs/DIRECT_HTTPS.md`.

pub mod addresses;
pub mod cloud;
pub mod tls;

use std::{
    net::IpAddr,
    sync::{Arc, OnceLock},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tokio::{sync::Notify, time::Instant};

use crate::{
    error::{RsError, RsResult},
    model::{users::ConnectedUser, ModelController},
    server::{get_server_file_string, write_server_file, ServerConfig},
    tools::log::{log_error, log_info, LogServiceType},
};

use self::{
    addresses::DiscoveryOptions,
    cloud::{AddressReport, CertificateStatus, CloudClient, CsrOutcome, IssuedCertificate},
    tls::{SniCertificate, SniResolver},
};

const STATE_FILE: &str = "direct_https.json";
const CHAIN_FILE: &str = "direct_cert_chain.pem";
const KEY_FILE: &str = "direct_cert_key.pem";
/// Keys of CSRs sent but not yet issued, newest first.
const PENDING_KEYS_FILE: &str = "direct_pending_keys.json";
const MAX_PENDING_KEYS: usize = 4;

const MINUTE: Duration = Duration::from_secs(60);
const DAY: Duration = Duration::from_secs(24 * 60 * 60);
const ADDRESS_CHECK_INTERVAL: Duration = Duration::from_secs(10 * 60);
const ORDER_POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);
const RETRY_INTERVAL: Duration = Duration::from_secs(15 * 60);

const MAX_MEMBERS: usize = 5000;

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DirectState {
    label: Option<String>,
    pending_label: Option<String>,
    /// `notBefore` (ms) of the installed certificate, as reported by the cloud.
    certificate_not_before: Option<i64>,
    certificate_label: Option<String>,
    /// Name requested by the latest CSR sent.
    requested_name: Option<String>,
}

fn members_notify() -> &'static Notify {
    static NOTIFY: OnceLock<Notify> = OnceLock::new();
    NOTIFY.get_or_init(Notify::new)
}

/// Call when server users are added or removed, so the cloud's member list is refreshed.
pub fn members_changed() {
    members_notify().notify_one();
}

/// Direct HTTPS runs for registered servers, except those behind a custom domain or a
/// TLS-terminating proxy (`noCert`), which keep working unchanged.
pub fn is_enabled(config: &ServerConfig) -> bool {
    config.id.is_some() && config.token.is_some() && config.domain.is_none() && !config.noCert
}

/// Stores the label returned at registration. The cloud stays authoritative: it is read
/// again from `GET …/certificate`.
pub async fn store_registration_label(label: &str) -> RsResult<()> {
    if label.is_empty()
        || label.len() > 63
        || !label
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    {
        return Err(RsError::Error(format!("Invalid direct HTTPS label: {label}")));
    }
    let mut state = load_state().await;
    state.label = Some(label.to_string());
    save_state(&state).await
}

/// Loads the installed certificate into `resolver` and starts the background loop.
/// Returns false when direct HTTPS is disabled for this server. IPv6 candidates are only
/// reported when the server listens on IPv6.
pub async fn start(
    config: &ServerConfig,
    resolver: Arc<SniResolver>,
    mc: ModelController,
    listening_ipv6: bool,
) -> bool {
    if !is_enabled(config) {
        return false;
    }
    let (Some(id), Some(token)) = (config.id.as_deref(), config.token.as_deref()) else {
        return false;
    };
    let cloud = match CloudClient::new(&config.redseat_home, id, token) {
        Ok(cloud) => cloud,
        Err(error) => {
            log_error(
                LogServiceType::Register,
                format!("Direct HTTPS disabled: {error:?}"),
            );
            return false;
        }
    };

    if let Some(certificate) = load_installed_certificate().await {
        log_info(
            LogServiceType::Register,
            format!("Direct HTTPS certificate loaded for {:?}", certificate.names()),
        );
        resolver.set_direct(certificate);
    }

    let manager = Manager {
        cloud,
        resolver,
        mc,
        discovery: discovery_options(config, listening_ipv6),
        state: load_state().await,
        reported: None,
        recovery_rotation_done: false,
    };
    tokio::spawn(manager.run());
    true
}

fn discovery_options(config: &ServerConfig, listening_ipv6: bool) -> DiscoveryOptions {
    let lan_override = config.lan_ips.as_ref().map(|ips| {
        ips.iter()
            .filter_map(|ip| match ip.trim().parse::<IpAddr>() {
                Ok(ip) => Some(ip),
                Err(_) => {
                    log_error(
                        LogServiceType::Register,
                        format!("Direct HTTPS: ignoring invalid LAN address {ip:?}"),
                    );
                    None
                }
            })
            .collect()
    });
    DiscoveryOptions {
        port: config.get_exposed_port(),
        local_port: config.get_port(),
        lan_override,
        port_forwarded: config.port_forwarded,
        ipv6: listening_ipv6,
    }
}

async fn load_installed_certificate() -> Option<SniCertificate> {
    let chain = get_server_file_string(CHAIN_FILE).await.ok().flatten()?;
    let key = get_server_file_string(KEY_FILE).await.ok().flatten()?;
    match SniCertificate::from_pem(&chain, &key) {
        Ok(certificate) => Some(certificate),
        Err(error) => {
            log_error(
                LogServiceType::Register,
                format!("Direct HTTPS: unable to load the installed certificate: {error:?}"),
            );
            None
        }
    }
}

async fn load_state() -> DirectState {
    get_server_file_string(STATE_FILE)
        .await
        .ok()
        .flatten()
        .and_then(|data| serde_json::from_str(&data).ok())
        .unwrap_or_default()
}

async fn save_state(state: &DirectState) -> RsResult<()> {
    write_server_file(STATE_FILE, &serde_json::to_vec_pretty(state)?).await
}

async fn load_pending_keys() -> Vec<String> {
    get_server_file_string(PENDING_KEYS_FILE)
        .await
        .ok()
        .flatten()
        .and_then(|data| serde_json::from_str(&data).ok())
        .unwrap_or_default()
}

async fn save_pending_keys(keys: &[String]) -> RsResult<()> {
    write_server_file(PENDING_KEYS_FILE, &serde_json::to_vec(keys)?).await
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Uids the cloud accepts as member ids (Firebase keys: no `.#$[]/` or control characters).
fn valid_member_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && !id
            .chars()
            .any(|c| matches!(c, '.' | '#' | '$' | '[' | ']' | '/') || c.is_control())
}

struct Manager {
    cloud: CloudClient,
    resolver: Arc<SniResolver>,
    mc: ModelController,
    discovery: DiscoveryOptions,
    state: DirectState,
    /// Last report the cloud accepted, and when.
    reported: Option<(AddressReport, Instant)>,
    /// A lost certificate key is recovered at most once per run by rotating the label.
    recovery_rotation_done: bool,
}

impl Manager {
    async fn run(mut self) {
        let start = Instant::now();
        let mut next_addresses = start;
        let mut next_members = start;
        let mut next_certificate = start;
        loop {
            let now = Instant::now();
            if now >= next_addresses {
                next_addresses = Instant::now() + self.check_addresses().await;
            }
            if now >= next_members {
                next_members = Instant::now() + self.sync_members().await;
            }
            if now >= next_certificate {
                next_certificate = Instant::now() + self.check_certificate().await;
            }
            let next = next_addresses.min(next_members).min(next_certificate);
            tokio::select! {
                _ = tokio::time::sleep_until(next) => {}
                _ = members_notify().notified() => next_members = Instant::now(),
            }
        }
    }

    async fn check_addresses(&mut self) -> Duration {
        let report = addresses::discover(&self.discovery).await;
        let up_to_date = self
            .reported
            .as_ref()
            .is_some_and(|(reported, at)| *reported == report && at.elapsed() < DAY);
        if up_to_date {
            return ADDRESS_CHECK_INTERVAL;
        }
        match self.cloud.report_addresses(&report).await {
            Ok(()) => {
                log_info(
                    LogServiceType::Register,
                    format!("Direct HTTPS: reported addresses {report:?}"),
                );
                self.reported = Some((report, Instant::now()));
                ADDRESS_CHECK_INTERVAL
            }
            Err(error) => {
                log_error(
                    LogServiceType::Register,
                    format!("Direct HTTPS: unable to report addresses: {error:?}"),
                );
                ORDER_POLL_INTERVAL
            }
        }
    }

    async fn sync_members(&mut self) -> Duration {
        let users = match self.mc.get_users(&ConnectedUser::ServerAdmin).await {
            Ok(users) => users,
            Err(error) => {
                log_error(
                    LogServiceType::Register,
                    format!("Direct HTTPS: unable to list users: {error:?}"),
                );
                return ORDER_POLL_INTERVAL;
            }
        };
        let mut ids: Vec<String> = users
            .into_iter()
            .map(|user| user.id)
            .filter(|id| valid_member_id(id))
            .collect();
        ids.sort();
        ids.dedup();
        ids.truncate(MAX_MEMBERS);
        match self.cloud.report_members(&ids).await {
            Ok(()) => {
                log_info(
                    LogServiceType::Register,
                    format!("Direct HTTPS: reported {} members", ids.len()),
                );
                DAY
            }
            Err(error) => {
                log_error(
                    LogServiceType::Register,
                    format!("Direct HTTPS: unable to report members: {error:?}"),
                );
                ORDER_POLL_INTERVAL
            }
        }
    }

    async fn update_state(&mut self, update: impl FnOnce(&mut DirectState)) {
        let mut state = self.state.clone();
        update(&mut state);
        if state != self.state {
            self.state = state;
            if let Err(error) = save_state(&self.state).await {
                log_error(
                    LogServiceType::Register,
                    format!("Direct HTTPS: unable to save state: {error:?}"),
                );
            }
        }
    }

    /// Polls the certificate status, installs a newly issued chain and sends a CSR when needed.
    /// Returns the delay until the next poll.
    async fn check_certificate(&mut self) -> Duration {
        let status = match self.cloud.certificate_status().await {
            Ok(status) => status,
            Err(error) => {
                log_error(
                    LogServiceType::Register,
                    format!("Direct HTTPS: unable to read certificate status: {error:?}"),
                );
                return RETRY_INTERVAL;
            }
        };
        self.update_state(|state| {
            state.label = Some(status.label.clone());
            state.pending_label = status.pending_label.clone();
        })
        .await;

        let mut key_lost = false;
        if let Some(certificate) = &status.certificate {
            let is_new = certificate.label == status.label
                && self.state.certificate_not_before != Some(certificate.not_before);
            if is_new {
                key_lost = !self.install(certificate).await;
            }
        }

        let order_pending = status.order.as_ref().is_some_and(|order| !order.is_failed());
        if let Some(order) = status.order.as_ref().filter(|order| order.is_failed()) {
            log_error(
                LogServiceType::Register,
                format!(
                    "Direct HTTPS: certificate order {} failed: {}",
                    order.id,
                    order.error.as_deref().unwrap_or("unknown error")
                ),
            );
        }

        if status.csr_needed {
            // A queued or running order for this name uses a key we hold: let it finish.
            let waiting_on_our_order = order_pending
                && self.state.requested_name.as_deref() == Some(status.name.as_str())
                && !load_pending_keys().await.is_empty();
            if !waiting_on_our_order {
                return self.send_csr(&status.name).await;
            }
        } else if key_lost && !order_pending && status.pending_label.is_none() {
            return self.recover_lost_key().await;
        }

        if order_pending {
            return order_poll_delay(&status);
        }
        DAY
    }

    /// Installs `certificate` with the matching local key. Returns false when no key matches.
    async fn install(&mut self, certificate: &IssuedCertificate) -> bool {
        let mut pending_keys = load_pending_keys().await;
        let current_key = get_server_file_string(KEY_FILE).await.ok().flatten();
        let candidates = pending_keys
            .iter()
            .cloned()
            .map(Some)
            .chain(std::iter::once(current_key));
        for key in candidates.flatten() {
            let Ok(loaded) = SniCertificate::from_pem(&certificate.chain, &key) else {
                continue;
            };
            // Key first: a chain on disk is only loaded together with its key.
            let written = async {
                write_server_file(KEY_FILE, key.as_bytes()).await?;
                write_server_file(CHAIN_FILE, certificate.chain.as_bytes()).await
            }
            .await;
            if let Err(error) = written {
                log_error(
                    LogServiceType::Register,
                    format!("Direct HTTPS: unable to save the certificate: {error:?}"),
                );
            }
            log_info(
                LogServiceType::Register,
                format!(
                    "Direct HTTPS: installed certificate for {:?}, valid until {}",
                    loaded.names(),
                    chrono::DateTime::from_timestamp_millis(certificate.not_after)
                        .map(|date| date.to_rfc3339())
                        .unwrap_or_default()
                ),
            );
            self.resolver.set_direct(loaded);
            pending_keys.retain(|pending| *pending != key);
            if let Err(error) = save_pending_keys(&pending_keys).await {
                log_error(
                    LogServiceType::Register,
                    format!("Direct HTTPS: unable to save pending keys: {error:?}"),
                );
            }
            let (not_before, label) = (certificate.not_before, certificate.label.clone());
            self.update_state(|state| {
                state.certificate_not_before = Some(not_before);
                state.certificate_label = Some(label);
            })
            .await;
            return true;
        }
        log_error(
            LogServiceType::Register,
            format!(
                "Direct HTTPS: no local key matches the issued certificate for label {}",
                certificate.label
            ),
        );
        false
    }

    async fn send_csr(&mut self, name: &str) -> Duration {
        let request = match tls::create_csr(name) {
            Ok(request) => request,
            Err(error) => {
                log_error(
                    LogServiceType::Register,
                    format!("Direct HTTPS: unable to create a CSR: {error:?}"),
                );
                return RETRY_INTERVAL;
            }
        };
        // Save the key before sending: the certificate may be issued even if we stop now.
        let mut pending_keys = load_pending_keys().await;
        pending_keys.insert(0, request.key_pem);
        pending_keys.truncate(MAX_PENDING_KEYS);
        if let Err(error) = save_pending_keys(&pending_keys).await {
            log_error(
                LogServiceType::Register,
                format!("Direct HTTPS: unable to save the private key: {error:?}"),
            );
            return RETRY_INTERVAL;
        }

        match self.cloud.request_certificate(&request.csr_pem).await {
            Ok(CsrOutcome::Queued) => {
                log_info(
                    LogServiceType::Register,
                    format!("Direct HTTPS: certificate requested for {name}"),
                );
                let name = name.to_string();
                self.update_state(|state| state.requested_name = Some(name))
                    .await;
                ORDER_POLL_INTERVAL
            }
            Ok(CsrOutcome::Processing) => {
                log_info(
                    LogServiceType::Register,
                    "Direct HTTPS: a certificate order is already running".to_string(),
                );
                ORDER_POLL_INTERVAL
            }
            Ok(CsrOutcome::Refused { status, message }) => {
                // 409 (not due), 429 (weekly limit), 400 (bad CSR), 503 (no issuance):
                // wait for the next daily check instead of retrying.
                log_error(
                    LogServiceType::Register,
                    format!("Direct HTTPS: certificate request refused ({status}): {message}"),
                );
                DAY
            }
            Err(error) => {
                log_error(
                    LogServiceType::Register,
                    format!("Direct HTTPS: certificate request failed: {error:?}"),
                );
                RETRY_INTERVAL
            }
        }
    }

    /// The active certificate's key is gone (e.g. the config folder was restored without it)
    /// and the cloud won't accept a CSR before the renewal window. Rotating the label makes
    /// the cloud ask for a CSR for the new label right away.
    async fn recover_lost_key(&mut self) -> Duration {
        if self.recovery_rotation_done {
            return DAY;
        }
        self.recovery_rotation_done = true;
        match self.cloud.rotate_label().await {
            Ok(label) => {
                log_info(
                    LogServiceType::Register,
                    format!(
                        "Direct HTTPS: rotated label to recover the certificate key (pending {:?})",
                        label.pending_label
                    ),
                );
                MINUTE
            }
            Err(error) => {
                log_error(
                    LogServiceType::Register,
                    format!("Direct HTTPS: unable to rotate label: {error:?}"),
                );
                DAY
            }
        }
    }
}

/// While an order is pending, poll every few minutes, or when the weekly budget frees up.
fn order_poll_delay(status: &CertificateStatus) -> Duration {
    let queued_until = status
        .order
        .as_ref()
        .and_then(|order| order.queue.as_ref())
        .and_then(|queue| queue.not_before)
        .map(|not_before| not_before - now_ms())
        .filter(|wait| *wait > 0)
        .map(|wait| Duration::from_millis(wait as u64));
    match queued_until {
        Some(wait) => wait.clamp(ORDER_POLL_INTERVAL, DAY),
        None => ORDER_POLL_INTERVAL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn member_ids_follow_firebase_key_rules() {
        assert!(valid_member_id("kF3x9aB2cD4eF5gH6iJ7kL8mN9o1"));
        assert!(!valid_member_id(""));
        assert!(!valid_member_id("a.b"));
        assert!(!valid_member_id("a/b"));
        assert!(!valid_member_id(&"a".repeat(129)));
    }

    #[test]
    fn queued_orders_poll_when_the_budget_frees_up() {
        let status = |not_before: Option<i64>| CertificateStatus {
            label: "abc".into(),
            pending_label: None,
            name: "*.abc.servers.redseat.cloud".into(),
            certificate: None,
            csr_needed: false,
            order: Some(cloud::CertificateOrder {
                id: "o".into(),
                status: "queued".into(),
                error: None,
                queue: Some(cloud::OrderQueue {
                    position: Some(3),
                    not_before,
                }),
            }),
        };
        assert_eq!(order_poll_delay(&status(None)), ORDER_POLL_INTERVAL);
        let hours = order_poll_delay(&status(Some(now_ms() + 3 * 60 * 60 * 1000)));
        assert!(hours > Duration::from_secs(2 * 60 * 60) && hours <= DAY);
        assert_eq!(
            order_poll_delay(&status(Some(now_ms() + 30 * DAY.as_millis() as i64))),
            DAY
        );
    }
}
