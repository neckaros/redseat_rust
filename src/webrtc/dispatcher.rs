use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    io::SeekFrom,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use async_trait::async_trait;
use axum::{
    body::{Body, Bytes, HttpBody},
    http::{header, HeaderMap, HeaderName, HeaderValue, Method, Request, Response, Uri},
    Router,
};
use futures::{Stream, StreamExt, TryStreamExt};
use http_body_util::BodyExt;
use nanoid::nanoid;
use serde::Deserialize;
use serde_json::Value;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    sync::{mpsc, Mutex, Semaphore},
};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::{
    io::{ReaderStream, StreamReader},
    sync::CancellationToken,
};
use tower::ServiceExt;
use webrtc::data_channel::{
    data_channel_message::DataChannelMessage, data_channel_state::RTCDataChannelState,
    RTCDataChannel,
};

use crate::tools::log::{log_error, LogServiceType};

use super::{
    peer::{EstablishedPeer, EstablishedPeerSink, PeerRegistry},
    protocol::{
        decode_binary, decode_control, encode_binary, encode_control, max_chunk_size,
        validate_identifier, CancelTarget, IncomingControl, OutgoingControl, PayloadDescriptor,
        PayloadEncoding, RequestFrame, SubscribeFrame, WireError, MAX_MESSAGE_SIZE, WIRE_VERSION,
    },
};

const MAX_ACTIVE_REQUESTS: usize = 64;
const MAX_ACTIVE_SUBSCRIPTIONS: usize = 16;
const MAX_PENDING_PAYLOADS: usize = 128;
const MAX_TRANSFER_SIZE: u64 = 8 * 1024 * 1024 * 1024 * 1024;
const MAX_LEGACY_RESPONSE_SIZE: u64 = 256 * 1024 * 1024;
const MAX_PEER_SPOOL_BYTES: u64 = 512 * 1024 * 1024;
const MAX_SERVER_SPOOL_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const RESPONSE_SEGMENT_SIZE: usize = 1024 * 1024;
const REQUEST_BODY_QUEUE: usize = 16;
const MAX_RESPONSE_CREDITS: usize = 16;
const MAX_REORDERED_CHUNKS: usize = 4096;
const REORDER_SLOT_SIZE: u64 = MAX_MESSAGE_SIZE as u64;
const MAX_FORM_MANIFEST_SIZE: usize = 1024 * 1024;
const MAX_FORM_ENTRIES: usize = 4096;
const MAX_EVENT_SIZE: usize = 8 * 1024 * 1024;
const MAX_SEEN_IDENTIFIERS: usize = 8192;
const MAX_HEADERS: usize = 128;
const MAX_HEADER_BYTES: usize = 16 * 1024;
const PAYLOAD_INACTIVITY: Duration = Duration::from_secs(60);
const BUFFERED_AMOUNT_HIGH: usize = 8 * 1024 * 1024;
const BUFFERED_AMOUNT_LOW: usize = 4 * 1024 * 1024;

pub struct DispatcherSink {
    router: Router,
    registry: Arc<PeerRegistry>,
    server_spool: Arc<DiskBudget>,
}

impl DispatcherSink {
    pub fn new(router: Router, registry: Arc<PeerRegistry>) -> Self {
        Self {
            router,
            registry,
            server_spool: Arc::new(DiskBudget::new(MAX_SERVER_SPOOL_BYTES)),
        }
    }
}

#[async_trait]
impl EstablishedPeerSink for DispatcherSink {
    async fn contains_session(&self, session_id: &str) -> bool {
        self.registry.contains_session(session_id).await
    }

    async fn accept(&self, peer: EstablishedPeer) -> Result<(), String> {
        let incoming = peer.take_incoming().await?;
        let stream_channels = peer.take_stream_channels().await;
        self.registry.accept(peer.clone()).await?;
        let router = self.router.clone();
        let budgets = SpoolBudgets {
            peer: Arc::new(DiskBudget::new(MAX_PEER_SPOOL_BYTES)),
            server: Arc::clone(&self.server_spool),
        };
        if let Some(mut stream_channels) = stream_channels {
            // Each bulk-transfer channel gets its own dispatcher (and so its own
            // send buffer and backpressure) sharing the peer's spool budget.
            let router = router.clone();
            let budgets = budgets.clone();
            let parent = peer.clone();
            tokio::spawn(async move {
                let closed = parent.closed();
                loop {
                    let stream = tokio::select! {
                        _ = closed.cancelled() => break,
                        stream = stream_channels.recv() => stream,
                    };
                    let Some(stream) = stream else { break };
                    let stream_peer = stream.into_peer(&parent);
                    let Ok(incoming) = stream_peer.take_incoming().await else {
                        continue;
                    };
                    let router = router.clone();
                    let budgets = budgets.clone();
                    tokio::spawn(async move {
                        PeerDispatcher::new(router, stream_peer, incoming, budgets)
                            .run()
                            .await;
                    });
                }
            });
        }
        tokio::spawn(async move {
            PeerDispatcher::new(router, peer, incoming, budgets)
                .run()
                .await;
        });
        Ok(())
    }

    async fn shutdown(&self) {
        self.registry.shutdown().await;
    }
}

struct DiskBudget {
    used: AtomicU64,
    limit: u64,
}

impl DiskBudget {
    fn new(limit: u64) -> Self {
        Self {
            used: AtomicU64::new(0),
            limit,
        }
    }

    fn reserve(&self, bytes: u64) -> bool {
        let mut used = self.used.load(Ordering::Relaxed);
        loop {
            let Some(next) = used.checked_add(bytes) else {
                return false;
            };
            if next > self.limit {
                return false;
            }
            match self
                .used
                .compare_exchange_weak(used, next, Ordering::AcqRel, Ordering::Relaxed)
            {
                Ok(_) => return true,
                Err(actual) => used = actual,
            }
        }
    }

    fn release(&self, bytes: u64) {
        self.used.fetch_sub(bytes, Ordering::AcqRel);
    }
}

#[derive(Clone)]
struct SpoolBudgets {
    peer: Arc<DiskBudget>,
    server: Arc<DiskBudget>,
}

struct DiskReservation {
    budgets: SpoolBudgets,
    bytes: u64,
}

impl DiskReservation {
    fn new(budgets: SpoolBudgets) -> Self {
        Self { budgets, bytes: 0 }
    }

    fn grow(&mut self, bytes: u64) -> Result<(), String> {
        if !self.budgets.peer.reserve(bytes) {
            return Err("peer payload spool limit exceeded".to_owned());
        }
        if !self.budgets.server.reserve(bytes) {
            self.budgets.peer.release(bytes);
            return Err("server payload spool limit exceeded".to_owned());
        }
        self.bytes += bytes;
        Ok(())
    }

    fn shrink(&mut self, bytes: u64) {
        debug_assert!(bytes <= self.bytes);
        self.bytes -= bytes;
        self.budgets.peer.release(bytes);
        self.budgets.server.release(bytes);
    }
}

struct ReorderSpool {
    file: ReservedFile,
    free_slots: BTreeSet<u64>,
    slots: u64,
    /// Bytes reserved per slot: the largest chunk written to it. Slots are
    /// sized for the largest message, but the file is sparse, so a 16 KiB chunk
    /// from a peer using the base message size only reserves 16 KiB.
    slot_bytes: Vec<u64>,
}

impl ReorderSpool {
    fn new(budgets: SpoolBudgets) -> Result<Self, String> {
        Ok(Self {
            file: ReservedFile::new(budgets)?,
            free_slots: BTreeSet::new(),
            slots: 0,
            slot_bytes: Vec::new(),
        })
    }

    async fn write(&mut self, bytes: &[u8]) -> Result<u64, String> {
        if bytes.len() as u64 > REORDER_SLOT_SIZE {
            return Err("payload chunk exceeds the spool slot size".to_owned());
        }
        let slot = if let Some(slot) = self.free_slots.pop_first() {
            slot
        } else {
            let slot = self.slots;
            self.slots += 1;
            self.slot_bytes.push(0);
            self.file
                .file
                .set_len(self.slots * REORDER_SLOT_SIZE)
                .await
                .map_err(|_| "unable to grow payload spool file".to_owned())?;
            slot
        };
        let held = &mut self.slot_bytes[slot as usize];
        if bytes.len() as u64 > *held {
            self.file.reservation.grow(bytes.len() as u64 - *held)?;
            *held = bytes.len() as u64;
        }
        self.file
            .file
            .seek(SeekFrom::Start(slot * REORDER_SLOT_SIZE))
            .await
            .map_err(|_| "unable to seek in payload spool file".to_owned())?;
        self.file
            .file
            .write_all(bytes)
            .await
            .map_err(|_| "unable to write payload spool file".to_owned())?;
        Ok(slot)
    }

    async fn read_and_free(&mut self, slot: u64, length: usize) -> Result<Bytes, String> {
        self.file
            .file
            .seek(SeekFrom::Start(slot * REORDER_SLOT_SIZE))
            .await
            .map_err(|_| "unable to seek in payload spool file".to_owned())?;
        let mut bytes = vec![0; length];
        self.file
            .file
            .read_exact(&mut bytes)
            .await
            .map_err(|_| "unable to read payload spool file".to_owned())?;
        self.free_slots.insert(slot);

        let old_slots = self.slots;
        while self.slots > 0 && self.free_slots.remove(&(self.slots - 1)) {
            self.slots -= 1;
        }
        if self.slots != old_slots {
            self.file
                .file
                .set_len(self.slots * REORDER_SLOT_SIZE)
                .await
                .map_err(|_| "unable to reclaim payload spool file".to_owned())?;
            let released = self.slot_bytes.drain(self.slots as usize..).sum::<u64>();
            self.file.reservation.shrink(released);
        }
        Ok(Bytes::from(bytes))
    }
}

impl Drop for DiskReservation {
    fn drop(&mut self) {
        self.budgets.peer.release(self.bytes);
        self.budgets.server.release(self.bytes);
    }
}

struct ReservedFile {
    file: File,
    reservation: DiskReservation,
}

impl ReservedFile {
    fn new(budgets: SpoolBudgets) -> Result<Self, String> {
        let file = tempfile::tempfile()
            .map(File::from_std)
            .map_err(|_| "unable to create a payload spool file".to_owned())?;
        Ok(Self {
            file,
            reservation: DiskReservation::new(budgets),
        })
    }

    async fn append(&mut self, bytes: &[u8]) -> Result<u64, String> {
        self.reservation.grow(bytes.len() as u64)?;
        let offset = self
            .file
            .seek(SeekFrom::End(0))
            .await
            .map_err(|_| "unable to seek in payload spool file".to_owned())?;
        self.file
            .write_all(bytes)
            .await
            .map_err(|_| "unable to write payload spool file".to_owned())?;
        Ok(offset)
    }
}

#[derive(Clone)]
struct DataChannelSender {
    channel: Arc<RTCDataChannel>,
    closed: CancellationToken,
    message_lock: Arc<Mutex<()>>,
}

impl DataChannelSender {
    async fn send_control(
        &self,
        frame: &OutgoingControl<'_>,
        cancellation: &CancellationToken,
    ) -> Result<(), String> {
        self.send_text(encode_control(frame)?, cancellation).await
    }

    async fn send_text(
        &self,
        value: String,
        cancellation: &CancellationToken,
    ) -> Result<(), String> {
        if value.len() > MAX_MESSAGE_SIZE {
            return Err("outgoing control frame is too large".to_owned());
        }
        self.wait_for_backpressure(cancellation).await?;
        let _guard = self.message_lock.lock().await;
        self.ensure_open(cancellation)?;
        self.channel
            .send_text(value)
            .await
            .map_err(|_| "unable to send DataChannel control frame".to_owned())?;
        Ok(())
    }

    async fn send_binary(
        &self,
        value: Vec<u8>,
        cancellation: &CancellationToken,
    ) -> Result<(), String> {
        if value.len() > MAX_MESSAGE_SIZE {
            return Err("outgoing binary frame is too large".to_owned());
        }
        self.wait_for_backpressure(cancellation).await?;
        let _guard = self.message_lock.lock().await;
        self.ensure_open(cancellation)?;
        self.channel
            .send(&Bytes::from(value))
            .await
            .map_err(|_| "unable to send DataChannel binary frame".to_owned())?;
        Ok(())
    }

    fn ensure_open(&self, cancellation: &CancellationToken) -> Result<(), String> {
        if cancellation.is_cancelled() {
            return Err("operation was cancelled".to_owned());
        }
        if self.closed.is_cancelled() || self.channel.ready_state() != RTCDataChannelState::Open {
            return Err("DataChannel is closed".to_owned());
        }
        Ok(())
    }

