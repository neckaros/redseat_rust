use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use futures::{SinkExt, StreamExt};
use rand::Rng;
use serde::{Deserialize, Serialize};
use tokio::{net::TcpStream, sync::mpsc, task::JoinHandle};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{
        protocol::{frame::coding::CloseCode, CloseFrame, WebSocketConfig},
        Message,
    },
    MaybeTlsStream, WebSocketStream,
};
use tokio_util::sync::CancellationToken;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;

use crate::{
    server::ServerConfig,
    tools::log::{log_error, log_info, LogServiceType},
};

use super::peer::{EstablishedPeerSink, IceServer, PeerEvent, PeerRegistry, PeerSession};

const PROTOCOL_VERSION: u8 = 1;
const MAX_MESSAGE_SIZE: usize = 64 * 1024;
const MAX_SDP_SIZE: usize = 60_000;
const MAX_IDENTIFIER_SIZE: usize = 128;
const MAX_ERROR_SIZE: usize = 512;
const MAX_PENDING_SESSIONS: usize = 64;
const MAX_QUEUED_CANDIDATES: usize = 256;
const AUTH_TIMEOUT: Duration = Duration::from_secs(10);
const STABLE_CONNECTION: Duration = Duration::from_secs(30);

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub struct SignalingHandle {
    shutdown: CancellationToken,
    task: Option<JoinHandle<()>>,
    pub peers: Arc<PeerRegistry>,
}

