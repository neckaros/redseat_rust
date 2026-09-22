use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Weak,
    },
    time::Duration,
};

use async_trait::async_trait;
use tokio::sync::{broadcast, Mutex};
use webrtc::{
    api::APIBuilder,
    data_channel::{data_channel_state::RTCDataChannelState, RTCDataChannel},
    ice_transport::{
        ice_candidate::{RTCIceCandidate, RTCIceCandidateInit},
        ice_candidate_type::RTCIceCandidateType,
        ice_server::RTCIceServer,
    },
    peer_connection::{
        configuration::RTCConfiguration, peer_connection_state::RTCPeerConnectionState,
        sdp::session_description::RTCSessionDescription, RTCPeerConnection,
    },
};

pub const DATA_CHANNEL_LABEL: &str = "redseat-api-v1";
const DISCONNECTED_PEER_GRACE: Duration = Duration::from_secs(30);
const MAX_ESTABLISHED_PEERS: usize = 64;
const MAX_ESTABLISHED_PEERS_PER_USER: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IceServer {
    pub urls: Vec<String>,
    pub username: String,
    pub credential: String,
}

#[derive(Clone)]
pub struct EstablishedPeer {
    pub session_id: String,
    pub cloud_user_uid: String,
    pub peer_connection: Arc<RTCPeerConnection>,
    pub data_channel: Arc<RTCDataChannel>,
}

/// Boundary between WebRTC negotiation and the future DataChannel API dispatcher.
#[async_trait]
pub trait EstablishedPeerSink: Send + Sync {
    async fn contains_session(&self, session_id: &str) -> bool;
    async fn accept(&self, peer: EstablishedPeer) -> Result<(), String>;
    async fn shutdown(&self);
}

pub struct PeerRegistry {
    peers: Arc<Mutex<HashMap<String, EstablishedPeer>>>,
    established_tx: broadcast::Sender<EstablishedPeer>,
    max_peers: usize,
    max_peers_per_user: usize,
}

impl PeerRegistry {
    pub fn new() -> Self {
        Self::with_limits(MAX_ESTABLISHED_PEERS, MAX_ESTABLISHED_PEERS_PER_USER)
    }

    fn with_limits(max_peers: usize, max_peers_per_user: usize) -> Self {
        let (established_tx, _) = broadcast::channel(64);
        Self {
            peers: Arc::new(Mutex::new(HashMap::new())),
            established_tx,
            max_peers,
            max_peers_per_user,
        }
    }

    /// Subscribe to established peers without coupling a consumer to signaling.
    pub fn subscribe(&self) -> broadcast::Receiver<EstablishedPeer> {
        self.established_tx.subscribe()
    }

    pub async fn get(&self, session_id: &str) -> Option<EstablishedPeer> {
        self.peers.lock().await.get(session_id).cloned()
    }
}

#[async_trait]
impl EstablishedPeerSink for PeerRegistry {
    async fn contains_session(&self, session_id: &str) -> bool {
        self.peers.lock().await.contains_key(session_id)
    }

