use std::collections::hash_map::Entry::Vacant;
use std::collections::HashMap;
use std::net::SocketAddrV4;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Instant;

#[derive(Debug)]
pub enum InactivenessReason {
    Unknown,
    UnableToConnect,
    ChokedForTooLong,
}

#[derive(Debug)]
struct PeerEntry {
    active: bool,
    inactiveness_reason: InactivenessReason,
    last_connection_instant: Option<Instant>,
}

#[derive(Clone, Debug)]
pub struct Peers {
    data: Arc<StdMutex<HashMap<SocketAddrV4, PeerEntry>>>,
}

impl Peers {
    pub fn new() -> Self {
        Self {
            data: Arc::new(StdMutex::new(HashMap::new())),
        }
    }

    pub fn get_random(&self) -> Option<SocketAddrV4> {
        let mut data = self.data.lock().unwrap();

        let no_or_min_time_entry = data.iter_mut().min_by(|x, y| {
            x.1.last_connection_instant
                .cmp(&y.1.last_connection_instant)
        });

        match no_or_min_time_entry {
            Some((sock_addr, entry)) => {
                entry.active = true;
                Some(*sock_addr)
            }
            None => None,
        }
    }

    pub fn closed(&self, sa: &SocketAddrV4, reason: InactivenessReason) {
        let mut data = self.data.lock().unwrap();

        if let Some(entry) = data.get_mut(sa) {
            entry.active = false;
            entry.inactiveness_reason = reason;
            entry.last_connection_instant = Some(Instant::now());
        } // TODO else should be impossible
    }

    pub fn insert_list(&self, list: &Vec<SocketAddrV4>) {
        let mut data = self.data.lock().unwrap();

        for socket_address in list {
            if let Vacant(entry) = data.entry(socket_address.clone()) {
                entry.insert(PeerEntry {
                    active: false,
                    inactiveness_reason: InactivenessReason::Unknown,
                    last_connection_instant: None,
                });
            }
        }
    }

    pub fn peer_addresses(&self) -> Vec<SocketAddrV4> {
        self.data.lock().unwrap().keys().cloned().collect()
    }
}