impl SignalingHandle {
    pub async fn shutdown(mut self) {
        self.shutdown.cancel();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for SignalingHandle {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

pub fn start_from_config(config: &ServerConfig) -> Option<SignalingHandle> {
    let server_id = config.id.clone().filter(|value| !value.is_empty())?;
    let token = config.token.clone().filter(|value| !value.is_empty())?;
    let url = config.get_signaling_url();
    if !(url.starts_with("wss://") || url.starts_with("ws://")) {
        log_error(
            LogServiceType::Register,
            "WebRTC signaling URL must use ws:// or wss://".to_owned(),
        );
        return None;
    }

    let shutdown = CancellationToken::new();
    let peers = Arc::new(PeerRegistry::new());
    let task_shutdown = shutdown.clone();
    let task_peers = Arc::clone(&peers);
    let task = tokio::spawn(async move {
        supervise(url, server_id, token, task_peers, task_shutdown).await;
    });
    Some(SignalingHandle {
        shutdown,
        task: Some(task),
        peers,
    })
}

async fn supervise(
    url: String,
    server_id: String,
    token: String,
    peers: Arc<PeerRegistry>,
    shutdown: CancellationToken,
) {
    let mut retry = 0u32;
    while !shutdown.is_cancelled() {
        let started = Instant::now();
        let result =
            run_connection(&url, &server_id, &token, peers.as_ref(), shutdown.clone()).await;
        if shutdown.is_cancelled() {
            break;
        }
        if let Err(error) = result {
            log_error(
                LogServiceType::Other,
                format!("WebRTC signaling disconnected: {error}"),
            );
        }
        retry = if started.elapsed() >= STABLE_CONNECTION {
            0
        } else {
            retry.saturating_add(1).min(6)
        };
        let base_ms = 500u64.saturating_mul(1u64 << retry).min(30_000);
        let jitter_ms = rand::thread_rng().gen_range(0..=(base_ms / 4).max(1));
        tokio::select! {
            _ = shutdown.cancelled() => break,
            _ = tokio::time::sleep(Duration::from_millis(base_ms + jitter_ms)) => {}
        }
    }
    peers.shutdown().await;
}

async fn run_connection(
    url: &str,
    server_id: &str,
    token: &str,
    peer_sink: &dyn EstablishedPeerSink,
    shutdown: CancellationToken,
) -> Result<(), String> {
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE_SIZE))
        .max_frame_size(Some(MAX_MESSAGE_SIZE));
    let connection = tokio::select! {
        _ = shutdown.cancelled() => return Ok(()),
        connection = connect_async_with_config(url, Some(config), false) => connection,
    };
    let (mut socket, _) =
        connection.map_err(|_| "unable to connect to cloud rendezvous".to_owned())?;

    send_json(
        &mut socket,
        &ClientMessage::Authenticate {
            v: PROTOCOL_VERSION,
            role: "server",
            server_id,
            token,
        },
    )
    .await?;
    authenticate(&mut socket, server_id, &shutdown).await?;
    log_info(
        LogServiceType::Other,
        format!("WebRTC signaling authenticated for server {server_id}"),
    );

    let (peer_event_tx, mut peer_event_rx) = mpsc::channel(512);
    let mut pending = HashMap::<String, PendingSession>::new();
    let mut established_sessions = HashSet::<String>::new();
    let mut expiry = tokio::time::interval(Duration::from_secs(1));

    let result = loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                let _ = socket.send(Message::Close(Some(CloseFrame {
                    code: CloseCode::Away,
                    reason: "server shutting down".into(),
                }))).await;
                break Ok(());
            }
            _ = expiry.tick() => {
                let expired = pending.iter()
                    .filter(|(_, session)| session.expires_at <= Utc::now())
                    .map(|(id, _)| id.clone())
                    .collect::<Vec<_>>();
                for session_id in expired {
                    fail_session(&mut socket, &mut pending, &session_id, "signaling session expired").await;
                }
            }
            event = peer_event_rx.recv() => {
                if let Some(event) = event {
                    handle_peer_event(
                        &mut socket,
                        &mut pending,
                        &mut established_sessions,
                        peer_sink,
                        event,
                    ).await;
                }
            }
            message = socket.next() => {
                match message {
                    Some(Ok(Message::Text(text))) => {
                        if text.len() > MAX_MESSAGE_SIZE {
                            break Err("cloud sent an oversized signaling message".to_owned());
                        }
                        let message = match serde_json::from_str::<CloudMessage>(&text) {
                            Ok(message) => message,
                            Err(_) => break Err("cloud sent invalid signaling JSON".to_owned()),
                        };
                        if let Err(error) = handle_cloud_message(
                            &mut socket,
                            &mut pending,
                            &established_sessions,
                            peer_sink,
                            server_id,
                            peer_event_tx.clone(),
                            message,
                        ).await {
                            break Err(error);
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            break Err("unable to answer signaling heartbeat".to_owned());
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(frame))) => {
                        break Err(describe_close(frame));
                    }
                    Some(Ok(Message::Binary(_))) => {
                        break Err("cloud sent a binary signaling message".to_owned());
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break Err("cloud signaling socket failed".to_owned()),
                    None => break Err("cloud signaling socket closed".to_owned()),
                }
            }
        }
    };

    // A DataChannel-open callback and signaling close can become ready together.
    // Drain already-produced peer events before invalidating pending negotiations.
    tokio::task::yield_now().await;
    while let Ok(event) = peer_event_rx.try_recv() {
        handle_peer_event(
            &mut socket,
            &mut pending,
            &mut established_sessions,
            peer_sink,
            event,
        )
        .await;
    }
    close_all_pending(&mut pending).await;
    result
}

async fn authenticate(
    socket: &mut Socket,
    server_id: &str,
    shutdown: &CancellationToken,
) -> Result<(), String> {
    let deadline = tokio::time::sleep(AUTH_TIMEOUT);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return Err("signaling stopped during authentication".to_owned()),
            _ = &mut deadline => return Err("cloud signaling authentication timed out".to_owned()),
            message = socket.next() => match message {
                Some(Ok(Message::Text(text))) if text.len() <= MAX_MESSAGE_SIZE => {
                    let message = serde_json::from_str::<CloudMessage>(&text)
                        .map_err(|_| "cloud sent invalid authentication response".to_owned())?;
                    match message {
                        CloudMessage::Authenticated { v, role, server_id: received }
                            if v == PROTOCOL_VERSION && role == "server" && received.as_deref() == Some(server_id) => return Ok(()),
                        CloudMessage::Error { v, code, .. } if v == PROTOCOL_VERSION => {
                            return Err(format!("cloud rejected signaling authentication ({code})"));
                        }
                        _ => return Err("cloud sent a message before authentication completed".to_owned()),
                    }
                }
                Some(Ok(Message::Ping(payload))) => {
                    socket.send(Message::Pong(payload)).await
                        .map_err(|_| "unable to answer signaling heartbeat".to_owned())?;
                }
                Some(Ok(Message::Close(frame))) => return Err(describe_close(frame)),
                Some(Ok(_)) => return Err("invalid signaling authentication frame".to_owned()),
                Some(Err(_)) | None => return Err("signaling closed during authentication".to_owned()),
            }
        }
    }
}