    async fn wait_for_backpressure(&self, cancellation: &CancellationToken) -> Result<(), String> {
        let mut draining = false;
        loop {
            self.ensure_open(cancellation)?;
            if !is_backpressured(self.channel.buffered_amount().await, draining) {
                return Ok(());
            }
            draining = true;
            tokio::select! {
                _ = cancellation.cancelled() => return Err("operation was cancelled".to_owned()),
                _ = self.closed.cancelled() => return Err("DataChannel is closed".to_owned()),
                _ = tokio::time::sleep(Duration::from_millis(20)) => {}
            }
        }
    }
}

fn is_backpressured(buffered_amount: usize, draining: bool) -> bool {
    buffered_amount
        > if draining {
            BUFFERED_AMOUNT_LOW
        } else {
            BUFFERED_AMOUNT_HIGH
        }
}

struct RequestState {
    frame: Option<RequestFrame>,
    cancellation: CancellationToken,
    payload_id: Option<String>,
    started: bool,
    response_flow: Arc<ResponseFlow>,
}

struct ResponseFlow {
    semaphore: Semaphore,
    credits: AtomicUsize,
    window_exceeded: AtomicBool,
}

impl ResponseFlow {
    fn new() -> Self {
        Self {
            semaphore: Semaphore::new(0),
            credits: AtomicUsize::new(0),
            window_exceeded: AtomicBool::new(false),
        }
    }

    /// Adds one credit. A peer that grants more than the advertised window has
    /// lost track of its credits, so the stream is failed rather than silently
    /// dropping the grant (which would leave the peer waiting for a segment).
    fn grant(&self) {
        let mut credits = self.credits.load(Ordering::Relaxed);
        loop {
            if credits >= MAX_RESPONSE_CREDITS {
                self.window_exceeded.store(true, Ordering::Release);
                self.semaphore.close();
                return;
            }
            match self.credits.compare_exchange_weak(
                credits,
                credits + 1,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    self.semaphore.add_permits(1);
                    return;
                }
                Err(actual) => credits = actual,
            }
        }
    }

    async fn acquire(&self, cancellation: &CancellationToken) -> Result<(), String> {
        let permit = tokio::select! {
            _ = cancellation.cancelled() => return Err("operation was cancelled".to_owned()),
            result = self.semaphore.acquire() => result.map_err(|_| "response flow control closed".to_owned())?,
        };
        permit.forget();
        self.credits.fetch_sub(1, Ordering::AcqRel);
        Ok(())
    }

    fn window_exceeded(&self) -> bool {
        self.window_exceeded.load(Ordering::Acquire)
    }
}

struct SubscriptionState {
    cancellation: CancellationToken,
}

struct IncomingPayload {
    count: u32,
    received_count: u32,
    received_bytes: u64,
    next_index: u32,
    buffered: BTreeMap<u32, (u64, usize)>,
    spool: Option<ReorderSpool>,
    body_tx: Option<mpsc::Sender<Result<Bytes, std::io::Error>>>,
    waiting_for_capacity: bool,
    descriptor: Option<PayloadDescriptor>,
    owner: Option<String>,
    updated_at: Instant,
}

impl IncomingPayload {
    fn new(count: u32) -> Result<Self, String> {
        if count == 0 {
            return Err("payload has an invalid chunk count".to_owned());
        }
        Ok(Self {
            count,
            received_count: 0,
            received_bytes: 0,
            next_index: 0,
            buffered: BTreeMap::new(),
            spool: None,
            body_tx: None,
            waiting_for_capacity: false,
            descriptor: None,
            owner: None,
            updated_at: Instant::now(),
        })
    }

    async fn add_chunk(
        &mut self,
        index: u32,
        count: u32,
        bytes: Vec<u8>,
        budgets: &SpoolBudgets,
    ) -> Result<bool, String> {
        if count != self.count || index >= self.count || bytes.is_empty() {
            return Err("payload chunk conflicts with earlier framing".to_owned());
        }
        if index < self.next_index || self.buffered.contains_key(&index) {
            return Err("duplicate payload chunk".to_owned());
        }
        self.received_count += 1;
        self.received_bytes = self
            .received_bytes
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| "payload size overflow".to_owned())?;
        if self.received_bytes > MAX_TRANSFER_SIZE
            || self
                .descriptor
                .as_ref()
                .is_some_and(|descriptor| self.received_bytes > descriptor.byte_length)
        {
            return Err("incoming payload exceeds its declared size".to_owned());
        }
        self.updated_at = Instant::now();

        if self.body_tx.is_some() && index == self.next_index {
            let body_tx = self
                .body_tx
                .as_ref()
                .ok_or_else(|| "request body sender disappeared".to_owned())?;
            match body_tx.try_send(Ok(Bytes::from(bytes))) {
                Ok(()) => {
                    self.next_index += 1;
                    if !self.flush_available().await? {
                        return Ok(false);
                    }
                }
                Err(mpsc::error::TrySendError::Full(Ok(bytes))) => {
                    self.spool_chunk(index, &bytes, budgets).await?;
                }
                Err(mpsc::error::TrySendError::Full(Err(_))) => {
                    return Err("invalid request body queue value".to_owned())
                }
                Err(mpsc::error::TrySendError::Closed(_)) => return Ok(false),
            }
        } else {
            self.spool_chunk(index, &bytes, budgets).await?;
        }
        Ok(true)
    }

    async fn attach_body(
        &mut self,
        body_tx: mpsc::Sender<Result<Bytes, std::io::Error>>,
    ) -> Result<bool, String> {
        self.body_tx = Some(body_tx);
        self.flush_available().await
    }

    async fn spool_chunk(
        &mut self,
        index: u32,
        bytes: &[u8],
        budgets: &SpoolBudgets,
    ) -> Result<(), String> {
        if self.buffered.len() >= MAX_REORDERED_CHUNKS {
            return Err("payload reordering window exceeded".to_owned());
        }
        let spool = match self.spool.as_mut() {
            Some(spool) => spool,
            None => self.spool.insert(ReorderSpool::new(budgets.clone())?),
        };
        let slot = spool.write(bytes).await?;
        self.buffered.insert(index, (slot, bytes.len()));
        Ok(())
    }

    async fn flush_available(&mut self) -> Result<bool, String> {
        while self.buffered.contains_key(&self.next_index) {
            let body_tx = self
                .body_tx
                .as_ref()
                .ok_or_else(|| "request body sender disappeared".to_owned())?
                .clone();
            let permit = match body_tx.try_reserve_owned() {
                Ok(permit) => permit,
                Err(mpsc::error::TrySendError::Full(_)) => return Ok(true),
                Err(mpsc::error::TrySendError::Closed(_)) => return Ok(false),
            };
            let (slot, length) = self
                .buffered
                .remove(&self.next_index)
                .ok_or_else(|| "payload reorder index disappeared".to_owned())?;
            let bytes = self
                .spool
                .as_mut()
                .ok_or_else(|| "payload spool disappeared".to_owned())?
                .read_and_free(slot, length)
                .await?;
            permit.send(Ok(bytes));
            self.next_index += 1;
            self.updated_at = Instant::now();
        }
        if self.buffered.is_empty() {
            // The reservation is released only when the file itself is dropped.
            self.spool = None;
        }
        Ok(true)
    }

    fn needs_capacity_waiter(&self) -> bool {
        self.body_tx.is_some()
            && !self.waiting_for_capacity
            && self.buffered.contains_key(&self.next_index)
    }

    async fn flush_reserved(
        &mut self,
        permit: mpsc::OwnedPermit<Result<Bytes, std::io::Error>>,
    ) -> Result<(), String> {
        self.waiting_for_capacity = false;
        let Some((slot, length)) = self.buffered.remove(&self.next_index) else {
            drop(permit);
            return Ok(());
        };
        let bytes = self
            .spool
            .as_mut()
            .ok_or_else(|| "payload spool disappeared".to_owned())?
            .read_and_free(slot, length)
            .await?;
        permit.send(Ok(bytes));
        self.next_index += 1;
        self.updated_at = Instant::now();
        self.flush_available().await?;
        if self.buffered.is_empty() {
            self.spool = None;
        }
        Ok(())
    }

    fn is_complete(&self) -> bool {
        self.descriptor.is_some()
            && self.received_count == self.count
            && self.next_index == self.count
            && self.buffered.is_empty()
    }

    fn validate_complete(&self) -> Result<(), String> {
        let descriptor = self
            .descriptor
            .as_ref()
            .ok_or_else(|| "payload has no descriptor".to_owned())?;
        if descriptor.chunks != self.count || descriptor.byte_length != self.received_bytes {
            return Err("payload length does not match its descriptor".to_owned());
        }
        Ok(())
    }

    fn should_expire(&self) -> bool {
        self.updated_at.elapsed() >= PAYLOAD_INACTIVITY
    }
}

enum Completion {
    Request(String),
    Subscription(String),
    PayloadCapacity {
        payload_id: String,
        permit: Option<mpsc::OwnedPermit<Result<Bytes, std::io::Error>>>,
    },
    Fatal(String),
}

struct PeerDispatcher {
    router: Router,
    peer: EstablishedPeer,
    incoming: mpsc::Receiver<DataChannelMessage>,
    sender: DataChannelSender,
    requests: HashMap<String, RequestState>,
    subscriptions: HashMap<String, SubscriptionState>,
    payloads: HashMap<String, IncomingPayload>,
    seen_operations: HashSet<String>,
    operation_order: VecDeque<String>,
    seen_payloads: HashSet<String>,
    payload_order: VecDeque<String>,
    ignored_payloads: HashSet<String>,
    ignored_payload_order: VecDeque<String>,
    cancelled_requests: HashSet<String>,
    cancelled_request_order: VecDeque<String>,
    cancelled_subscriptions: HashSet<String>,
    cancelled_subscription_order: VecDeque<String>,
    spool_budgets: SpoolBudgets,
    completion_tx: mpsc::UnboundedSender<Completion>,
    completion_rx: mpsc::UnboundedReceiver<Completion>,
}

impl PeerDispatcher {
    fn new(
        router: Router,
        peer: EstablishedPeer,
        incoming: mpsc::Receiver<DataChannelMessage>,
        spool_budgets: SpoolBudgets,
    ) -> Self {
        let (completion_tx, completion_rx) = mpsc::unbounded_channel();
        let sender = DataChannelSender {
            channel: Arc::clone(&peer.data_channel),
            closed: peer.closed(),
            message_lock: Arc::new(Mutex::new(())),
        };
        Self {
            router,
            peer,
            incoming,
            sender,
            requests: HashMap::new(),
            subscriptions: HashMap::new(),
            payloads: HashMap::new(),
            seen_operations: HashSet::new(),
            operation_order: VecDeque::new(),
            seen_payloads: HashSet::new(),
            payload_order: VecDeque::new(),
            ignored_payloads: HashSet::new(),
            ignored_payload_order: VecDeque::new(),
            cancelled_requests: HashSet::new(),
            cancelled_request_order: VecDeque::new(),
            cancelled_subscriptions: HashSet::new(),
            cancelled_subscription_order: VecDeque::new(),
            spool_budgets,
            completion_tx,
            completion_rx,
        }
    }

    async fn run(mut self) {
        let closed = self.peer.closed();
        let mut expiry = tokio::time::interval(Duration::from_secs(5));
        let result = loop {
            tokio::select! {
                _ = closed.cancelled() => break Ok(()),
                _ = expiry.tick() => {
                    if let Err(error) = self.expire_payloads().await {
                        break Err(error);
                    }
                }
                completion = self.completion_rx.recv() => {
                    match completion {
                        Some(Completion::Request(id)) => { self.finish_request(&id); }
                        Some(Completion::Subscription(id)) => { self.subscriptions.remove(&id); }
                        Some(Completion::PayloadCapacity { payload_id, permit }) => {
                            if let Err(error) = self.handle_payload_capacity(payload_id, permit).await {
                                break Err(error);
                            }
                        }
                        Some(Completion::Fatal(error)) => break Err(error),
                        None => break Ok(()),
                    }
                }
                message = self.incoming.recv() => {
                    let Some(message) = message else { break Ok(()); };
                    if let Err(error) = self.handle_message(message).await {
                        break Err(error);
                    }
                }
            }
        };

        let fatal_error = result.err();
        if let Some(error) = fatal_error.as_deref() {
            log_error(
                LogServiceType::Other,
                format!("WebRTC API dispatcher failed: {error}"),
            );
            let cancellation = CancellationToken::new();
            let safe_error = error.chars().take(256).collect::<String>();
            let _ = self
                .sender
                .send_control(
                    &OutgoingControl::Close {
                        v: WIRE_VERSION,
                        reason: &safe_error,
                    },
                    &cancellation,
                )
                .await;
        }
        for state in self.requests.values() {
            state.cancellation.cancel();
        }
        for state in self.subscriptions.values() {
            state.cancellation.cancel();
        }
        closed.cancel();
        if fatal_error.is_some() {
            let _ = self.peer.data_channel.close().await;
            let _ = self.peer.peer_connection.close().await;
        }
    }