    async fn accept(&self, peer: EstablishedPeer) -> Result<(), String> {
        {
            let mut peers = self.peers.lock().await;
            if peers.contains_key(&peer.session_id) {
                return Err("peer session is already established".to_owned());
            }
            if peers.len() >= self.max_peers {
                return Err("established peer capacity reached".to_owned());
            }
            if peers
                .values()
                .filter(|existing| existing.cloud_user_uid == peer.cloud_user_uid)
                .count()
                >= self.max_peers_per_user
            {
                return Err("established peer capacity reached for user".to_owned());
            }
            peers.insert(peer.session_id.clone(), peer.clone());
        }

        let session_id = peer.session_id.clone();
        let peers = Arc::downgrade(&self.peers);
        let peer_connection = Arc::downgrade(&peer.peer_connection);
        peer.data_channel.on_close(Box::new(move || {
            let peers = peers.clone();
            let peer_connection = peer_connection.clone();
            let session_id = session_id.clone();
            Box::pin(async move {
                remove_matching_peer(&peers, &session_id, &peer_connection, true).await;
            })
        }));

        let session_id = peer.session_id.clone();
        let peers = Arc::downgrade(&self.peers);
        let peer_connection = Arc::downgrade(&peer.peer_connection);
        let state_generation = Arc::new(AtomicU64::new(0));
        let callback_generation = Arc::clone(&state_generation);
        peer.peer_connection
            .on_peer_connection_state_change(Box::new(move |state| {
                let peers = peers.clone();
                let peer_connection = peer_connection.clone();
                let session_id = session_id.clone();
                let state_generation = Arc::clone(&callback_generation);
                Box::pin(async move {
                    handle_established_peer_state(
                        peers,
                        session_id,
                        peer_connection,
                        state_generation,
                        state,
                        DISCONNECTED_PEER_GRACE,
                    );
                })
            }));

        let connection_state = peer.peer_connection.connection_state();
        if matches!(
            connection_state,
            RTCPeerConnectionState::Failed | RTCPeerConnectionState::Closed
        ) || matches!(
            peer.data_channel.ready_state(),
            RTCDataChannelState::Closing | RTCDataChannelState::Closed
        ) {
            let peers = Arc::downgrade(&self.peers);
            let peer_connection = Arc::downgrade(&peer.peer_connection);
            remove_matching_peer(&peers, &peer.session_id, &peer_connection, true).await;
            return Err("peer closed before it could be registered".to_owned());
        }
        if connection_state == RTCPeerConnectionState::Disconnected {
            handle_established_peer_state(
                Arc::downgrade(&self.peers),
                peer.session_id.clone(),
                Arc::downgrade(&peer.peer_connection),
                state_generation,
                connection_state,
                DISCONNECTED_PEER_GRACE,
            );
        }

        let _ = self.established_tx.send(peer);
        Ok(())
    }

    async fn shutdown(&self) {
        let peers = {
            let mut guard = self.peers.lock().await;
            guard.drain().map(|(_, peer)| peer).collect::<Vec<_>>()
        };
        for peer in peers {
            let _ = peer.peer_connection.close().await;
        }
    }
}

fn handle_established_peer_state(
    peers: Weak<Mutex<HashMap<String, EstablishedPeer>>>,
    session_id: String,
    peer_connection: Weak<RTCPeerConnection>,
    state_generation: Arc<AtomicU64>,
    state: RTCPeerConnectionState,
    disconnect_grace: Duration,
) {
    let generation = state_generation.fetch_add(1, Ordering::SeqCst) + 1;
    match state {
        RTCPeerConnectionState::Disconnected => {
            tokio::spawn(async move {
                tokio::time::sleep(disconnect_grace).await;
                if state_generation.load(Ordering::SeqCst) == generation {
                    remove_matching_peer(&peers, &session_id, &peer_connection, true).await;
                }
            });
        }
        RTCPeerConnectionState::Failed | RTCPeerConnectionState::Closed => {
            // Closing a peer from inside its own state callback can re-enter
            // the callback mutex, so defer cleanup until this callback returns.
            tokio::spawn(async move {
                remove_matching_peer(
                    &peers,
                    &session_id,
                    &peer_connection,
                    state != RTCPeerConnectionState::Closed,
                )
                .await;
            });
        }
        _ => {}
    }
}

fn handle_negotiating_peer_state(
    lifecycle_tx: tokio::sync::mpsc::UnboundedSender<PeerEvent>,
    session_id: String,
    established: Arc<AtomicBool>,
    state_generation: Arc<AtomicU64>,
    state: RTCPeerConnectionState,
    disconnect_grace: Duration,
) {
    let generation = state_generation.fetch_add(1, Ordering::SeqCst) + 1;
    match state {
        RTCPeerConnectionState::Disconnected => {
            tokio::spawn(async move {
                tokio::time::sleep(disconnect_grace).await;
                if state_generation.load(Ordering::SeqCst) == generation
                    && !established.load(Ordering::SeqCst)
                {
                    let _ = lifecycle_tx.send(PeerEvent::Failed {
                        session_id,
                        message: "WebRTC peer negotiation remained disconnected",
                    });
                }
            });
        }
        RTCPeerConnectionState::Failed | RTCPeerConnectionState::Closed => {
            if !established.load(Ordering::SeqCst) {
                let _ = lifecycle_tx.send(PeerEvent::Failed {
                    session_id,
                    message: "WebRTC peer negotiation failed",
                });
            }
        }
        _ => {}
    }
}