async fn handle_cloud_message(
    socket: &mut Socket,
    pending: &mut HashMap<String, PendingSession>,
    established_sessions: &HashSet<String>,
    peer_sink: &dyn EstablishedPeerSink,
    server_id: &str,
    event_tx: mpsc::Sender<PeerEvent>,
    message: CloudMessage,
) -> Result<(), String> {
    if message.version() != PROTOCOL_VERSION {
        return Err("cloud sent an unsupported signaling protocol version".to_owned());
    }
    match message {
        CloudMessage::SessionRequest {
            v,
            session_id,
            server_id: requested_server,
            expires_at,
            user,
            ice_servers,
        } => {
            if v != PROTOCOL_VERSION
                || !valid_identifier(&session_id)
                || requested_server != server_id
                || !valid_identifier(&user.uid)
                || pending.contains_key(&session_id)
                || pending.len() >= MAX_PENDING_SESSIONS
                || peer_sink.contains_session(&session_id).await
            {
                send_session_error(socket, &session_id, "invalid signaling session request").await;
                return Ok(());
            }
            let Ok(expires_at) =
                DateTime::parse_from_rfc3339(&expires_at).map(|time| time.with_timezone(&Utc))
            else {
                send_session_error(socket, &session_id, "invalid signaling session expiry").await;
                return Ok(());
            };
            if expires_at <= Utc::now() {
                send_session_error(socket, &session_id, "signaling session expired").await;
                return Ok(());
            }
            let ice_servers = match validate_ice_servers(ice_servers.unwrap_or_default()) {
                Ok(servers) => servers,
                Err(message) => {
                    send_session_error(socket, &session_id, message).await;
                    return Ok(());
                }
            };
            match PeerSession::new(session_id.clone(), user.uid, ice_servers, event_tx).await {
                Ok(peer) => {
                    pending.insert(
                        session_id,
                        PendingSession {
                            expires_at,
                            peer,
                            offer_seen: false,
                            remote_description_set: false,
                            queued_candidates: VecDeque::new(),
                        },
                    );
                }
                Err(_) => {
                    send_session_error(socket, &session_id, "unable to create WebRTC peer").await
                }
            }
        }
        CloudMessage::Signal {
            v,
            session_id,
            signal,
        } => {
            if v != PROTOCOL_VERSION
                || !valid_identifier(&session_id)
                || signal.session_id() != session_id
            {
                fail_session(
                    socket,
                    pending,
                    &session_id,
                    "invalid signaling session identifier",
                )
                .await;
                return Ok(());
            }
            handle_signal(socket, pending, established_sessions, session_id, signal).await;
        }
        CloudMessage::Error {
            v,
            code,
            session_id,
            ..
        } => {
            if v != PROTOCOL_VERSION {
                return Ok(());
            }
            if let Some(session_id) = session_id {
                if let Some(session) = pending.remove(&session_id) {
                    session.peer.close().await;
                }
            } else if matches!(
                code.as_str(),
                "UNAUTHORIZED" | "AUTH_TIMEOUT" | "PROTOCOL_ERROR"
            ) {
                // The broker normally closes the socket immediately after these errors.
                log_error(
                    LogServiceType::Other,
                    format!("Cloud signaling error: {code}"),
                );
            }
        }
        CloudMessage::Authenticated { .. } => {
            return Err("cloud repeated signaling authentication".to_owned());
        }
    }
    Ok(())
}

