use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use async_trait::async_trait;
use tokio::sync::{broadcast, Mutex};
use webrtc::{
    api::APIBuilder,
    data_channel::RTCDataChannel,
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
    peers: Mutex<HashMap<String, EstablishedPeer>>,
    established_tx: broadcast::Sender<EstablishedPeer>,
}

impl PeerRegistry {
    pub fn new() -> Self {
        let (established_tx, _) = broadcast::channel(64);
        Self {
            peers: Mutex::new(HashMap::new()),
            established_tx,
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
        let mut peers = self.peers.lock().await;
        if peers.contains_key(&peer.session_id) {
            return Err("peer session is already established".to_owned());
        }
        peers.insert(peer.session_id.clone(), peer.clone());
        drop(peers);
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
        event_tx: tokio::sync::mpsc::Sender<PeerEvent>,
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
        let candidate_tx = event_tx.clone();
        pc.on_ice_candidate(Box::new(move |candidate: Option<RTCIceCandidate>| {
            let tx = candidate_tx.clone();
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
                let _ = tx
                    .send(PeerEvent::LocalCandidate {
                        session_id,
                        candidate,
                    })
                    .await;
            })
        }));

        let channel_session_id = session_id.clone();
        let channel_uid = cloud_user_uid;
        let channel_pc = Arc::clone(&pc);
        let channel_tx = event_tx.clone();
        let channel_established = Arc::clone(&established);
        pc.on_data_channel(Box::new(move |channel: Arc<RTCDataChannel>| {
            let session_id = channel_session_id.clone();
            let cloud_user_uid = channel_uid.clone();
            let pc = Arc::clone(&channel_pc);
            let tx = channel_tx.clone();
            let established = Arc::clone(&channel_established);
            Box::pin(async move {
                if !valid_data_channel(&channel) {
                    let _ = tx
                        .send(PeerEvent::Failed {
                            session_id,
                            message: "invalid RedSeat data channel",
                        })
                        .await;
                    let _ = channel.close().await;
                    let _ = pc.close().await;
                    return;
                }

                let opened_channel = Arc::clone(&channel);
                channel.on_open(Box::new(move || {
                    let tx = tx.clone();
                    let session_id = session_id.clone();
                    let cloud_user_uid = cloud_user_uid.clone();
                    let pc = Arc::clone(&pc);
                    let channel = Arc::clone(&opened_channel);
                    let established = Arc::clone(&established);
                    Box::pin(async move {
                        if established.swap(true, Ordering::SeqCst) {
                            return;
                        }
                        let _ = tx
                            .send(PeerEvent::Established(EstablishedPeer {
                                session_id,
                                cloud_user_uid,
                                peer_connection: pc,
                                data_channel: channel,
                            }))
                            .await;
                    })
                }));
            })
        }));

        let state_session_id = session_id;
        let state_tx = event_tx;
        let state_established = Arc::clone(&established);
        pc.on_peer_connection_state_change(Box::new(move |state: RTCPeerConnectionState| {
            let tx = state_tx.clone();
            let session_id = state_session_id.clone();
            let established = Arc::clone(&state_established);
            Box::pin(async move {
                if !established.load(Ordering::SeqCst)
                    && matches!(
                        state,
                        RTCPeerConnectionState::Disconnected
                            | RTCPeerConnectionState::Failed
                            | RTCPeerConnectionState::Closed
                    )
                {
                    let _ = tx
                        .send(PeerEvent::Failed {
                            session_id,
                            message: "WebRTC peer negotiation failed",
                        })
                        .await;
                }
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
        if is_relay_candidate(&candidate.candidate) {
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

    #[tokio::test]
    async fn browser_compatible_data_channel_reaches_handoff() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(32);
        let answerer = PeerSession::new(
            "session-1".to_owned(),
            "cloud-user-1".to_owned(),
            Vec::new(),
            event_tx,
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
                match event_rx.recv().await.unwrap() {
                    PeerEvent::LocalCandidate { candidate, .. } => {
                        offerer
                            .add_ice_candidate(candidate.unwrap_or_default())
                            .await
                            .unwrap();
                    }
                    PeerEvent::Established(peer) => break peer,
                    PeerEvent::Failed { message, .. } => panic!("{message}"),
                }
            }
        })
        .await
        .expect("data channel did not open");

        assert_eq!(established.session_id, "session-1");
        assert_eq!(established.cloud_user_uid, "cloud-user-1");
        assert_eq!(established.data_channel.label(), DATA_CHANNEL_LABEL);
        established.peer_connection.close().await.unwrap();
        offerer.close().await.unwrap();
    }
}