async fn remove_matching_peer(
    peers: &Weak<Mutex<HashMap<String, EstablishedPeer>>>,
    session_id: &str,
    peer_connection: &Weak<RTCPeerConnection>,
    close: bool,
) {
    let (Some(peers), Some(peer_connection)) = (peers.upgrade(), peer_connection.upgrade()) else {
        return;
    };
    let removed = {
        let mut peers = peers.lock().await;
        if peers
            .get(session_id)
            .is_some_and(|peer| Arc::ptr_eq(&peer.peer_connection, &peer_connection))
        {
            peers.remove(session_id)
        } else {
            None
        }
    };
    if close {
        if let Some(peer) = removed {
            let _ = peer.peer_connection.close().await;
        }
    }
}

pub enum PeerEvent {
    LocalCandidate {
        session_id: String,
        candidate: Option<RTCIceCandidateInit>,
    },
    Established(EstablishedPeer),
    Failed {
        session_id: String,
        message: &'static str,
    },
}

pub struct PeerSession {
    pub peer_connection: Arc<RTCPeerConnection>,
    established: Arc<AtomicBool>,
}

impl PeerSession {
    pub async fn new(
        session_id: String,
        cloud_user_uid: String,
        ice_servers: Vec<IceServer>,
        candidate_tx: tokio::sync::mpsc::Sender<PeerEvent>,
        lifecycle_tx: tokio::sync::mpsc::UnboundedSender<PeerEvent>,
    ) -> Result<Self, String> {
        let rtc_ice_servers = ice_servers
            .into_iter()
            .filter_map(|server| {
                let urls = server
                    .urls
                    .into_iter()
                    .filter(|url| !is_turn_url(url))
                    .collect::<Vec<_>>();
                (!urls.is_empty()).then_some(RTCIceServer {
                    urls,
                    username: server.username,
                    credential: server.credential,
                })
            })
            .collect();

        let pc = Arc::new(
            APIBuilder::new()
                .build()
                .new_peer_connection(RTCConfiguration {
                    ice_servers: rtc_ice_servers,
                    ..Default::default()
                })
                .await
                .map_err(|error| format!("unable to create peer: {error}"))?,
        );
        let established = Arc::new(AtomicBool::new(false));

        let candidate_session_id = session_id.clone();
        let candidate_lifecycle_tx = lifecycle_tx.clone();
        pc.on_ice_candidate(Box::new(move |candidate: Option<RTCIceCandidate>| {
            let tx = candidate_tx.clone();
            let lifecycle_tx = candidate_lifecycle_tx.clone();
            let session_id = candidate_session_id.clone();
            Box::pin(async move {
                let candidate = match candidate {
                    Some(candidate)
                        if matches!(
                            candidate.typ,
                            RTCIceCandidateType::Host | RTCIceCandidateType::Srflx
                        ) =>
                    {
                        candidate.to_json().ok()
                    }
                    Some(_) => return,
                    None => None,
                };
                if tx
                    .try_send(PeerEvent::LocalCandidate {
                        session_id: session_id.clone(),
                        candidate,
                    })
                    .is_err()
                {
                    let _ = lifecycle_tx.send(PeerEvent::Failed {
                        session_id,
                        message: "too many local ICE candidates",
                    });
                }
            })
        }));

        let channel_session_id = session_id.clone();
        let channel_uid = cloud_user_uid;
        let channel_pc = Arc::downgrade(&pc);
        let channel_tx = lifecycle_tx.clone();
        let channel_established = Arc::clone(&established);
        pc.on_data_channel(Box::new(move |channel: Arc<RTCDataChannel>| {
            let session_id = channel_session_id.clone();
            let cloud_user_uid = channel_uid.clone();
            let pc = channel_pc.clone();
            let tx = channel_tx.clone();
            let established = Arc::clone(&channel_established);
            Box::pin(async move {
                if !valid_data_channel(&channel) {
                    let _ = tx.send(PeerEvent::Failed {
                        session_id,
                        message: "invalid RedSeat data channel",
                    });
                    let _ = channel.close().await;
                    if let Some(pc) = pc.upgrade() {
                        let _ = pc.close().await;
                    }
                    return;
                }

                let opened_channel = Arc::downgrade(&channel);
                channel.on_open(Box::new(move || {
                    let tx = tx.clone();
                    let session_id = session_id.clone();
                    let cloud_user_uid = cloud_user_uid.clone();
                    let pc = pc.clone();
                    let channel = opened_channel.clone();
                    let established = Arc::clone(&established);
                    Box::pin(async move {
                        let Some(channel) = channel.upgrade() else {
                            return;
                        };
                        if established.swap(true, Ordering::SeqCst) {
                            let _ = channel.close().await;
                            return;
                        }
                        let Some(pc) = pc.upgrade() else {
                            let _ = channel.close().await;
                            return;
                        };
                        let _ = tx.send(PeerEvent::Established(EstablishedPeer {
                            session_id,
                            cloud_user_uid,
                            peer_connection: pc,
                            data_channel: channel,
                        }));
                    })
                }));
            })
        }));

        let state_session_id = session_id;
        let state_tx = lifecycle_tx;
        let state_established = Arc::clone(&established);
        let state_generation = Arc::new(AtomicU64::new(0));
        pc.on_peer_connection_state_change(Box::new(move |state: RTCPeerConnectionState| {
            let tx = state_tx.clone();
            let session_id = state_session_id.clone();
            let established = Arc::clone(&state_established);
            let state_generation = Arc::clone(&state_generation);
            Box::pin(async move {
                handle_negotiating_peer_state(
                    tx,
                    session_id,
                    established,
                    state_generation,
                    state,
                    DISCONNECTED_PEER_GRACE,
                );
            })
        }));

        Ok(Self {
            peer_connection: pc,
            established,
        })
    }