async fn handle_signal(
    socket: &mut Socket,
    pending: &mut HashMap<String, PendingSession>,
    established_sessions: &HashSet<String>,
    session_id: String,
    signal: PeerSignal,
) {
    let Some(session) = pending.get_mut(&session_id) else {
        if established_sessions.contains(&session_id) && matches!(signal, PeerSignal::Close { .. })
        {
            return;
        }
        send_session_error(socket, &session_id, "unknown or expired signaling session").await;
        return;
    };
    match signal {
        PeerSignal::Offer { description, .. } => {
            if session.offer_seen
                || description.kind != "offer"
                || description.sdp.is_empty()
                || description.sdp.len() > MAX_SDP_SIZE
            {
                fail_session(socket, pending, &session_id, "invalid WebRTC offer").await;
                return;
            }
            session.offer_seen = true;
            match session.peer.accept_offer(description.sdp).await {
                Ok(answer) => {
                    session.remote_description_set = true;
                    while let Some(candidate) = session.queued_candidates.pop_front() {
                        if session.peer.add_remote_candidate(candidate).await.is_err() {
                            fail_session(
                                socket,
                                pending,
                                &session_id,
                                "invalid remote ICE candidate",
                            )
                            .await;
                            return;
                        }
                    }
                    let message = ClientMessage::Signal {
                        v: PROTOCOL_VERSION,
                        session_id: &session_id,
                        signal: ServerSignal::Answer {
                            session_id: &session_id,
                            description: OutDescription {
                                kind: "answer",
                                sdp: &answer,
                            },
                        },
                    };
                    if send_json(socket, &message).await.is_err() {
                        fail_session(socket, pending, &session_id, "unable to send WebRTC answer")
                            .await;
                    }
                }
                Err(_) => {
                    fail_session(
                        socket,
                        pending,
                        &session_id,
                        "unable to accept WebRTC offer",
                    )
                    .await
                }
            }
        }
        PeerSignal::IceCandidate { candidate, .. } => {
            if candidate
                .as_ref()
                .is_some_and(|candidate| candidate.invalid())
            {
                fail_session(socket, pending, &session_id, "invalid remote ICE candidate").await;
                return;
            }
            if candidate
                .as_ref()
                .is_some_and(|candidate| !super::peer::is_direct_candidate(&candidate.candidate))
            {
                // Peer-reflexive and relay candidates are not exchanged in direct-only v1.
                return;
            }
            let candidate = candidate.map(Into::into);
            if session.remote_description_set {
                if session.peer.add_remote_candidate(candidate).await.is_err() {
                    fail_session(socket, pending, &session_id, "invalid remote ICE candidate")
                        .await;
                }
            } else if session.queued_candidates.len() >= MAX_QUEUED_CANDIDATES {
                fail_session(
                    socket,
                    pending,
                    &session_id,
                    "too many queued ICE candidates",
                )
                .await;
            } else {
                session.queued_candidates.push_back(candidate);
            }
        }
        PeerSignal::Close { .. } => {
            if session.peer.is_established() {
                return;
            }
            if let Some(session) = pending.remove(&session_id) {
                session.peer.close().await;
            }
        }
        PeerSignal::Answer { .. } | PeerSignal::Error { .. } => {
            fail_session(
                socket,
                pending,
                &session_id,
                "invalid client signaling direction",
            )
            .await;
        }
    }
}

async fn handle_peer_event(
    socket: &mut Socket,
    pending: &mut HashMap<String, PendingSession>,
    established_sessions: &mut HashSet<String>,
    peer_sink: &dyn EstablishedPeerSink,
    event: PeerEvent,
) {
    match event {
        PeerEvent::LocalCandidate {
            session_id,
            candidate,
        } if pending.contains_key(&session_id) => {
            let message = ClientMessage::Signal {
                v: PROTOCOL_VERSION,
                session_id: &session_id,
                signal: ServerSignal::IceCandidate {
                    session_id: &session_id,
                    candidate,
                },
            };
            if send_json(socket, &message).await.is_err() {
                fail_session(
                    socket,
                    pending,
                    &session_id,
                    "unable to send local ICE candidate",
                )
                .await;
            }
        }
        PeerEvent::Established(peer) => {
            let session_id = peer.session_id.clone();
            if pending.remove(&session_id).is_some() {
                let peer_connection = Arc::clone(&peer.peer_connection);
                if peer_sink.accept(peer).await.is_err() {
                    let _ = peer_connection.close().await;
                    send_session_error(socket, &session_id, "unable to accept established peer")
                        .await;
                } else {
                    established_sessions.insert(session_id);
                }
            }
        }
        PeerEvent::Failed {
            session_id,
            message,
        } if pending.contains_key(&session_id) => {
            fail_session(socket, pending, &session_id, message).await;
        }
        _ => {}
    }
}