    async fn handle_message(&mut self, message: DataChannelMessage) -> Result<(), String> {
        if message.data.len() > MAX_MESSAGE_SIZE {
            return Err("DataChannel message exceeds the v1 limit".to_owned());
        }
        if message.is_string {
            self.handle_control(decode_control(&message.data)?).await
        } else {
            self.handle_chunk(decode_binary(&message.data)?).await
        }
    }

    async fn handle_control(&mut self, frame: IncomingControl) -> Result<(), String> {
        match frame {
            IncomingControl::Request(frame) => self.handle_request(frame).await,
            IncomingControl::Subscribe(frame) => self.handle_subscribe(frame).await,
            IncomingControl::Cancel(frame) => {
                validate_identifier(&frame.id, "operation")?;
                match frame.target {
                    CancelTarget::Request => self.cancel_request(&frame.id),
                    CancelTarget::Subscription => self.cancel_subscription(&frame.id),
                }
                Ok(())
            }
            IncomingControl::ResponseCredit(frame) => {
                validate_identifier(&frame.id, "request")?;
                if let Some(request) = self.requests.get(&frame.id) {
                    request.response_flow.grant();
                }
                Ok(())
            }
            IncomingControl::Close(_) => {
                let _ = self.peer.data_channel.close().await;
                let _ = self.peer.peer_connection.close().await;
                self.peer.closed().cancel();
                Ok(())
            }
        }
    }

    async fn handle_request(&mut self, frame: RequestFrame) -> Result<(), String> {
        validate_identifier(&frame.id, "request")?;
        validate_request_metadata(&frame.method, &frame.path, &frame.headers, &frame.params)?;
        if frame.response_type.as_deref().is_some_and(|value| {
            !matches!(value, "json" | "text" | "blob" | "arraybuffer" | "stream")
        }) {
            return Err("unsupported response type".to_owned());
        }
        if frame.response_stream.is_some_and(|version| version != 1)
            || (frame.response_stream.is_some() && frame.response_type.as_deref() != Some("stream"))
        {
            return Err("unsupported response stream protocol".to_owned());
        }
        validate_descriptor(&frame.payload)?;
        if self.cancelled_requests.contains(&frame.id) {
            self.register_operation(&frame.id)?;
            self.discard_request_payload(&frame.payload)?;
            return Ok(());
        }
        if self.requests.len() >= MAX_ACTIVE_REQUESTS {
            self.register_operation(&frame.id)?;
            self.discard_request_payload(&frame.payload)?;
            return self
                .send_immediate_error(&frame.id, 429, "transport", "too many active requests")
                .await;
        }
        self.register_operation(&frame.id)?;

        let cancellation = CancellationToken::new();
        let response_flow = Arc::new(ResponseFlow::new());
        let payload_id = frame.payload.id.clone();
        self.requests.insert(
            frame.id.clone(),
            RequestState {
                frame: Some(frame.clone()),
                cancellation,
                payload_id: payload_id.clone(),
                started: false,
                response_flow,
            },
        );

        if frame.payload.byte_length == 0 {
            if let Some(payload_id) = payload_id {
                self.register_payload_id(&payload_id)?;
            }
            self.start_request(&frame.id, None).await?;
            return Ok(());
        }

        let payload_id = payload_id.ok_or_else(|| "non-empty payload has no ID".to_owned())?;
        self.register_payload_id(&payload_id)?;
        if self.ignored_payloads.contains(&payload_id) {
            return Err("payload ID refers to an expired payload".to_owned());
        }
        if !self.payloads.contains_key(&payload_id) {
            if self.payloads.len() >= MAX_PENDING_PAYLOADS {
                return Err("too many pending payloads".to_owned());
            }
            self.payloads.insert(
                payload_id.clone(),
                IncomingPayload::new(frame.payload.chunks)?,
            );
        }
        let payload = self
            .payloads
            .get_mut(&payload_id)
            .ok_or_else(|| "payload disappeared before dispatch".to_owned())?;
        if payload.count != frame.payload.chunks
            || payload.descriptor.is_some()
            || payload.owner.is_some()
        {
            return Err("payload descriptor conflicts with earlier framing".to_owned());
        }
        let descriptor_length = frame.payload.byte_length;
        payload.descriptor = Some(frame.payload);
        payload.owner = Some(frame.id.clone());
        payload.updated_at = Instant::now();
        if payload.received_bytes > descriptor_length
            || (payload.received_count == payload.count
                && payload.received_bytes != descriptor_length)
        {
            return Err("payload length does not match its descriptor".to_owned());
        }
        let (body_tx, body_rx) = mpsc::channel(REQUEST_BODY_QUEUE);
        self.start_request(&frame.id, Some(body_rx)).await?;
        let keep = self
            .payloads
            .get_mut(&payload_id)
            .ok_or_else(|| "payload disappeared before body attachment".to_owned())?
            .attach_body(body_tx)
            .await?;
        if !keep {
            self.abandon_payload(&payload_id);
            return Ok(());
        }
        self.schedule_payload_capacity(&payload_id);
        self.finish_payload_if_complete(&payload_id)?;
        Ok(())
    }

    async fn handle_subscribe(&mut self, frame: SubscribeFrame) -> Result<(), String> {
        validate_identifier(&frame.id, "subscription")?;
        validate_request_metadata("GET", &frame.path, &frame.headers, &frame.params)?;
        if self.cancelled_subscriptions.contains(&frame.id) {
            self.register_operation(&frame.id)?;
            return Ok(());
        }
        if self.subscriptions.len() >= MAX_ACTIVE_SUBSCRIPTIONS {
            let cancellation = CancellationToken::new();
            return self
                .sender
                .send_control(
                    &OutgoingControl::SubscriptionEnd {
                        v: WIRE_VERSION,
                        subscription_id: &frame.id,
                        sequence: Some(0),
                        error: Some(WireError {
                            kind: "transport",
                            message: "too many active subscriptions".to_owned(),
                        }),
                    },
                    &cancellation,
                )
                .await;
        }
        self.register_operation(&frame.id)?;
        let cancellation = CancellationToken::new();
        self.subscriptions.insert(
            frame.id.clone(),
            SubscriptionState {
                cancellation: cancellation.clone(),
            },
        );
        let router = self.router.clone();
        let sender = self.sender.clone();
        let completion = self.completion_tx.clone();
        tokio::spawn(async move {
            let id = frame.id.clone();
            let result = serve_subscription(router, sender, frame, cancellation.clone()).await;
            if let Err(error) = result {
                if !cancellation.is_cancelled() {
                    let _ = completion.send(Completion::Fatal(error));
                    return;
                }
            }
            let _ = completion.send(Completion::Subscription(id));
        });
        Ok(())
    }

    async fn handle_chunk(&mut self, chunk: super::protocol::BinaryChunk) -> Result<(), String> {
        if self.ignored_payloads.contains(&chunk.payload_id) {
            return Ok(());
        }
        if self.seen_payloads.contains(&chunk.payload_id)
            && !self.payloads.contains_key(&chunk.payload_id)
        {
            return Err("payload chunk arrived after payload completion".to_owned());
        }
        if chunk.bytes.len() > max_chunk_size(&chunk.payload_id)? {
            return Err("payload chunk exceeds the message-size limit".to_owned());
        }
        if !self.payloads.contains_key(&chunk.payload_id) {
            if self.payloads.len() >= MAX_PENDING_PAYLOADS {
                return Err("too many orphan payloads".to_owned());
            }
            self.payloads
                .insert(chunk.payload_id.clone(), IncomingPayload::new(chunk.count)?);
        }
        let keep = {
            let payload = self
                .payloads
                .get_mut(&chunk.payload_id)
                .ok_or_else(|| "payload disappeared before chunk dispatch".to_owned())?;
            payload
                .add_chunk(chunk.index, chunk.count, chunk.bytes, &self.spool_budgets)
                .await?
        };
        if !keep {
            self.abandon_payload(&chunk.payload_id);
            return Ok(());
        }
        self.schedule_payload_capacity(&chunk.payload_id);
        self.finish_payload_if_complete(&chunk.payload_id)?;
        Ok(())
    }

    fn schedule_payload_capacity(&mut self, payload_id: &str) {
        let Some(payload) = self.payloads.get_mut(payload_id) else {
            return;
        };
        if !payload.needs_capacity_waiter() {
            return;
        }
        let Some(body_tx) = payload.body_tx.clone() else {
            return;
        };
        payload.waiting_for_capacity = true;
        let payload_id = payload_id.to_owned();
        let completion = self.completion_tx.clone();
        tokio::spawn(async move {
            let permit = body_tx.reserve_owned().await.ok();
            let _ = completion.send(Completion::PayloadCapacity { payload_id, permit });
        });
    }

    async fn handle_payload_capacity(
        &mut self,
        payload_id: String,
        permit: Option<mpsc::OwnedPermit<Result<Bytes, std::io::Error>>>,
    ) -> Result<(), String> {
        let Some(permit) = permit else {
            self.abandon_payload(&payload_id);
            return Ok(());
        };
        let Some(payload) = self.payloads.get_mut(&payload_id) else {
            return Ok(());
        };
        payload.flush_reserved(permit).await?;
        self.schedule_payload_capacity(&payload_id);
        self.finish_payload_if_complete(&payload_id)
    }

    fn abandon_payload(&mut self, payload_id: &str) {
        let owner = self
            .payloads
            .remove(payload_id)
            .and_then(|payload| payload.owner);
        self.remember_ignored_payload(payload_id.to_owned());
        if let Some(owner) = owner {
            if let Some(request) = self.requests.get_mut(&owner) {
                request.payload_id = None;
            }
        }
    }

    fn register_operation(&mut self, id: &str) -> Result<(), String> {
        if self.requests.contains_key(id)
            || self.subscriptions.contains_key(id)
            || !self.seen_operations.insert(id.to_owned())
        {
            return Err("duplicate operation ID".to_owned());
        }
        self.operation_order.push_back(id.to_owned());
        if self.operation_order.len() > MAX_SEEN_IDENTIFIERS {
            if let Some(expired) = self.operation_order.pop_front() {
                self.seen_operations.remove(&expired);
            }
        }
        Ok(())
    }

    fn register_payload_id(&mut self, id: &str) -> Result<(), String> {
        validate_identifier(id, "payload")?;
        if !self.seen_payloads.insert(id.to_owned()) {
            return Err("duplicate payload ID".to_owned());
        }
        self.payload_order.push_back(id.to_owned());
        if self.payload_order.len() > MAX_SEEN_IDENTIFIERS {
            if let Some(expired) = self.payload_order.pop_front() {
                self.seen_payloads.remove(&expired);
            }
        }
        Ok(())
    }

    fn remember_ignored_payload(&mut self, id: String) {
        if !self.ignored_payloads.insert(id.clone()) {
            return;
        }
        self.ignored_payload_order.push_back(id);
        if self.ignored_payload_order.len() > MAX_SEEN_IDENTIFIERS {
            if let Some(expired) = self.ignored_payload_order.pop_front() {
                self.ignored_payloads.remove(&expired);
            }
        }
    }

    fn discard_request_payload(&mut self, descriptor: &PayloadDescriptor) -> Result<(), String> {
        let Some(payload_id) = descriptor.id.as_deref() else {
            return Ok(());
        };
        validate_identifier(payload_id, "payload")?;
        self.payloads.remove(payload_id);
        self.remember_ignored_payload(payload_id.to_owned());
        Ok(())
    }

    fn finish_payload_if_complete(&mut self, payload_id: &str) -> Result<(), String> {
        let Some(payload) = self.payloads.get(payload_id) else {
            return Ok(());
        };
        if !payload.is_complete() {
            return Ok(());
        }
        payload.validate_complete()?;
        self.payloads.remove(payload_id);
        Ok(())
    }