    pub fn is_established(&self) -> bool {
        self.established.load(Ordering::SeqCst)
    }

    pub async fn accept_offer(&self, sdp: String) -> Result<String, String> {
        validate_direct_sdp_candidates(&sdp)?;
        let offer = RTCSessionDescription::offer(sdp)
            .map_err(|error| format!("invalid WebRTC offer: {error}"))?;
        self.peer_connection
            .set_remote_description(offer)
            .await
            .map_err(|error| format!("unable to install WebRTC offer: {error}"))?;
        let answer = self
            .peer_connection
            .create_answer(None)
            .await
            .map_err(|error| format!("unable to create WebRTC answer: {error}"))?;
        self.peer_connection
            .set_local_description(answer)
            .await
            .map_err(|error| format!("unable to install WebRTC answer: {error}"))?;
        self.peer_connection
            .local_description()
            .await
            .map(|description| description.sdp)
            .ok_or_else(|| "WebRTC answer was not available".to_owned())
    }

    pub async fn add_remote_candidate(
        &self,
        candidate: Option<RTCIceCandidateInit>,
    ) -> Result<(), String> {
        let candidate = candidate.unwrap_or_default();
        if !is_direct_candidate(&candidate.candidate) {
            return Ok(());
        }
        self.peer_connection
            .add_ice_candidate(candidate)
            .await
            .map_err(|error| format!("unable to add remote ICE candidate: {error}"))
    }

    pub async fn close(&self) {
        let _ = self.peer_connection.close().await;
    }
}

pub fn is_turn_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("turn:") || lower.starts_with("turns:")
}

pub fn is_relay_candidate(candidate: &str) -> bool {
    candidate_type(candidate).is_some_and(|kind| kind.eq_ignore_ascii_case("relay"))
}

pub fn is_direct_candidate(candidate: &str) -> bool {
    if candidate.is_empty() {
        return true;
    }
    candidate_type(candidate)
        .is_some_and(|kind| kind.eq_ignore_ascii_case("host") || kind.eq_ignore_ascii_case("srflx"))
}

fn candidate_type(candidate: &str) -> Option<&str> {
    let fields = candidate.split_ascii_whitespace().collect::<Vec<_>>();
    fields
        .windows(2)
        .find(|pair| pair[0].eq_ignore_ascii_case("typ"))
        .map(|pair| pair[1])
}

fn validate_direct_sdp_candidates(sdp: &str) -> Result<(), String> {
    for line in sdp.split(['\r', '\n']).map(str::trim) {
        let Some(prefix) = line.get(.."a=candidate:".len()) else {
            continue;
        };
        if prefix.eq_ignore_ascii_case("a=candidate:") && !is_direct_candidate(&line["a=".len()..])
        {
            return Err("WebRTC offer contains a non-direct ICE candidate".to_owned());
        }
    }
    Ok(())
}