async fn fail_session(
    socket: &mut Socket,
    pending: &mut HashMap<String, PendingSession>,
    session_id: &str,
    message: &str,
) {
    if let Some(session) = pending.remove(session_id) {
        session.peer.close().await;
    }
    send_session_error(socket, session_id, message).await;
}

async fn send_session_error(socket: &mut Socket, session_id: &str, message: &str) {
    if !valid_identifier(session_id) {
        return;
    }
    let safe_message = message.chars().take(MAX_ERROR_SIZE).collect::<String>();
    let envelope = ClientMessage::Signal {
        v: PROTOCOL_VERSION,
        session_id,
        signal: ServerSignal::Error {
            session_id,
            message: &safe_message,
        },
    };
    let _ = send_json(socket, &envelope).await;
}

async fn close_all_pending(pending: &mut HashMap<String, PendingSession>) {
    let sessions = pending
        .drain()
        .map(|(_, session)| session)
        .collect::<Vec<_>>();
    for session in sessions {
        session.peer.close().await;
    }
}

async fn send_json<T: Serialize>(socket: &mut Socket, message: &T) -> Result<(), String> {
    let text = serde_json::to_string(message)
        .map_err(|_| "unable to encode signaling message".to_owned())?;
    if text.len() > MAX_MESSAGE_SIZE {
        return Err("outgoing signaling message is too large".to_owned());
    }
    socket
        .send(Message::Text(text.into()))
        .await
        .map_err(|_| "unable to send signaling message".to_owned())
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_IDENTIFIER_SIZE
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
}

fn validate_ice_servers(servers: Vec<IceServerWire>) -> Result<Vec<IceServer>, &'static str> {
    if servers.len() > 16 {
        return Err("too many ICE servers");
    }
    let mut result = Vec::new();
    for server in servers {
        let urls = server.urls.into_vec();
        if urls.len() > 16
            || urls.iter().any(|url| url.len() > 2048)
            || server
                .username
                .as_ref()
                .is_some_and(|value| value.len() > 1024)
            || server
                .credential
                .as_ref()
                .is_some_and(|value| value.len() > 4096)
        {
            return Err("invalid ICE server configuration");
        }
        let urls = urls
            .into_iter()
            .filter(|url| !super::peer::is_turn_url(url))
            .collect::<Vec<_>>();
        if !urls.is_empty() {
            result.push(IceServer {
                urls,
                username: server.username.unwrap_or_default(),
                credential: server.credential.unwrap_or_default(),
            });
        }
    }
    Ok(result)
}

fn describe_close(frame: Option<CloseFrame>) -> String {
    match frame.map(|frame| u16::from(frame.code)) {
        Some(4001) => "cloud closed signaling after authentication failure".to_owned(),
        Some(4002) => "cloud closed signaling after a protocol error".to_owned(),
        Some(4003) => "cloud closed signaling because capacity was reached".to_owned(),
        Some(4004) => "cloud replaced this server signaling connection".to_owned(),
        Some(1001) => "cloud signaling service is shutting down".to_owned(),
        Some(code) => format!("cloud closed signaling (code {code})"),
        None => "cloud closed signaling".to_owned(),
    }
}