    async fn start_request(
        &mut self,
        request_id: &str,
        payload: Option<mpsc::Receiver<Result<Bytes, std::io::Error>>>,
    ) -> Result<(), String> {
        let state = self
            .requests
            .get_mut(request_id)
            .ok_or_else(|| "request disappeared before dispatch".to_owned())?;
        if state.started {
            return Err("request was dispatched twice".to_owned());
        }
        let frame = state
            .frame
            .take()
            .ok_or_else(|| "request frame was already consumed".to_owned())?;
        state.started = true;
        let cancellation = state.cancellation.clone();
        let response_flow = Arc::clone(&state.response_flow);
        let router = self.router.clone();
        let sender = self.sender.clone();
        let spool_budgets = self.spool_budgets.clone();
        let completion = self.completion_tx.clone();
        tokio::spawn(async move {
            let id = frame.id.clone();
            let result = serve_request(
                router,
                sender,
                frame,
                payload,
                spool_budgets,
                response_flow,
                cancellation.clone(),
            )
            .await;
            if let Err(error) = result {
                if !cancellation.is_cancelled() {
                    let _ = completion.send(Completion::Fatal(error));
                    return;
                }
            }
            let _ = completion.send(Completion::Request(id));
        });
        Ok(())
    }

    fn cancel_request(&mut self, id: &str) {
        let Some(state) = self.requests.remove(id) else {
            remember_bounded(
                &mut self.cancelled_requests,
                &mut self.cancelled_request_order,
                id,
            );
            return;
        };
        state.cancellation.cancel();
        if let Some(payload_id) = state.payload_id {
            self.payloads.remove(&payload_id);
            self.remember_ignored_payload(payload_id);
        }
    }

    fn cancel_subscription(&mut self, id: &str) {
        if let Some(state) = self.subscriptions.remove(id) {
            state.cancellation.cancel();
        } else {
            remember_bounded(
                &mut self.cancelled_subscriptions,
                &mut self.cancelled_subscription_order,
                id,
            );
        }
    }

    fn finish_request(&mut self, id: &str) {
        let Some(state) = self.requests.remove(id) else {
            return;
        };
        if let Some(payload_id) = state.payload_id {
            if self.payloads.remove(&payload_id).is_some() {
                self.remember_ignored_payload(payload_id);
            }
        }
    }

    async fn expire_payloads(&mut self) -> Result<(), String> {
        let expired = self
            .payloads
            .iter()
            .filter(|(_, payload)| payload.should_expire())
            .map(|(id, payload)| (id.clone(), payload.owner.clone()))
            .collect::<Vec<_>>();
        for (payload_id, owner) in expired {
            self.payloads.remove(&payload_id);
            self.remember_ignored_payload(payload_id);
            if let Some(owner) = owner {
                if let Some(request) = self.requests.remove(&owner) {
                    request.cancellation.cancel();
                    self.send_immediate_error(&owner, 408, "timeout", "request payload timed out")
                        .await?;
                }
            }
        }
        Ok(())
    }

    async fn send_immediate_error(
        &self,
        id: &str,
        status: u16,
        kind: &'static str,
        message: &str,
    ) -> Result<(), String> {
        let cancellation = CancellationToken::new();
        self.sender
            .send_control(
                &OutgoingControl::Response {
                    v: WIRE_VERSION,
                    id,
                    status,
                    headers: HashMap::new(),
                    payload: PayloadDescriptor::none(),
                    error: Some(WireError {
                        kind,
                        message: message.to_owned(),
                    }),
                },
                &cancellation,
            )
            .await
    }
}

fn remember_bounded(set: &mut HashSet<String>, order: &mut VecDeque<String>, id: &str) {
    if !set.insert(id.to_owned()) {
        return;
    }
    order.push_back(id.to_owned());
    if order.len() > MAX_SEEN_IDENTIFIERS {
        if let Some(expired) = order.pop_front() {
            set.remove(&expired);
        }
    }
}

fn validate_descriptor(descriptor: &PayloadDescriptor) -> Result<(), String> {
    if descriptor.byte_length > MAX_TRANSFER_SIZE {
        return Err("incoming payload exceeds the size limit".to_owned());
    }
    if descriptor
        .content_type
        .as_ref()
        .is_some_and(|value| value.len() > 512 || value.contains(['\r', '\n', '\0']))
    {
        return Err("invalid payload content type".to_owned());
    }
    match descriptor.encoding {
        PayloadEncoding::None => {
            if descriptor.id.is_some()
                || descriptor.byte_length != 0
                || descriptor.chunks != 0
                || descriptor.content_type.is_some()
            {
                return Err("none payload has data fields".to_owned());
            }
        }
        _ if descriptor.byte_length == 0 => {
            if descriptor.chunks != 0 {
                return Err("empty payload declares chunks".to_owned());
            }
            if let Some(id) = &descriptor.id {
                validate_identifier(id, "payload")?;
            }
        }
        _ => {
            let id = descriptor
                .id
                .as_deref()
                .ok_or_else(|| "non-empty payload has no ID".to_owned())?;
            validate_identifier(id, "payload")?;
            if descriptor.chunks == 0 {
                return Err("non-empty payload has no chunks".to_owned());
            }
            let minimum_chunks = descriptor.byte_length.div_ceil(max_chunk_size(id)? as u64);
            if (descriptor.chunks as u64) < minimum_chunks
                || (descriptor.chunks as u64) > descriptor.byte_length
            {
                return Err("payload chunk count cannot represent its byte length".to_owned());
            }
        }
    }
    Ok(())
}

fn validate_request_metadata(
    method: &str,
    path: &str,
    headers: &HashMap<String, String>,
    params: &HashMap<String, Value>,
) -> Result<(), String> {
    let method =
        Method::from_bytes(method.as_bytes()).map_err(|_| "invalid request method".to_owned())?;
    if !matches!(
        method,
        Method::GET
            | Method::POST
            | Method::PUT
            | Method::PATCH
            | Method::DELETE
            | Method::HEAD
            | Method::OPTIONS
    ) {
        return Err("unsupported request method".to_owned());
    }
    build_uri(path, params)?;
    build_headers(headers)?;
    Ok(())
}

fn build_uri(path: &str, params: &HashMap<String, Value>) -> Result<Uri, String> {
    if !path.starts_with('/')
        || path.starts_with("//")
        || path.contains('#')
        || path.contains("://")
    {
        return Err("request path must be a relative absolute-path".to_owned());
    }
    let mut value = path.to_owned();
    let mut separator = if path.contains('?') { '&' } else { '?' };
    for (key, item) in params {
        if item.is_null() {
            continue;
        }
        let values: Vec<&Value> = match item {
            Value::Array(values) => values.iter().collect(),
            value => vec![value],
        };
        for item in values {
            value.push(separator);
            separator = '&';
            value.push_str(&urlencoding::encode(key));
            value.push('=');
            value.push_str(&urlencoding::encode(&javascript_string(item)));
        }
    }
    value
        .parse::<Uri>()
        .map_err(|_| "request path and parameters do not form a valid URI".to_owned())
}

fn javascript_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(values) => values
            .iter()
            .map(javascript_string)
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_owned(),
    }
}

fn build_headers(values: &HashMap<String, String>) -> Result<HeaderMap, String> {
    if values.len() > MAX_HEADERS
        || values
            .iter()
            .map(|(name, value)| name.len() + value.len())
            .sum::<usize>()
            > MAX_HEADER_BYTES
    {
        return Err("request headers exceed transport limits".to_owned());
    }
    let mut headers = HeaderMap::new();
    for (name, value) in values {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| "invalid request header name".to_owned())?;
        let value =
            HeaderValue::from_str(value).map_err(|_| "invalid request header value".to_owned())?;
        headers.insert(name, value);
    }
    Ok(headers)
}

async fn serve_request(
    router: Router,
    sender: DataChannelSender,
    frame: RequestFrame,
    payload: Option<mpsc::Receiver<Result<Bytes, std::io::Error>>>,
    spool_budgets: SpoolBudgets,
    response_flow: Arc<ResponseFlow>,
    cancellation: CancellationToken,
) -> Result<(), String> {
    let request = build_request(&frame, payload).await?;
    let response = tokio::select! {
        _ = cancellation.cancelled() => return Ok(()),
        response = router.oneshot(request) => response.map_err(|_| "Axum request dispatch failed".to_owned())?,
    };
    let bodyless = frame.method.eq_ignore_ascii_case("HEAD");
    send_response(
        &sender,
        &frame.id,
        response,
        bodyless,
        frame.response_stream == Some(1),
        spool_budgets,
        response_flow,
        &cancellation,
    )
    .await
}

async fn build_request(
    frame: &RequestFrame,
    payload: Option<mpsc::Receiver<Result<Bytes, std::io::Error>>>,
) -> Result<Request<Body>, String> {
    let method = Method::from_bytes(frame.method.as_bytes())
        .map_err(|_| "invalid request method".to_owned())?;
    let uri = build_uri(&frame.path, &frame.params)?;
    let mut headers = build_headers(&frame.headers)?;
    headers.remove(header::CONTENT_LENGTH);

    let body = match payload {
        None => {
            apply_payload_content_type(&mut headers, &frame.payload, None)?;
            Body::empty()
        }
        Some(payload) if frame.payload.encoding == PayloadEncoding::FormData => {
            let (body, content_type) = multipart_body(payload, frame.payload.byte_length).await?;
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_str(&content_type)
                    .map_err(|_| "unable to create multipart content type".to_owned())?,
            );
            body
        }
        Some(payload) => {
            apply_payload_content_type(&mut headers, &frame.payload, None)?;
            headers.insert(
                header::CONTENT_LENGTH,
                HeaderValue::from_str(&frame.payload.byte_length.to_string())
                    .map_err(|_| "invalid request payload length".to_owned())?,
            );
            Body::from_stream(ReceiverStream::new(payload))
        }
    };

    let mut request = Request::new(body);
    *request.method_mut() = method;
    *request.uri_mut() = uri;
    *request.headers_mut() = headers;
    Ok(request)
}

fn apply_payload_content_type(
    headers: &mut HeaderMap,
    descriptor: &PayloadDescriptor,
    override_value: Option<&str>,
) -> Result<(), String> {
    if headers.contains_key(header::CONTENT_TYPE) {
        return Ok(());
    }
    let value = override_value
        .map(str::to_owned)
        .or_else(|| descriptor.content_type.clone())
        .or_else(|| match descriptor.encoding {
            PayloadEncoding::Json => Some("application/json".to_owned()),
            PayloadEncoding::Text => Some("text/plain;charset=UTF-8".to_owned()),
            PayloadEncoding::Binary => Some("application/octet-stream".to_owned()),
            _ => None,
        });
    if let Some(value) = value {
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_str(&value).map_err(|_| "invalid payload content type".to_owned())?,
        );
    }
    Ok(())
}

type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, String>> + Send>>;

async fn send_response(
    sender: &DataChannelSender,
    request_id: &str,
    response: Response<Body>,
    head_request: bool,
    streaming: bool,
    spool_budgets: SpoolBudgets,
    response_flow: Arc<ResponseFlow>,
    cancellation: &CancellationToken,
) -> Result<(), String> {
    let status = response.status();
    let headers = response_headers(response.headers());
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let declared_length = response
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .or_else(|| response.body().size_hint().exact());
    let body = response.into_body();
    let error = (status.as_u16() >= 400).then(|| WireError {
        kind: status_error_kind(status.as_u16()),
        message: status
            .canonical_reason()
            .unwrap_or("request failed")
            .to_owned(),
    });

    if response_is_bodyless(head_request, status.as_u16()) {
        drop(body);
        return sender
            .send_control(
                &OutgoingControl::Response {
                    v: WIRE_VERSION,
                    id: request_id,
                    status: status.as_u16(),
                    headers,
                    payload: PayloadDescriptor::none(),
                    error,
                },
                cancellation,
            )
            .await;
    }

    if streaming {
        return send_segmented_response(
            sender,
            request_id,
            status.as_u16(),
            headers,
            content_type,
            declared_length,
            body,
            error,
            response_flow,
            cancellation,
        )
        .await;
    }

    let (length, stream, _reservation): (u64, ByteStream, Option<DiskReservation>) =
        match declared_length {
            Some(length) => {
                if length > MAX_LEGACY_RESPONSE_SIZE {
                    drop(body);
                    return send_response_error(
                        sender,
                        request_id,
                        413,
                        headers,
                        "response requires streaming mode",
                        cancellation,
                    )
                    .await;
                }
                (
                    length,
                    Box::pin(TryStreamExt::map_err(body.into_data_stream(), |_| {
                        "response body stream failed".to_owned()
                    })),
                    None,
                )
            }
            None => {
                let (spool, length) = match spool_response(body, spool_budgets, cancellation).await
                {
                    Ok(value) => value,
                    Err(message) => {
                        return send_response_error(
                            sender,
                            request_id,
                            413,
                            headers,
                            &message,
                            cancellation,
                        )
                        .await;
                    }
                };
                let ReservedFile { file, reservation } = spool;
                (
                    length,
                    Box::pin(
                        ReaderStream::new(file)
                            .map_err(|_| "spooled response body could not be read".to_owned()),
                    ),
                    Some(reservation),
                )
            }
        };
    if length == 0 {
        sender
            .send_control(
                &OutgoingControl::Response {
                    v: WIRE_VERSION,
                    id: request_id,
                    status: status.as_u16(),
                    headers,
                    payload: PayloadDescriptor::none(),
                    error,
                },
                cancellation,
            )
            .await?;
        return Ok(());
    }

    let payload_id = new_payload_id();
    let chunk_size = max_chunk_size(&payload_id)?;
    let chunks = length.div_ceil(chunk_size as u64);
    if chunks > u32::MAX as u64 {
        return Err("response body has too many chunks".to_owned());
    }
    let descriptor = PayloadDescriptor {
        id: Some(payload_id.clone()),
        encoding: response_encoding(content_type.as_deref()),
        byte_length: length,
        chunks: chunks as u32,
        content_type,
    };
    sender
        .send_control(
            &OutgoingControl::Response {
                v: WIRE_VERSION,
                id: request_id,
                status: status.as_u16(),
                headers,
                payload: descriptor.clone(),
                error,
            },
            cancellation,
        )
        .await?;
    send_payload_stream(sender, &descriptor, stream, cancellation).await
}

