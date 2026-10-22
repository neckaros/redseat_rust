//! The IPTV proxy streams open per library: each holds one of the library's
//! stream slots, and so one connection to the provider, for as long as it is
//! registered here. A viewer changing channel or quality asks for the new
//! stream while the previous one is still open: the newest request wins, the
//! older stream is cancelled and its slot freed as soon as its task ends,
//! without waiting for the server to notice that the client went away.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
pub struct ProxyStreams {
    inner: Mutex<Inner>,
    /// Woken whenever a stream ends and frees its slot.
    released: Notify,
    /// Held while a request checks and takes a slot, HLS or proxy: the two
    /// kinds share the library's slots but are kept in different maps.
    reserving: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct Inner {
    next_id: u64,
    /// library_id → its streams, oldest first.
    libraries: HashMap<String, Vec<Entry>>,
}

struct Entry {
    id: u64,
    channel_id: String,
    cancel: CancellationToken,
}

/// A registered stream. Dropping it frees the slot.
pub struct ProxySlot {
    streams: Arc<ProxyStreams>,
    library_id: String,
    id: u64,
    /// Cancelled when a newer stream needs the slot.
    pub cancel: CancellationToken,
}

impl ProxyStreams {
    /// Streams open for a library, cancelled or not: a cancelled one keeps its
    /// provider connection until its task ends.
    pub fn count(&self, library_id: &str) -> usize {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.libraries.get(library_id).map_or(0, Vec::len)
    }

    /// Registers a stream when fewer than `capacity` are open for the library.
    pub fn try_register(
        self: &Arc<Self>,
        library_id: &str,
        channel_id: &str,
        capacity: usize,
    ) -> Option<ProxySlot> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.next_id += 1;
        let id = inner.next_id;
        let streams = inner.libraries.entry(library_id.to_string()).or_default();
        if streams.len() >= capacity {
            if streams.is_empty() {
                inner.libraries.remove(library_id);
            }
            return None;
        }
        let cancel = CancellationToken::new();
        streams.push(Entry {
            id,
            channel_id: channel_id.to_string(),
            cancel: cancel.clone(),
        });
        Some(ProxySlot {
            streams: self.clone(),
            library_id: library_id.to_string(),
            id,
            cancel,
        })
    }

    /// Cancels the library's oldest stream not cancelled yet, and returns its
    /// channel. None when every stream is already closing (or there is none).
    pub fn cancel_oldest(&self, library_id: &str) -> Option<String> {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let entry = inner
            .libraries
            .get(library_id)?
            .iter()
            .find(|entry| !entry.cancel.is_cancelled())?;
        entry.cancel.cancel();
        Some(entry.channel_id.clone())
    }

    /// Whether the library has a stream that is closing.
    pub fn has_closing(&self, library_id: &str) -> bool {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner
            .libraries
            .get(library_id)
            .is_some_and(|streams| streams.iter().any(|entry| entry.cancel.is_cancelled()))
    }

    pub async fn reserving(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.reserving.lock().await
    }

    /// Resolves when a stream frees its slot. Created before checking the
    /// slots, it also catches a release that happens in between.
    pub fn released(&self) -> tokio::sync::futures::Notified<'_> {
        self.released.notified()
    }
}

impl Drop for ProxySlot {
    fn drop(&mut self) {
        {
            let mut inner = self.streams.inner.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(streams) = inner.libraries.get_mut(&self.library_id) {
                streams.retain(|entry| entry.id != self.id);
                if streams.is_empty() {
                    inner.libraries.remove(&self.library_id);
                }
            }
        }
        self.streams.released.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_up_to_capacity_and_frees_on_drop() {
        let streams = Arc::new(ProxyStreams::default());
        let first = streams.try_register("lib", "a", 1).expect("first slot");
        assert!(streams.try_register("lib", "b", 1).is_none());
        assert!(streams.try_register("other", "b", 1).is_some());
        assert_eq!(streams.count("lib"), 1);
        drop(first);
        assert_eq!(streams.count("lib"), 0);
        assert!(streams.try_register("lib", "b", 1).is_some());
    }

    #[test]
    fn cancels_the_oldest_stream_once() {
        let streams = Arc::new(ProxyStreams::default());
        let first = streams.try_register("lib", "a", 2).expect("first slot");
        let second = streams.try_register("lib", "b", 2).expect("second slot");
        assert_eq!(streams.cancel_oldest("lib").as_deref(), Some("a"));
        assert!(first.cancel.is_cancelled());
        assert!(streams.has_closing("lib"));
        // The closing stream still holds its slot until it ends.
        assert_eq!(streams.count("lib"), 2);
        assert_eq!(streams.cancel_oldest("lib").as_deref(), Some("b"));
        assert!(second.cancel.is_cancelled());
        assert_eq!(streams.cancel_oldest("lib"), None);
    }

    #[tokio::test]
    async fn wakes_waiters_when_a_slot_is_freed() {
        let streams = Arc::new(ProxyStreams::default());
        let slot = streams.try_register("lib", "a", 1).expect("slot");
        let released = streams.released();
        drop(slot);
        tokio::time::timeout(std::time::Duration::from_secs(1), released)
            .await
            .expect("woken");
    }
}