struct PendingSession {
    expires_at: DateTime<Utc>,
    peer: PeerSession,
    offer_seen: bool,
    remote_description_set: bool,
    queued_candidates: VecDeque<Option<RTCIceCandidateInit>>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum CloudMessage {
    Authenticated {
        v: u8,
        role: String,
        #[serde(rename = "serverId")]
        server_id: Option<String>,
    },
    SessionRequest {
        v: u8,
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "serverId")]
        server_id: String,
        #[serde(rename = "expiresAt")]
        expires_at: String,
        user: SignalingUser,
        #[serde(rename = "iceServers")]
        ice_servers: Option<Vec<IceServerWire>>,
    },
    Signal {
        v: u8,
        #[serde(rename = "sessionId")]
        session_id: String,
        signal: PeerSignal,
    },
    Error {
        v: u8,
        code: String,
        message: String,
        #[serde(rename = "requestId")]
        request_id: Option<String>,
        #[serde(rename = "sessionId")]
        session_id: Option<String>,
    },
}

impl CloudMessage {
    fn version(&self) -> u8 {
        match self {
            Self::Authenticated { v, .. }
            | Self::SessionRequest { v, .. }
            | Self::Signal { v, .. }
            | Self::Error { v, .. } => *v,
        }
    }
}

#[derive(Deserialize)]
struct SignalingUser {
    uid: String,
}