fn response_is_bodyless(head_request: bool, status: u16) -> bool {
    head_request || (100..200).contains(&status) || status == 204 || status == 304
}

async fn spool_response(
    body: Body,
    budgets: SpoolBudgets,
    cancellation: &CancellationToken,
) -> Result<(ReservedFile, u64), String> {
    let mut spool = ReservedFile::new(budgets)?;
    let mut stream = body.into_data_stream();
    let mut length = 0u64;
    loop {
        let next = tokio::select! {
            _ = cancellation.cancelled() => return Err("operation was cancelled".to_owned()),
            next = stream.next() => next,
        };
        let Some(bytes) = next else { break };
        let bytes = bytes.map_err(|_| "response body stream failed".to_owned())?;
        length = length
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| "response size overflow".to_owned())?;
        if length > MAX_LEGACY_RESPONSE_SIZE {
            return Err("response requires streaming mode".to_owned());
        }
        spool.append(&bytes).await?;
    }
    spool
        .file
        .seek(SeekFrom::Start(0))
        .await
        .map_err(|_| "unable to rewind response spool file".to_owned())?;
    Ok((spool, length))
}

#[allow(clippy::too_many_arguments)]
async fn send_segmented_response(
    sender: &DataChannelSender,
    request_id: &str,
    status: u16,
    mut headers: HashMap<String, String>,
    content_type: Option<String>,
    declared_length: Option<u64>,
    body: Body,
    error: Option<WireError>,
    response_flow: Arc<ResponseFlow>,
    cancellation: &CancellationToken,
) -> Result<(), String> {
    let encoding = response_encoding(content_type.as_deref());
    let has_error = error.is_some();
    let wire_length = prepare_segmented_metadata(&mut headers, declared_length, has_error);
    sender
        .send_control(
            &OutgoingControl::ResponseStart {
                v: WIRE_VERSION,
                id: request_id,
                status,
                headers,
                encoding,
                content_type: content_type.clone(),
                byte_length: wire_length,
                error,
            },
            cancellation,
        )
        .await?;
    if has_error {
        drop(body);
        return sender
            .send_control(
                &OutgoingControl::ResponseEnd {
                    v: WIRE_VERSION,
                    id: request_id,
                    segments: 0,
                    byte_length: 0,
                    error: None,
                },
                cancellation,
            )
            .await;
    }

    let mut stream = body.into_data_stream();
    let mut pending = Vec::with_capacity(RESPONSE_SEGMENT_SIZE);
    let mut sequence = 0u64;
    let mut sent = 0u64;
    while let Some(result) = tokio::select! {
        _ = cancellation.cancelled() => return Ok(()),
        next = stream.next() => next,
    } {
        let bytes = match result {
            Ok(bytes) => bytes,
            Err(_) => {
                return send_response_end_error(
                    sender,
                    request_id,
                    sequence,
                    sent,
                    "response body stream failed",
                    cancellation,
                )
                .await;
            }
        };
        let mut offset = 0;
        while offset < bytes.len() {
            let length = (RESPONSE_SEGMENT_SIZE - pending.len()).min(bytes.len() - offset);
            pending.extend_from_slice(&bytes[offset..offset + length]);
            offset += length;
            if pending.len() == RESPONSE_SEGMENT_SIZE {
                if let Err(error) = response_flow.acquire(cancellation).await {
                    if !response_flow.window_exceeded() {
                        return Err(error);
                    }
                    return send_response_end_error(
                        sender,
                        request_id,
                        sequence,
                        sent,
                        "response credit window exceeded",
                        cancellation,
                    )
                    .await;
                }
                send_response_segment(
                    sender,
                    request_id,
                    sequence,
                    &pending,
                    encoding,
                    content_type.clone(),
                    cancellation,
                )
                .await?;
                sent += pending.len() as u64;
                sequence += 1;
                pending.clear();
            }
        }
        if sent.saturating_add(pending.len() as u64) > MAX_TRANSFER_SIZE {
            return send_response_end_error(
                sender,
                request_id,
                sequence,
                sent,
                "response body exceeds the WebRTC transport limit",
                cancellation,
            )
            .await;
        }
    }
    if !pending.is_empty() {
        if let Err(error) = response_flow.acquire(cancellation).await {
            if !response_flow.window_exceeded() {
                return Err(error);
            }
            return send_response_end_error(
                sender,
                request_id,
                sequence,
                sent,
                "response credit window exceeded",
                cancellation,
            )
            .await;
        }
        send_response_segment(
            sender,
            request_id,
            sequence,
            &pending,
            encoding,
            content_type,
            cancellation,
        )
        .await?;
        sent += pending.len() as u64;
        sequence += 1;
    }
    if declared_length.is_some_and(|length| length != sent) {
        return send_response_end_error(
            sender,
            request_id,
            sequence,
            sent,
            "response body did not match its declared length",
            cancellation,
        )
        .await;
    }
    sender
        .send_control(
            &OutgoingControl::ResponseEnd {
                v: WIRE_VERSION,
                id: request_id,
                segments: sequence,
                byte_length: sent,
                error: None,
            },
            cancellation,
        )
        .await
}

fn prepare_segmented_metadata(
    headers: &mut HashMap<String, String>,
    declared_length: Option<u64>,
    has_error: bool,
) -> Option<u64> {
    if has_error {
        headers.insert(header::CONTENT_LENGTH.as_str().to_owned(), "0".to_owned());
        Some(0)
    } else {
        declared_length
    }
}

async fn send_response_segment(
    sender: &DataChannelSender,
    request_id: &str,
    sequence: u64,
    bytes: &[u8],
    encoding: PayloadEncoding,
    content_type: Option<String>,
    cancellation: &CancellationToken,
) -> Result<(), String> {
    let payload_id = new_payload_id();
    let chunk_size = max_chunk_size(&payload_id)?;
    let descriptor = PayloadDescriptor {
        id: Some(payload_id),
        encoding,
        byte_length: bytes.len() as u64,
        chunks: bytes.len().div_ceil(chunk_size) as u32,
        content_type,
    };
    sender
        .send_control(
            &OutgoingControl::ResponseSegment {
                v: WIRE_VERSION,
                id: request_id,
                sequence,
                payload: descriptor.clone(),
            },
            cancellation,
        )
        .await?;
    let stream: ByteStream = Box::pin(futures::stream::once({
        let bytes = Bytes::copy_from_slice(bytes);
        async move { Ok(bytes) }
    }));
    send_payload_stream(sender, &descriptor, stream, cancellation).await
}

async fn send_response_end_error(
    sender: &DataChannelSender,
    request_id: &str,
    segments: u64,
    byte_length: u64,
    message: &str,
    cancellation: &CancellationToken,
) -> Result<(), String> {
    sender
        .send_control(
            &OutgoingControl::ResponseEnd {
                v: WIRE_VERSION,
                id: request_id,
                segments,
                byte_length,
                error: Some(WireError {
                    kind: "server",
                    message: message.to_owned(),
                }),
            },
            cancellation,
        )
        .await
}

async fn send_response_error(
    sender: &DataChannelSender,
    request_id: &str,
    status: u16,
    headers: HashMap<String, String>,
    message: &str,
    cancellation: &CancellationToken,
) -> Result<(), String> {
    sender
        .send_control(
            &OutgoingControl::Response {
                v: WIRE_VERSION,
                id: request_id,
                status,
                headers,
                payload: PayloadDescriptor::none(),
                error: Some(WireError {
                    kind: "transport",
                    message: message.to_owned(),
                }),
            },
            cancellation,
        )
        .await
}

async fn send_payload_stream(
    sender: &DataChannelSender,
    descriptor: &PayloadDescriptor,
    mut stream: ByteStream,
    cancellation: &CancellationToken,
) -> Result<(), String> {
    let id = descriptor
        .id
        .as_deref()
        .ok_or_else(|| "outgoing payload has no ID".to_owned())?;
    let chunk_size = max_chunk_size(id)?;
    let mut pending = Vec::with_capacity(chunk_size);
    let mut index = 0u32;
    let mut sent = 0u64;
    while let Some(bytes) = tokio::select! {
        _ = cancellation.cancelled() => return Ok(()),
        next = stream.next() => next,
    } {
        let bytes = bytes?;
        let mut offset = 0;
        while offset < bytes.len() {
            let length = (chunk_size - pending.len()).min(bytes.len() - offset);
            pending.extend_from_slice(&bytes[offset..offset + length]);
            offset += length;
            if pending.len() == chunk_size {
                sender
                    .send_binary(
                        encode_binary(id, index, descriptor.chunks, &pending)?,
                        cancellation,
                    )
                    .await?;
                sent += pending.len() as u64;
                index += 1;
                pending.clear();
            }
        }
    }
    if !pending.is_empty() {
        sender
            .send_binary(
                encode_binary(id, index, descriptor.chunks, &pending)?,
                cancellation,
            )
            .await?;
        sent += pending.len() as u64;
        index += 1;
    }
    if !cancellation.is_cancelled()
        && (sent != descriptor.byte_length || index != descriptor.chunks)
    {
        return Err("response body did not match its declared length".to_owned());
    }
    Ok(())
}

fn response_headers(headers: &HeaderMap) -> HashMap<String, String> {
    let mut output = HashMap::new();
    for (name, value) in headers {
        let Ok(value) = value.to_str() else { continue };
        output
            .entry(name.as_str().to_owned())
            .and_modify(|existing: &mut String| {
                existing.push_str(", ");
                existing.push_str(value);
            })
            .or_insert_with(|| value.to_owned());
    }
    output
}

fn response_encoding(content_type: Option<&str>) -> PayloadEncoding {
    let content_type = content_type.unwrap_or_default().to_ascii_lowercase();
    if content_type.starts_with("application/json") || content_type.contains("+json") {
        PayloadEncoding::Json
    } else if content_type.starts_with("text/") {
        PayloadEncoding::Text
    } else {
        PayloadEncoding::Binary
    }
}

fn status_error_kind(status: u16) -> &'static str {
    match status {
        401 => "unauthorized",
        403 => "forbidden",
        404 => "not-found",
        500..=599 => "server",
        _ => "transport",
    }
}

fn new_payload_id() -> String {
    format!("server-{}", nanoid!(24))
}