fn valid_data_channel(channel: &RTCDataChannel) -> bool {
    channel.label() == DATA_CHANNEL_LABEL
        && !channel.ordered()
        && channel.max_retransmits().is_none()
        && channel.max_packet_lifetime().is_none()
        && !channel.negotiated()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use webrtc::{
        api::APIBuilder,
        data_channel::data_channel_init::RTCDataChannelInit,
        peer_connection::{
            configuration::RTCConfiguration, sdp::session_description::RTCSessionDescription,
        },
    };

    async fn unopened_peer(session_id: &str, cloud_user_uid: &str) -> EstablishedPeer {
        let peer_connection = Arc::new(
            APIBuilder::new()
                .build()
                .new_peer_connection(RTCConfiguration::default())
                .await
                .unwrap(),
        );
        let data_channel = peer_connection
            .create_data_channel(
                DATA_CHANNEL_LABEL,
                Some(RTCDataChannelInit {
                    ordered: Some(false),
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
        EstablishedPeer {
            session_id: session_id.to_owned(),
            cloud_user_uid: cloud_user_uid.to_owned(),
            peer_connection,
            data_channel,
        }
    }

    #[test]
    fn filters_turn_and_relay_candidates_case_insensitively() {
        assert!(is_turn_url("TURN:relay.example"));
        assert!(is_turn_url("turns:relay.example"));
        assert!(!is_turn_url("stun:stun.example"));
        assert!(is_relay_candidate(
            "candidate:1 1 udp 1 203.0.113.1 1234 TyP ReLaY raddr 0.0.0.0"
        ));
        assert!(!is_relay_candidate(
            "candidate:1 1 udp 1 192.0.2.1 1234 typ srflx"
        ));
        assert!(is_direct_candidate(
            "candidate:1 1 udp 1 192.0.2.1 1234 typ host"
        ));
        assert!(!is_direct_candidate(
            "candidate:1 1 udp 1 192.0.2.1 1234 typ prflx"
        ));
    }

    #[test]
    fn rejects_embedded_relay_and_peer_reflexive_sdp_candidates() {
        let direct = concat!(
            "v=0\r\n",
            "a=candidate:1 1 UDP 1 192.0.2.1 5000 typ host\r\n",
            "a=candidate:2 1 UDP 1 198.51.100.1 5001 typ srflx\r\n"
        );
        assert!(validate_direct_sdp_candidates(direct).is_ok());

        let relay = concat!(
            "v=0\r\n",
            "a=candidate:3 1 UDP 1 203.0.113.1 5002 typ relay raddr 0.0.0.0 rport 0\r\n"
        );
        assert!(validate_direct_sdp_candidates(relay).is_err());

        let peer_reflexive = concat!(
            "v=0\r\n",
            "a=candidate:4 1 UDP 1 203.0.113.2 5003 typ prflx\r\n"
        );
        assert!(validate_direct_sdp_candidates(peer_reflexive).is_err());
    }

    #[tokio::test]
    async fn browser_compatible_data_channel_reaches_handoff() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (candidate_tx, mut candidate_rx) = tokio::sync::mpsc::channel(32);
        let (lifecycle_tx, mut lifecycle_rx) = tokio::sync::mpsc::unbounded_channel();
        let answerer = PeerSession::new(
            "session-1".to_owned(),
            "cloud-user-1".to_owned(),
            Vec::new(),
            candidate_tx,
            lifecycle_tx,
        )
        .await
        .unwrap();

        let offerer = Arc::new(
            APIBuilder::new()
                .build()
                .new_peer_connection(RTCConfiguration::default())
                .await
                .unwrap(),
        );
        let channel = offerer
            .create_data_channel(
                DATA_CHANNEL_LABEL,
                Some(RTCDataChannelInit {
                    ordered: Some(false),
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
        let duplicate_channel = offerer
            .create_data_channel(
                DATA_CHANNEL_LABEL,
                Some(RTCDataChannelInit {
                    ordered: Some(false),
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
        assert!(!channel.ordered());
        assert_eq!(channel.max_retransmits(), None);
        assert_eq!(channel.max_packet_lifetime(), None);

        let mut gathering_complete = offerer.gathering_complete_promise().await;
        let offer = offerer.create_offer(None).await.unwrap();
        offerer.set_local_description(offer).await.unwrap();
        let _ = gathering_complete.recv().await;
        let offer = offerer.local_description().await.unwrap();

        let answer = answerer.accept_offer(offer.sdp).await.unwrap();
        offerer
            .set_remote_description(RTCSessionDescription::answer(answer).unwrap())
            .await
            .unwrap();

        let established = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                tokio::select! {
                    event = candidate_rx.recv() => {
                        let PeerEvent::LocalCandidate { candidate, .. } = event.unwrap() else {
                            panic!("unexpected lifecycle event in candidate queue");
                        };
                        offerer.add_ice_candidate(candidate.unwrap_or_default()).await.unwrap();
                    }
                    event = lifecycle_rx.recv() => match event.unwrap() {
                        PeerEvent::Established(peer) => break peer,
                        PeerEvent::Failed { message, .. } => panic!("{message}"),
                        PeerEvent::LocalCandidate { .. } => {
                            panic!("unexpected candidate in lifecycle queue")
                        }
                    }
                }
            }
        })
        .await
        .expect("data channel did not open");

        assert_eq!(established.session_id, "session-1");
        assert_eq!(established.cloud_user_uid, "cloud-user-1");
        assert_eq!(established.data_channel.label(), DATA_CHANNEL_LABEL);
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let states = [channel.ready_state(), duplicate_channel.ready_state()];
                if states
                    .iter()
                    .filter(|state| **state == RTCDataChannelState::Open)
                    .count()
                    == 1
                    && states
                        .iter()
                        .filter(|state| **state == RTCDataChannelState::Closed)
                        .count()
                        == 1
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("duplicate data channel was not closed");

        let registry = PeerRegistry::new();
        registry.accept(established.clone()).await.unwrap();
        assert!(registry.get("session-1").await.is_some());
        established.data_channel.close().await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while registry.get("session-1").await.is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("closed data channel remained in the peer registry");
        offerer.close().await.unwrap();
    }

    #[tokio::test]
    async fn disconnected_peer_is_removed_only_after_a_resettable_grace_period() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let peer_connection = Arc::new(
            APIBuilder::new()
                .build()
                .new_peer_connection(RTCConfiguration::default())
                .await
                .unwrap(),
        );
        let data_channel = peer_connection
            .create_data_channel(
                DATA_CHANNEL_LABEL,
                Some(RTCDataChannelInit {
                    ordered: Some(false),
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
        let registry = PeerRegistry::new();
        registry.peers.lock().await.insert(
            "session-grace".to_owned(),
            EstablishedPeer {
                session_id: "session-grace".to_owned(),
                cloud_user_uid: "cloud-user-1".to_owned(),
                peer_connection: Arc::clone(&peer_connection),
                data_channel,
            },
        );
        let state_generation = Arc::new(AtomicU64::new(0));
        let grace = Duration::from_millis(25);

        handle_established_peer_state(
            Arc::downgrade(&registry.peers),
            "session-grace".to_owned(),
            Arc::downgrade(&peer_connection),
            Arc::clone(&state_generation),
            RTCPeerConnectionState::Disconnected,
            grace,
        );
        handle_established_peer_state(
            Arc::downgrade(&registry.peers),
            "session-grace".to_owned(),
            Arc::downgrade(&peer_connection),
            Arc::clone(&state_generation),
            RTCPeerConnectionState::Connected,
            grace,
        );
        tokio::time::sleep(grace * 2).await;
        assert!(registry.get("session-grace").await.is_some());

        handle_established_peer_state(
            Arc::downgrade(&registry.peers),
            "session-grace".to_owned(),
            Arc::downgrade(&peer_connection),
            state_generation,
            RTCPeerConnectionState::Disconnected,
            grace,
        );
        tokio::time::timeout(grace * 4, async {
            while registry.get("session-grace").await.is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("disconnected peer remained after its grace period");
    }

    #[tokio::test]
    async fn negotiating_peer_uses_a_resettable_disconnection_grace_period() {
        let (lifecycle_tx, mut lifecycle_rx) = tokio::sync::mpsc::unbounded_channel();
        let established = Arc::new(AtomicBool::new(false));
        let state_generation = Arc::new(AtomicU64::new(0));
        let grace = Duration::from_millis(25);

        handle_negotiating_peer_state(
            lifecycle_tx.clone(),
            "session-negotiating".to_owned(),
            Arc::clone(&established),
            Arc::clone(&state_generation),
            RTCPeerConnectionState::Disconnected,
            grace,
        );
        handle_negotiating_peer_state(
            lifecycle_tx.clone(),
            "session-negotiating".to_owned(),
            Arc::clone(&established),
            Arc::clone(&state_generation),
            RTCPeerConnectionState::Connected,
            grace,
        );
        assert!(tokio::time::timeout(grace * 2, lifecycle_rx.recv())
            .await
            .is_err());

        handle_negotiating_peer_state(
            lifecycle_tx,
            "session-negotiating".to_owned(),
            established,
            state_generation,
            RTCPeerConnectionState::Disconnected,
            grace,
        );
        let event = tokio::time::timeout(grace * 4, lifecycle_rx.recv())
            .await
            .expect("negotiating peer did not fail after its grace period")
            .expect("negotiating peer lifecycle channel closed");
        assert!(matches!(
            event,
            PeerEvent::Failed {
                session_id,
                message: "WebRTC peer negotiation remained disconnected",
            } if session_id == "session-negotiating"
        ));
    }

    #[tokio::test]
    async fn established_peer_quotas_are_atomic_per_server_and_user() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let registry = PeerRegistry::with_limits(2, 1);
        let first = unopened_peer("session-a1", "user-a").await;
        let first_pc = Arc::clone(&first.peer_connection);
        let same_user = unopened_peer("session-a2", "user-a").await;
        let same_user_pc = Arc::clone(&same_user.peer_connection);
        let (first_result, same_user_result) =
            tokio::join!(registry.accept(first), registry.accept(same_user));
        assert_ne!(first_result.is_ok(), same_user_result.is_ok());
        if let Err(error) = first_result {
            assert_eq!(error, "established peer capacity reached for user");
            first_pc.close().await.unwrap();
        }
        if let Err(error) = same_user_result {
            assert_eq!(error, "established peer capacity reached for user");
            same_user_pc.close().await.unwrap();
        }

        let second = unopened_peer("session-b1", "user-b").await;
        registry.accept(second).await.unwrap();
        let over_server_limit = unopened_peer("session-c1", "user-c").await;
        let over_server_limit_pc = Arc::clone(&over_server_limit.peer_connection);
        assert_eq!(
            registry.accept(over_server_limit).await.unwrap_err(),
            "established peer capacity reached"
        );
        over_server_limit_pc.close().await.unwrap();

        assert_ne!(
            registry.get("session-a1").await.is_some(),
            registry.get("session-a2").await.is_some()
        );
        assert!(registry.get("session-b1").await.is_some());
        assert!(registry.get("session-c1").await.is_none());
        registry.shutdown().await;
    }

    #[tokio::test]
    async fn closing_with_a_full_candidate_queue_does_not_deadlock_or_retain_peer() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (candidate_tx, _candidate_rx) = tokio::sync::mpsc::channel(1);
        candidate_tx
            .try_send(PeerEvent::LocalCandidate {
                session_id: "occupied".to_owned(),
                candidate: None,
            })
            .unwrap();
        let (lifecycle_tx, _lifecycle_rx) = tokio::sync::mpsc::unbounded_channel();
        let session = PeerSession::new(
            "session-full".to_owned(),
            "cloud-user-1".to_owned(),
            Vec::new(),
            candidate_tx,
            lifecycle_tx,
        )
        .await
        .unwrap();
        let peer = Arc::downgrade(&session.peer_connection);

        tokio::time::timeout(Duration::from_secs(2), session.close())
            .await
            .expect("peer close blocked on a full event queue");
        drop(session);
        tokio::time::timeout(Duration::from_secs(2), async {
            while peer.upgrade().is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("closed pending peer was retained by one of its callbacks");
    }

    #[tokio::test]
    async fn relay_candidate_in_offer_is_rejected_before_installation() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (candidate_tx, _candidate_rx) = tokio::sync::mpsc::channel(1);
        let (lifecycle_tx, _lifecycle_rx) = tokio::sync::mpsc::unbounded_channel();
        let session = PeerSession::new(
            "session-relay".to_owned(),
            "cloud-user-1".to_owned(),
            Vec::new(),
            candidate_tx,
            lifecycle_tx,
        )
        .await
        .unwrap();
        let offer = concat!(
            "v=0\r\n",
            "a=candidate:3 1 UDP 1 203.0.113.1 5002 typ relay raddr 0.0.0.0 rport 0\r\n"
        );

        assert!(session.accept_offer(offer.to_owned()).await.is_err());
        assert!(session.peer_connection.remote_description().await.is_none());
        session.close().await;
    }
}