#[derive(Deserialize)]
struct IceServerWire {
    urls: StringOrStrings,
    username: Option<String>,
    credential: Option<String>,
    #[serde(rename = "credentialType")]
    credential_type: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StringOrStrings {
    One(String),
    Many(Vec<String>),
}

impl StringOrStrings {
    fn into_vec(self) -> Vec<String> {
        match self {
            Self::One(value) => vec![value],
            Self::Many(values) => values,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum PeerSignal {
    Offer {
        #[serde(rename = "sessionId")]
        session_id: String,
        description: InDescription,
    },
    IceCandidate {
        #[serde(rename = "sessionId")]
        session_id: String,
        candidate: Option<CandidateWire>,
    },
    Close {
        #[serde(rename = "sessionId")]
        session_id: String,
        reason: Option<String>,
    },
    Answer {
        #[serde(rename = "sessionId")]
        session_id: String,
        description: InDescription,
    },
    Error {
        #[serde(rename = "sessionId")]
        session_id: String,
        message: String,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CandidateWire {
    #[serde(default)]
    candidate: String,
    sdp_mid: Option<String>,
    #[serde(rename = "sdpMLineIndex")]
    sdp_mline_index: Option<u16>,
    username_fragment: Option<String>,
}

impl CandidateWire {
    fn invalid(&self) -> bool {
        self.candidate.len() > 4096
            || self.sdp_mid.as_ref().is_some_and(|value| value.len() > 256)
            || self
                .username_fragment
                .as_ref()
                .is_some_and(|value| value.len() > 256)
    }
}

impl From<CandidateWire> for RTCIceCandidateInit {
    fn from(candidate: CandidateWire) -> Self {
        Self {
            candidate: candidate.candidate,
            sdp_mid: candidate.sdp_mid,
            sdp_mline_index: candidate.sdp_mline_index,
            username_fragment: candidate.username_fragment,
        }
    }
}

impl PeerSignal {
    fn session_id(&self) -> &str {
        match self {
            Self::Offer { session_id, .. }
            | Self::IceCandidate { session_id, .. }
            | Self::Close { session_id, .. }
            | Self::Answer { session_id, .. }
            | Self::Error { session_id, .. } => session_id,
        }
    }
}

#[derive(Deserialize)]
struct InDescription {
    #[serde(rename = "type")]
    kind: String,
    sdp: String,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum ClientMessage<'a> {
    Authenticate {
        v: u8,
        role: &'static str,
        #[serde(rename = "serverId")]
        server_id: &'a str,
        token: &'a str,
    },
    Signal {
        v: u8,
        #[serde(rename = "sessionId")]
        session_id: &'a str,
        signal: ServerSignal<'a>,
    },
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum ServerSignal<'a> {
    Answer {
        #[serde(rename = "sessionId")]
        session_id: &'a str,
        description: OutDescription<'a>,
    },
    IceCandidate {
        #[serde(rename = "sessionId")]
        session_id: &'a str,
        candidate: Option<RTCIceCandidateInit>,
    },
    Error {
        #[serde(rename = "sessionId")]
        session_id: &'a str,
        message: &'a str,
    },
}

#[derive(Serialize)]
struct OutDescription<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    sdp: &'a str,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use tokio::net::TcpListener;
    use tokio_tungstenite::accept_async;

    #[test]
    fn authentication_uses_body_and_never_url() {
        let config: ServerConfig = serde_json::from_value(json!({
            "id": "server-1",
            "token": "a-secret-token-value",
            "redseat_home": "cloud.example",
            "signaling_url": "ws://127.0.0.1:9999/signaling"
        }))
        .unwrap();
        assert_eq!(config.get_signaling_url(), "ws://127.0.0.1:9999/signaling");
        assert!(!config.get_signaling_url().contains("a-secret-token-value"));
        let value = serde_json::to_value(ClientMessage::Authenticate {
            v: PROTOCOL_VERSION,
            role: "server",
            server_id: "server-1",
            token: "a-secret-token-value",
        })
        .unwrap();
        assert_eq!(value["serverId"], "server-1");
        assert_eq!(value["token"], "a-secret-token-value");
    }

    #[test]
    fn typed_protocol_rejects_unknown_messages_and_mismatches() {
        assert!(
            serde_json::from_value::<CloudMessage>(json!({"v": 1, "type": "session-created"}))
                .is_err()
        );
        let message = serde_json::from_value::<CloudMessage>(json!({
            "v": 1,
            "type": "signal",
            "sessionId": "outer",
            "signal": {"type": "close", "sessionId": "inner"}
        }))
        .unwrap();
        let CloudMessage::Signal {
            session_id, signal, ..
        } = message
        else {
            panic!()
        };
        assert_ne!(session_id, signal.session_id());
    }

    #[test]
    fn answer_and_candidates_match_cloud_wire_format() {
        let message = ClientMessage::Signal {
            v: 1,
            session_id: "session-1",
            signal: ServerSignal::Answer {
                session_id: "session-1",
                description: OutDescription {
                    kind: "answer",
                    sdp: "v=0",
                },
            },
        };
        let value: Value = serde_json::to_value(message).unwrap();
        assert_eq!(value["type"], "signal");
        assert_eq!(value["signal"]["type"], "answer");
        assert_eq!(value["signal"]["description"]["type"], "answer");

        let end = ClientMessage::Signal {
            v: 1,
            session_id: "session-1",
            signal: ServerSignal::IceCandidate {
                session_id: "session-1",
                candidate: None,
            },
        };
        assert!(serde_json::to_value(end).unwrap()["signal"]["candidate"].is_null());
    }

    #[test]
    fn direct_ice_policy_removes_turn_urls() {
        let servers = vec![IceServerWire {
            urls: StringOrStrings::Many(vec![
                "stun:one.example".to_owned(),
                "turn:two.example".to_owned(),
            ]),
            username: None,
            credential: None,
            credential_type: None,
        }];
        let filtered = validate_ice_servers(servers).unwrap();
        assert_eq!(filtered[0].urls, vec!["stun:one.example"]);
    }

    #[tokio::test]
    async fn authenticated_acknowledgement_gates_session_processing() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let broker = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let Message::Text(authentication) = socket.next().await.unwrap().unwrap() else {
                panic!("expected authentication text frame");
            };
            let authentication: Value = serde_json::from_str(&authentication).unwrap();
            assert_eq!(authentication["type"], "authenticate");
            assert_eq!(authentication["serverId"], "server-1");
            assert_eq!(authentication["token"], "existing-secret-token");

            socket
                .send(Message::Text(
                    json!({
                        "v": 1,
                        "type": "session-request",
                        "sessionId": "session-before-auth",
                        "serverId": "server-1",
                        "expiresAt": "2999-01-01T00:00:00Z",
                        "user": {"uid": "user-1"},
                        "iceServers": []
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();
        });

        let peers = PeerRegistry::new();
        let result = run_connection(
            &format!("ws://{address}/api/webrtc/signaling"),
            "server-1",
            "existing-secret-token",
            &peers,
            CancellationToken::new(),
        )
        .await;
        assert!(result
            .unwrap_err()
            .contains("before authentication completed"));
        assert!(!peers.contains_session("session-before-auth").await);
        broker.await.unwrap();
    }
}