#[derive(Debug, Deserialize)]
struct FormManifest {
    v: u8,
    entries: Vec<FormEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum FormEntry {
    Text {
        name: String,
        value: String,
    },
    Binary {
        name: String,
        offset: u64,
        #[serde(rename = "byteLength")]
        byte_length: u64,
        filename: String,
        #[serde(rename = "contentType")]
        content_type: Option<String>,
    },
}

async fn multipart_body(
    payload: mpsc::Receiver<Result<Bytes, std::io::Error>>,
    total_length: u64,
) -> Result<(Body, String), String> {
    if total_length < 4 {
        return Err("form-data payload is truncated".to_owned());
    }
    let mut reader = StreamReader::new(ReceiverStream::new(payload));
    let mut size = [0u8; 4];
    reader
        .read_exact(&mut size)
        .await
        .map_err(|_| "form-data manifest length is truncated".to_owned())?;
    let manifest_length = u32::from_be_bytes(size) as usize;
    if manifest_length > MAX_FORM_MANIFEST_SIZE || 4 + manifest_length as u64 > total_length {
        return Err("form-data manifest length is invalid".to_owned());
    }
    let mut manifest_bytes = vec![0; manifest_length];
    reader
        .read_exact(&mut manifest_bytes)
        .await
        .map_err(|_| "form-data manifest is truncated".to_owned())?;
    let manifest: FormManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|_| "form-data manifest is invalid JSON".to_owned())?;
    if manifest.v != WIRE_VERSION || manifest.entries.len() > MAX_FORM_ENTRIES {
        return Err("form-data manifest is invalid".to_owned());
    }
    let binary_length = total_length - (4 + manifest_length as u64);
    let mut expected_offset = 0u64;
    for entry in &manifest.entries {
        match entry {
            FormEntry::Text { name, .. } => validate_form_token(name, "field name")?,
            FormEntry::Binary {
                name,
                offset,
                byte_length,
                filename,
                content_type,
            } => {
                validate_form_token(name, "field name")?;
                validate_form_token(filename, "filename")?;
                if *offset != expected_offset
                    || offset
                        .checked_add(*byte_length)
                        .is_none_or(|end| end > binary_length)
                {
                    return Err("form-data binary offsets are invalid".to_owned());
                }
                if content_type.as_ref().is_some_and(|value| {
                    value.len() > 512
                        || value.contains(['\r', '\n', '\0'])
                        || value.parse::<mime::Mime>().is_err()
                }) {
                    return Err("form-data content type is invalid".to_owned());
                }
                expected_offset += byte_length;
            }
        }
    }
    if expected_offset != binary_length {
        return Err("form-data binary section length is invalid".to_owned());
    }

    let boundary = format!("redseat-{}", nanoid!(32));
    let stream_boundary = boundary.clone();
    let stream: Pin<Box<dyn Stream<Item = std::io::Result<Bytes>> + Send>> = Box::pin(
        async_stream::try_stream! {
            for entry in manifest.entries {
                match entry {
                    FormEntry::Text { name, value } => {
                        let header = format!(
                            "--{stream_boundary}\r\nContent-Disposition: form-data; name=\"{}\"\r\nContent-Length: {}\r\n\r\n",
                            quote_multipart(&name), value.len()
                        );
                        yield Bytes::from(header);
                        yield Bytes::from(value);
                        yield Bytes::from_static(b"\r\n");
                    }
                    FormEntry::Binary { name, offset: _, byte_length, filename, content_type } => {
                        let content_type = content_type.unwrap_or_else(|| "application/octet-stream".to_owned());
                        let header = format!(
                            "--{stream_boundary}\r\nContent-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: {content_type}\r\nContent-Length: {byte_length}\r\n\r\n",
                            quote_multipart(&name), quote_multipart(&filename)
                        );
                        yield Bytes::from(header);
                        let mut remaining = byte_length;
                        let mut buffer = vec![0u8; 64 * 1024];
                        while remaining > 0 {
                            let read_length = remaining.min(buffer.len() as u64) as usize;
                            let read = reader.read(&mut buffer[..read_length]).await?;
                            if read == 0 {
                                Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "form-data binary field is truncated"))?;
                            }
                            remaining -= read as u64;
                            yield Bytes::copy_from_slice(&buffer[..read]);
                        }
                        yield Bytes::from_static(b"\r\n");
                    }
                }
            }
            yield Bytes::from(format!("--{stream_boundary}--\r\n"));
        },
    );
    Ok((
        Body::from_stream(stream),
        format!("multipart/form-data; boundary={boundary}"),
    ))
}

fn validate_form_token(value: &str, kind: &str) -> Result<(), String> {
    if value.len() > 4096 || value.contains(['\r', '\n', '\0']) {
        return Err(format!("form-data {kind} is invalid"));
    }
    Ok(())
}

fn quote_multipart(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

async fn serve_subscription(
    router: Router,
    sender: DataChannelSender,
    frame: SubscribeFrame,
    cancellation: CancellationToken,
) -> Result<(), String> {
    let request = build_subscription_request(&frame)?;
    let response = tokio::select! {
        _ = cancellation.cancelled() => return Ok(()),
        response = router.oneshot(request) => response.map_err(|_| "Axum subscription dispatch failed".to_owned())?,
    };
    if !response.status().is_success() {
        sender
            .send_control(
                &OutgoingControl::SubscriptionEnd {
                    v: WIRE_VERSION,
                    subscription_id: &frame.id,
                    sequence: Some(0),
                    error: Some(WireError {
                        kind: status_error_kind(response.status().as_u16()),
                        message: response
                            .status()
                            .canonical_reason()
                            .unwrap_or("subscription failed")
                            .to_owned(),
                    }),
                },
                &cancellation,
            )
            .await?;
        return Ok(());
    }
    let is_sse = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().starts_with("text/event-stream"));
    if !is_sse {
        return Err("subscription route did not return an SSE stream".to_owned());
    }

    let mut decoder = SseDecoder::default();
    let mut stream = response.into_body().into_data_stream();
    let mut sequence = 0u64;
    loop {
        let next = tokio::select! {
            _ = cancellation.cancelled() => return Ok(()),
            next = stream.next() => next,
        };
        let Some(bytes) = next else { break };
        let bytes = bytes.map_err(|_| "SSE response stream failed".to_owned())?;
        for event in decoder.push(&bytes)? {
            send_event(&sender, &frame.id, sequence, event, &cancellation).await?;
            sequence += 1;
        }
    }
    for event in decoder.finish()? {
        send_event(&sender, &frame.id, sequence, event, &cancellation).await?;
        sequence += 1;
    }
    sender
        .send_control(
            &OutgoingControl::SubscriptionEnd {
                v: WIRE_VERSION,
                subscription_id: &frame.id,
                sequence: Some(sequence),
                error: None,
            },
            &cancellation,
        )
        .await
}

fn build_subscription_request(frame: &SubscribeFrame) -> Result<Request<Body>, String> {
    let mut headers = build_headers(&frame.headers)?;
    headers.insert(
        header::ACCEPT,
        HeaderValue::from_static("text/event-stream"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    let mut request = Request::new(Body::empty());
    *request.method_mut() = Method::GET;
    *request.uri_mut() = build_uri(&frame.path, &frame.params)?;
    *request.headers_mut() = headers;
    Ok(request)
}

#[derive(Debug, Default)]
struct SseDecoder {
    buffer: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
    data_bytes: usize,
    id: Option<String>,
    retry: Option<u64>,
}

#[derive(Debug, PartialEq, Eq)]
struct ParsedSseEvent {
    event: String,
    data: String,
    id: Option<String>,
    retry: Option<u64>,
}

impl SseDecoder {
    fn push(&mut self, bytes: &[u8]) -> Result<Vec<ParsedSseEvent>, String> {
        self.buffer.extend_from_slice(bytes);
        if self.buffer.len() > MAX_EVENT_SIZE {
            return Err("SSE event exceeds the transport limit".to_owned());
        }
        let mut events = Vec::new();
        while let Some(position) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let mut line = self.buffer.drain(..=position).collect::<Vec<_>>();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = std::str::from_utf8(&line)
                .map_err(|_| "SSE stream is not valid UTF-8".to_owned())?
                .to_owned();
            if let Some(event) = self.push_line(&line)? {
                events.push(event);
            }
        }
        Ok(events)
    }

    fn finish(&mut self) -> Result<Vec<ParsedSseEvent>, String> {
        let mut events = Vec::new();
        if !self.buffer.is_empty() {
            let line = std::str::from_utf8(&self.buffer)
                .map_err(|_| "SSE stream ended with invalid UTF-8".to_owned())?
                .to_owned();
            self.buffer.clear();
            if let Some(event) = self.push_line(line.trim_end_matches('\r'))? {
                events.push(event);
            }
        }
        if let Some(event) = self.dispatch_event() {
            events.push(event);
        }
        Ok(events)
    }

    fn push_line(&mut self, line: &str) -> Result<Option<ParsedSseEvent>, String> {
        if line.is_empty() {
            return Ok(self.dispatch_event());
        }
        if line.starts_with(':') {
            return Ok(None);
        }
        let (field, value) = line
            .split_once(':')
            .map(|(field, value)| (field, value.strip_prefix(' ').unwrap_or(value)))
            .unwrap_or((line, ""));
        match field {
            "event" if value.len() <= 4096 => self.event = Some(value.to_owned()),
            "event" => return Err("SSE event name exceeds the transport limit".to_owned()),
            "data" => {
                self.data_bytes = self
                    .data_bytes
                    .checked_add(value.len() + usize::from(!self.data.is_empty()))
                    .ok_or_else(|| "SSE event size overflow".to_owned())?;
                if self.data_bytes > MAX_EVENT_SIZE {
                    return Err("SSE event exceeds the transport limit".to_owned());
                }
                self.data.push(value.to_owned());
            }
            "id" if !value.contains('\0') && value.len() <= 4096 => {
                self.id = Some(value.to_owned())
            }
            "id" if !value.contains('\0') => {
                return Err("SSE event ID exceeds the transport limit".to_owned())
            }
            "retry" => self.retry = value.parse().ok(),
            _ => {}
        }
        Ok(None)
    }

    fn dispatch_event(&mut self) -> Option<ParsedSseEvent> {
        if self.data.is_empty() {
            self.event = None;
            self.retry = None;
            self.data_bytes = 0;
            return None;
        }
        let event = ParsedSseEvent {
            event: self.event.take().unwrap_or_else(|| "message".to_owned()),
            data: self.data.drain(..).collect::<Vec<_>>().join("\n"),
            id: self.id.clone(),
            retry: self.retry.take(),
        };
        self.data_bytes = 0;
        Some(event)
    }
}

async fn send_event(
    sender: &DataChannelSender,
    subscription_id: &str,
    sequence: u64,
    event: ParsedSseEvent,
    cancellation: &CancellationToken,
) -> Result<(), String> {
    if event.data.len() > MAX_EVENT_SIZE {
        return Err("SSE event exceeds the transport limit".to_owned());
    }
    let bytes = Bytes::from(event.data);
    let payload_id = new_payload_id();
    let chunk_size = max_chunk_size(&payload_id)?;
    let descriptor = PayloadDescriptor {
        id: Some(payload_id),
        encoding: if serde_json::from_slice::<Value>(&bytes).is_ok() {
            PayloadEncoding::Json
        } else {
            PayloadEncoding::Text
        },
        byte_length: bytes.len() as u64,
        chunks: bytes.len().div_ceil(chunk_size) as u32,
        content_type: None,
    };
    sender
        .send_control(
            &OutgoingControl::Event {
                v: WIRE_VERSION,
                subscription_id,
                sequence,
                event: &event.event,
                id: event.id.as_deref(),
                retry: event.retry,
                payload: descriptor.clone(),
            },
            cancellation,
        )
        .await?;
    let stream: ByteStream = Box::pin(futures::stream::once(async move { Ok(bytes) }));
    send_payload_stream(sender, &descriptor, stream, cancellation).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::{Multipart, Query},
        http::StatusCode,
        response::IntoResponse,
        routing::{get, post},
        Json,
    };
    use serde_json::json;
    use webrtc::{
        api::APIBuilder,
        data_channel::data_channel_init::RTCDataChannelInit,
        peer_connection::{
            configuration::RTCConfiguration, sdp::session_description::RTCSessionDescription,
        },
    };

    use crate::webrtc::peer::{PeerSession, DATA_CHANNEL_LABEL, STREAM_CHANNEL_LABEL};
    use webrtc::peer_connection::RTCPeerConnection;

    fn test_spool_budgets() -> SpoolBudgets {
        SpoolBudgets {
            peer: Arc::new(DiskBudget::new(1024 * 1024)),
            server: Arc::new(DiskBudget::new(1024 * 1024)),
        }
    }

    #[test]
    fn query_parameters_match_http_transport_semantics() {
        let params = HashMap::from([
            ("refresh".to_owned(), json!(true)),
            ("tag".to_owned(), json!(["one", "two"])),
            ("skip".to_owned(), Value::Null),
        ]);
        assert_eq!(
            build_uri("/items?existing=1", &params).unwrap().path(),
            "/items"
        );
        let query = build_uri("/items?existing=1", &params)
            .unwrap()
            .query()
            .unwrap()
            .to_owned();
        assert!(query.contains("existing=1"));
        assert!(query.contains("refresh=true"));
        assert!(query.contains("tag=one"));
        assert!(query.contains("tag=two"));
        assert!(!query.contains("skip"));
    }

    #[test]
    fn head_and_http_bodyless_statuses_never_advertise_wire_bytes() {
        assert!(response_is_bodyless(true, 200));
        assert!(response_is_bodyless(false, 103));
        assert!(response_is_bodyless(false, 204));
        assert!(response_is_bodyless(false, 304));
        assert!(!response_is_bodyless(false, 200));
        assert!(!response_is_bodyless(false, 206));
    }

    #[test]
    fn backpressure_drains_to_the_low_watermark_after_it_starts() {
        assert!(!is_backpressured(BUFFERED_AMOUNT_HIGH, false));
        assert!(is_backpressured(BUFFERED_AMOUNT_HIGH + 1, false));
        assert!(is_backpressured(BUFFERED_AMOUNT_HIGH, true));
        assert!(!is_backpressured(BUFFERED_AMOUNT_LOW, true));
    }

    #[test]
    fn streamed_errors_advertise_an_empty_wire_body() {
        let mut headers = HashMap::from([("content-length".to_owned(), "4096".to_owned())]);
        assert_eq!(
            prepare_segmented_metadata(&mut headers, Some(4096), true),
            Some(0)
        );
        assert_eq!(headers.get("content-length").map(String::as_str), Some("0"));

        let mut headers = HashMap::new();
        assert_eq!(
            prepare_segmented_metadata(&mut headers, None, true),
            Some(0)
        );
        assert_eq!(headers.get("content-length").map(String::as_str), Some("0"));

        let mut headers = HashMap::new();
        assert_eq!(
            prepare_segmented_metadata(&mut headers, Some(4096), false),
            Some(4096)
        );
        assert!(!headers.contains_key("content-length"));
    }

    #[tokio::test]
    async fn delivering_buffered_payload_data_refreshes_expiry_activity() {
        let budgets = test_spool_budgets();
        let mut payload = IncomingPayload::new(1).unwrap();
        payload
            .add_chunk(0, 1, b"body".to_vec(), &budgets)
            .await
            .unwrap();
        payload.updated_at = Instant::now() - PAYLOAD_INACTIVITY;
        assert!(payload.should_expire());

        let (tx, mut rx) = mpsc::channel(1);
        assert!(payload.attach_body(tx).await.unwrap());
        assert_eq!(rx.recv().await.unwrap().unwrap(), "body");
        assert!(!payload.should_expire());
    }

    #[tokio::test]
    async fn spool_quota_is_held_until_the_file_is_dropped() {
        let budgets = test_spool_budgets();
        let mut file = ReservedFile::new(budgets.clone()).unwrap();
        file.append(b"reserved").await.unwrap();
        assert_eq!(budgets.peer.used.load(Ordering::Relaxed), 8);
        assert_eq!(budgets.server.used.load(Ordering::Relaxed), 8);
        drop(file);
        assert_eq!(budgets.peer.used.load(Ordering::Relaxed), 0);
        assert_eq!(budgets.server.used.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn reorder_spool_reuses_and_reclaims_consumed_slots() {
        let budgets = test_spool_budgets();
        let mut spool = ReorderSpool::new(budgets.clone()).unwrap();
        let first = spool.write(b"first").await.unwrap();
        let second = spool.write(b"second").await.unwrap();
        // Quota follows the bytes written, not the slot size.
        assert_eq!(budgets.peer.used.load(Ordering::Relaxed), 5 + 6);

        assert_eq!(spool.read_and_free(first, 5).await.unwrap(), "first");
        let reused = spool.write(b"third-longer").await.unwrap();
        assert_eq!(reused, first);
        assert_eq!(budgets.peer.used.load(Ordering::Relaxed), 12 + 6);

        assert_eq!(spool.read_and_free(second, 6).await.unwrap(), "second");
        // The freed tail slot is truncated and released.
        assert_eq!(budgets.peer.used.load(Ordering::Relaxed), 12);
        assert_eq!(
            spool.read_and_free(reused, 12).await.unwrap(),
            "third-longer"
        );
        assert_eq!(budgets.peer.used.load(Ordering::Relaxed), 0);

        let oversized = vec![0; REORDER_SLOT_SIZE as usize + 1];
        assert!(spool.write(&oversized).await.is_err());
    }

    #[tokio::test]
    async fn full_request_body_queue_never_blocks_chunk_ingestion() {
        let budgets = test_spool_budgets();
        let mut payload = IncomingPayload::new(2).unwrap();
        let (tx, _rx) = mpsc::channel(1);
        assert!(payload.attach_body(tx).await.unwrap());
        payload
            .add_chunk(0, 2, b"first".to_vec(), &budgets)
            .await
            .unwrap();
        tokio::time::timeout(
            Duration::from_millis(100),
            payload.add_chunk(1, 2, b"second".to_vec(), &budgets),
        )
        .await
        .expect("chunk ingestion waited for the body consumer")
        .unwrap();
        assert!(payload.buffered.contains_key(&1));
        assert!(payload.needs_capacity_waiter());
    }

    #[test]
    fn response_credit_window_is_bounded() {
        let flow = ResponseFlow::new();
        for _ in 0..MAX_RESPONSE_CREDITS {
            flow.grant();
        }
        assert_eq!(flow.credits.load(Ordering::Relaxed), MAX_RESPONSE_CREDITS);
        assert_eq!(flow.semaphore.available_permits(), MAX_RESPONSE_CREDITS);
        assert!(!flow.window_exceeded());
    }

    #[tokio::test]
    async fn exceeding_the_response_credit_window_fails_the_stream() {
        let flow = ResponseFlow::new();
        for _ in 0..=MAX_RESPONSE_CREDITS {
            flow.grant();
        }
        assert!(flow.window_exceeded());
        assert!(flow.acquire(&CancellationToken::new()).await.is_err());
    }

    #[tokio::test]
    async fn request_body_reaches_axum_before_the_upload_finishes() {
        let first_chunk_seen = Arc::new(tokio::sync::Notify::new());
        let route_notify = Arc::clone(&first_chunk_seen);
        let router = Router::new().route(
            "/upload",
            post(move |request: Request<Body>| {
                let route_notify = Arc::clone(&route_notify);
                async move {
                    let mut stream = request.into_body().into_data_stream();
                    let first = stream.next().await.unwrap().unwrap();
                    route_notify.notify_one();
                    let second = stream.next().await.unwrap().unwrap();
                    [first, second].concat()
                }
            }),
        );
        let frame = RequestFrame {
            v: 1,
            id: "streaming-upload".to_owned(),
            method: "POST".to_owned(),
            path: "/upload".to_owned(),
            headers: HashMap::new(),
            params: HashMap::new(),
            response_type: None,
            response_stream: None,
            payload: PayloadDescriptor {
                id: Some("streaming-body".to_owned()),
                encoding: PayloadEncoding::Binary,
                byte_length: 6,
                chunks: 2,
                content_type: None,
            },
        };
        let (tx, rx) = mpsc::channel(1);
        let request = build_request(&frame, Some(rx)).await.unwrap();
        let response = tokio::spawn(router.oneshot(request));
        tx.send(Ok(Bytes::from_static(b"abc"))).await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), first_chunk_seen.notified())
            .await
            .unwrap();
        tx.send(Ok(Bytes::from_static(b"def"))).await.unwrap();
        drop(tx);
        let response = response.await.unwrap().unwrap();
        assert_eq!(
            response.into_body().collect().await.unwrap().to_bytes(),
            "abcdef"
        );
    }

    #[test]
    fn sse_decoder_preserves_fields_order_and_multiline_data() {
        let mut decoder = SseDecoder::default();
        assert!(decoder
            .push(b"event: result\nid: 7\ndata: {\"line\":1}\ndata: second")
            .unwrap()
            .is_empty());
        let events = decoder.push(b"\nretry: 1000\n\n").unwrap();
        assert_eq!(
            events,
            vec![ParsedSseEvent {
                event: "result".to_owned(),
                data: "{\"line\":1}\nsecond".to_owned(),
                id: Some("7".to_owned()),
                retry: Some(1000),
            }]
        );
    }

    #[tokio::test]
    async fn form_envelope_becomes_ordered_multipart_with_file_metadata() {
        let manifest = json!({
            "v": 1,
            "entries": [
                {"name":"tag","kind":"text","value":"one"},
                {"name":"tag","kind":"text","value":"two"},
                {"name":"file","kind":"binary","offset":0,"byteLength":3,"filename":"a.bin","contentType":"application/octet-stream"}
            ]
        });
        let manifest = serde_json::to_vec(&manifest).unwrap();
        let mut bytes = (manifest.len() as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(&manifest);
        bytes.extend_from_slice(&[1, 2, 3]);
        let length = bytes.len() as u64;
        let (tx, rx) = mpsc::channel(1);
        tx.send(Ok(Bytes::from(bytes))).await.unwrap();
        drop(tx);
        let (body, content_type) = multipart_body(rx, length).await.unwrap();

        async fn upload(mut multipart: Multipart) -> impl IntoResponse {
            let mut values = Vec::new();
            while let Some(field) = multipart.next_field().await.unwrap() {
                values.push(json!({
                    "name": field.name(),
                    "filename": field.file_name(),
                    "contentType": field.content_type(),
                    "bytes": field.bytes().await.unwrap().to_vec(),
                }));
            }
            Json(values)
        }
        let router = Router::new().route("/upload", post(upload));
        let response = router
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/upload")
                    .header(header::CONTENT_TYPE, content_type)
                    .body(body)
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let value: Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(value[0]["name"], "tag");
        assert_eq!(value[1]["name"], "tag");
        assert_eq!(value[2]["filename"], "a.bin");
        assert_eq!(value[2]["bytes"], json!([1, 2, 3]));
    }

    #[tokio::test]
    async fn built_requests_reuse_router_for_reads_mutations_queries_and_errors() {
        async fn read(Query(params): Query<HashMap<String, String>>) -> Json<Value> {
            Json(json!(params))
        }
        async fn mutate(body: String) -> (StatusCode, String) {
            (StatusCode::CREATED, body)
        }
        let router = Router::new()
            .route("/read", get(read))
            .route("/mutate", post(mutate));
        let read_frame = RequestFrame {
            v: 1,
            id: "read".to_owned(),
            method: "GET".to_owned(),
            path: "/read".to_owned(),
            headers: HashMap::new(),
            params: HashMap::from([("page".to_owned(), json!(2))]),
            response_type: None,
            response_stream: None,
            payload: PayloadDescriptor::none(),
        };
        let response = router
            .clone()
            .oneshot(build_request(&read_frame, None).await.unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            serde_json::from_slice::<Value>(&bytes).unwrap(),
            json!({"page":"2"})
        );

        let missing = RequestFrame {
            path: "/missing".to_owned(),
            ..read_frame.clone()
        };
        assert_eq!(
            router
                .clone()
                .oneshot(build_request(&missing, None).await.unwrap())
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );

        let mutation = RequestFrame {
            id: "mutation".to_owned(),
            method: "POST".to_owned(),
            path: "/mutate".to_owned(),
            payload: PayloadDescriptor {
                id: Some("body".to_owned()),
                encoding: PayloadEncoding::Text,
                byte_length: 5,
                chunks: 1,
                content_type: None,
            },
            ..read_frame
        };
        let (tx, payload) = mpsc::channel(1);
        tx.send(Ok(Bytes::from_static(b"hello"))).await.unwrap();
        drop(tx);
        let response = router
            .oneshot(build_request(&mutation, Some(payload)).await.unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(
            response.into_body().collect().await.unwrap().to_bytes(),
            "hello"
        );
    }

    #[tokio::test]
    async fn payload_spool_accepts_reverse_order_and_rejects_duplicates() {
        let mut payload = IncomingPayload::new(3).unwrap();
        let budgets = test_spool_budgets();
        payload
            .add_chunk(2, 3, b"c".to_vec(), &budgets)
            .await
            .unwrap();
        payload
            .add_chunk(1, 3, b"bb".to_vec(), &budgets)
            .await
            .unwrap();
        payload
            .add_chunk(0, 3, b"aa".to_vec(), &budgets)
            .await
            .unwrap();
        payload.descriptor = Some(PayloadDescriptor {
            id: Some("payload".to_owned()),
            encoding: PayloadEncoding::Binary,
            byte_length: 5,
            chunks: 3,
            content_type: None,
        });
        let (tx, mut rx) = mpsc::channel(3);
        assert!(payload.attach_body(tx).await.unwrap());
        payload.validate_complete().unwrap();
        assert!(payload
            .add_chunk(0, 3, b"aa".to_vec(), &budgets)
            .await
            .is_err());
        drop(payload);
        let mut bytes = Vec::new();
        while let Some(chunk) = rx.recv().await {
            bytes.extend_from_slice(&chunk.unwrap());
        }
        assert_eq!(bytes, b"aabbc");
    }

    #[tokio::test]
    async fn real_peer_routes_an_early_api_request_and_returns_json() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let router = Router::new().route(
            "/api",
            get(|| async { Json(json!({"transport": "webrtc"})) }),
        );
        let registry = Arc::new(PeerRegistry::new());
        let sink = DispatcherSink::new(router, Arc::clone(&registry));
        let (candidate_tx, mut candidate_rx) = mpsc::channel(32);
        let (lifecycle_tx, mut lifecycle_rx) = mpsc::unbounded_channel();
        let answerer = PeerSession::new(
            "dispatcher-session".to_owned(),
            "cloud-user-is-not-auth".to_owned(),
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
        let (response_tx, mut response_rx) = mpsc::unbounded_channel();
        channel.on_message(Box::new(move |message| {
            let response_tx = response_tx.clone();
            Box::pin(async move {
                let _ = response_tx.send(message);
            })
        }));
        let request_sent = Arc::new(tokio::sync::Notify::new());
        let request_channel = Arc::downgrade(&channel);
        let open_request_sent = Arc::clone(&request_sent);
        channel.on_open(Box::new(move || {
            let request_channel = request_channel.clone();
            let request_sent = Arc::clone(&open_request_sent);
            Box::pin(async move {
                let channel = request_channel.upgrade().unwrap();
                channel
                    .send_text(
                        json!({
                            "v": 1,
                            "type": "request",
                            "id": "early-request",
                            "method": "GET",
                            "path": "/api",
                            "headers": {},
                            "params": {},
                            "responseType": "json",
                            "payload": {"encoding": "none", "byteLength": 0, "chunks": 0}
                        })
                        .to_string(),
                    )
                    .await
                    .unwrap();
                request_sent.notify_one();
            })
        }));

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
                        let super::super::peer::PeerEvent::LocalCandidate { candidate, .. } = event.unwrap() else {
                            panic!("unexpected peer event in candidate queue");
                        };
                        offerer.add_ice_candidate(candidate.unwrap_or_default()).await.unwrap();
                    }
                    event = lifecycle_rx.recv() => match event.unwrap() {
                        super::super::peer::PeerEvent::Established(peer) => break peer,
                        super::super::peer::PeerEvent::Failed { message, .. } => panic!("{message}"),
                        super::super::peer::PeerEvent::LocalCandidate { .. } => panic!("candidate in lifecycle queue"),
                    }
                }
            }
        })
        .await
        .expect("WebRTC DataChannel did not open");
        tokio::time::timeout(Duration::from_secs(5), request_sent.notified())
            .await
            .expect("client did not send its request before handoff");
        sink.accept(established).await.unwrap();

        let (descriptor, chunks) = tokio::time::timeout(Duration::from_secs(10), async {
            let mut descriptor: Option<PayloadDescriptor> = None;
            let mut chunks = HashMap::new();
            loop {
                let message = response_rx.recv().await.unwrap();
                if message.is_string {
                    let value: Value = serde_json::from_slice(&message.data).unwrap();
                    if value["type"] == "response" && value["id"] == "early-request" {
                        assert_eq!(value["status"], 200);
                        descriptor =
                            Some(serde_json::from_value(value["payload"].clone()).unwrap());
                    }
                } else {
                    let chunk = decode_binary(&message.data).unwrap();
                    chunks.insert(chunk.index, chunk.bytes);
                }
                if let Some(descriptor) = &descriptor {
                    if chunks.len() == descriptor.chunks as usize {
                        break (descriptor.clone(), chunks);
                    }
                }
            }
        })
        .await
        .expect("dispatcher did not return an API response");
        let mut bytes = Vec::new();
        for index in 0..descriptor.chunks {
            bytes.extend_from_slice(chunks.get(&index).unwrap());
        }
        assert_eq!(
            serde_json::from_slice::<Value>(&bytes).unwrap(),
            json!({"transport": "webrtc"})
        );
        assert_eq!(descriptor.byte_length, bytes.len() as u64);

        channel
            .send_text(json!({"v": 1, "type": "close", "reason": "done"}).to_string())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while registry.contains_session("dispatcher-session").await {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("graceful close did not release the established peer");

        offerer.close().await.unwrap();
        registry.shutdown().await;
    }

    struct RealPeer {
        offerer: Arc<RTCPeerConnection>,
        api: Arc<RTCDataChannel>,
        api_rx: mpsc::UnboundedReceiver<DataChannelMessage>,
        registry: Arc<PeerRegistry>,
        _answerer: PeerSession,
        _sink: DispatcherSink,
    }

    impl RealPeer {
        async fn close(self) {
            self.offerer.close().await.unwrap();
            self.registry.shutdown().await;
        }
    }

    fn collect_messages(
        channel: &Arc<RTCDataChannel>,
    ) -> mpsc::UnboundedReceiver<DataChannelMessage> {
        let (tx, rx) = mpsc::unbounded_channel();
        channel.on_message(Box::new(move |message| {
            let tx = tx.clone();
            Box::pin(async move {
                let _ = tx.send(message);
            })
        }));
        rx
    }

    async fn wait_for_state(channel: &Arc<RTCDataChannel>, state: RTCDataChannelState) {
        tokio::time::timeout(Duration::from_secs(10), async {
            while channel.ready_state() != state {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("DataChannel did not reach {state}"));
    }

    fn new_offerer() -> impl std::future::Future<Output = Arc<RTCPeerConnection>> {
        async {
            Arc::new(
                APIBuilder::new()
                    .build()
                    .new_peer_connection(RTCConfiguration::default())
                    .await
                    .unwrap(),
            )
        }
    }

    /// Connects an in-process offerer to a dispatcher serving `router`.
    async fn connect_real_peer(router: Router) -> RealPeer {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let registry = Arc::new(PeerRegistry::new());
        let sink = DispatcherSink::new(router, Arc::clone(&registry));
        let (candidate_tx, mut candidate_rx) = mpsc::channel(32);
        let (lifecycle_tx, mut lifecycle_rx) = mpsc::unbounded_channel();
        let answerer = PeerSession::new(
            "real-peer-session".to_owned(),
            "cloud-user".to_owned(),
            Vec::new(),
            candidate_tx,
            lifecycle_tx,
        )
        .await
        .unwrap();
        let offerer = new_offerer().await;
        let api = offerer
            .create_data_channel(
                DATA_CHANNEL_LABEL,
                Some(RTCDataChannelInit {
                    ordered: Some(false),
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
        let api_rx = collect_messages(&api);

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
                        let super::super::peer::PeerEvent::LocalCandidate { candidate, .. } = event.unwrap() else {
                            panic!("unexpected peer event in candidate queue");
                        };
                        offerer.add_ice_candidate(candidate.unwrap_or_default()).await.unwrap();
                    }
                    event = lifecycle_rx.recv() => match event.unwrap() {
                        super::super::peer::PeerEvent::Established(peer) => break peer,
                        super::super::peer::PeerEvent::Failed { message, .. } => panic!("{message}"),
                        super::super::peer::PeerEvent::LocalCandidate { .. } => panic!("candidate in lifecycle queue"),
                    }
                }
            }
        })
        .await
        .expect("WebRTC DataChannel did not open");
        sink.accept(established).await.unwrap();
        wait_for_state(&api, RTCDataChannelState::Open).await;
        RealPeer {
            offerer,
            api,
            api_rx,
            registry,
            _answerer: answerer,
            _sink: sink,
        }
    }

    /// Reads the `response` frame for `id` and its reassembled payload.
    async fn read_response(
        rx: &mut mpsc::UnboundedReceiver<DataChannelMessage>,
        id: &str,
    ) -> (Value, Vec<u8>) {
        tokio::time::timeout(Duration::from_secs(10), async {
            let mut frame: Option<Value> = None;
            let mut chunks: HashMap<String, HashMap<u32, Vec<u8>>> = HashMap::new();
            loop {
                let message = rx.recv().await.expect("DataChannel closed");
                if message.is_string {
                    let value: Value = serde_json::from_slice(&message.data).unwrap();
                    if value["type"] == "response" && value["id"] == id {
                        frame = Some(value);
                    }
                } else {
                    let chunk = decode_binary(&message.data).unwrap();
                    chunks
                        .entry(chunk.payload_id)
                        .or_default()
                        .insert(chunk.index, chunk.bytes);
                }
                let Some(frame) = &frame else { continue };
                let descriptor: PayloadDescriptor =
                    serde_json::from_value(frame["payload"].clone()).unwrap();
                let Some(id) = &descriptor.id else {
                    break (frame.clone(), Vec::new());
                };
                let received = chunks.get(id).map_or(0, HashMap::len);
                if received == descriptor.chunks as usize {
                    let parts = chunks.remove(id).unwrap();
                    let mut bytes = Vec::new();
                    for index in 0..descriptor.chunks {
                        bytes.extend_from_slice(&parts[&index]);
                    }
                    break (frame.clone(), bytes);
                }
            }
        })
        .await
        .expect("dispatcher did not respond")
    }

    fn get_request(id: &str, path: &str) -> String {
        json!({
            "v": 1,
            "type": "request",
            "id": id,
            "method": "GET",
            "path": path,
            "responseType": "json",
            "payload": {"encoding": "none", "byteLength": 0, "chunks": 0}
        })
        .to_string()
    }

    #[tokio::test]
    async fn real_peer_accepts_messages_of_the_maximum_size() {
        let router = Router::new().route(
            "/upload",
            post(|body: Bytes| async move { Json(json!({"length": body.len()})) }),
        );
        let mut peer = connect_real_peer(router).await;

        let payload_id = "max-size-upload";
        let chunk_size = max_chunk_size(payload_id).unwrap();
        let body = (0..chunk_size * 3)
            .map(|index| index as u8)
            .collect::<Vec<_>>();
        peer.api
            .send_text(
                json!({
                    "v": 1,
                    "type": "request",
                    "id": "upload-1",
                    "method": "POST",
                    "path": "/upload",
                    "responseType": "json",
                    "payload": {
                        "id": payload_id,
                        "encoding": "binary",
                        "byteLength": body.len(),
                        "chunks": 3
                    }
                })
                .to_string(),
            )
            .await
            .unwrap();
        for (index, chunk) in body.chunks(chunk_size).enumerate() {
            let frame = encode_binary(payload_id, index as u32, 3, chunk).unwrap();
            assert_eq!(frame.len(), MAX_MESSAGE_SIZE);
            peer.api.send(&Bytes::from(frame)).await.unwrap();
        }

        let (frame, bytes) = read_response(&mut peer.api_rx, "upload-1").await;
        assert_eq!(frame["status"], 200);
        assert_eq!(
            serde_json::from_slice::<Value>(&bytes).unwrap(),
            json!({"length": body.len()})
        );
        peer.close().await;
    }

    #[tokio::test]
    async fn real_peer_serves_one_stream_channel_alongside_the_api_channel() {
        let router = Router::new().route(
            "/api",
            get(|| async { Json(json!({"transport": "webrtc"})) }),
        );
        let mut peer = connect_real_peer(router).await;
        let ordered = Some(RTCDataChannelInit {
            ordered: Some(true),
            ..Default::default()
        });
        let stream = peer
            .offerer
            .create_data_channel(STREAM_CHANNEL_LABEL, ordered.clone())
            .await
            .unwrap();
        let mut stream_rx = collect_messages(&stream);
        wait_for_state(&stream, RTCDataChannelState::Open).await;

        stream
            .send_text(get_request("stream-1", "/api"))
            .await
            .unwrap();
        let (frame, bytes) = read_response(&mut stream_rx, "stream-1").await;
        assert_eq!(frame["status"], 200);
        assert_eq!(
            serde_json::from_slice::<Value>(&bytes).unwrap(),
            json!({"transport": "webrtc"})
        );

        peer.api
            .send_text(get_request("api-1", "/api"))
            .await
            .unwrap();
        let (frame, _) = read_response(&mut peer.api_rx, "api-1").await;
        assert_eq!(frame["status"], 200);

        // Only one stream channel may be open per peer.
        let second = peer
            .offerer
            .create_data_channel(STREAM_CHANNEL_LABEL, ordered)
            .await
            .unwrap();
        wait_for_state(&second, RTCDataChannelState::Closed).await;
        assert_eq!(stream.ready_state(), RTCDataChannelState::Open);
        peer.close().await;
    }
}
