use std::collections::hash_map::Entry::Vacant;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Instant;

pub enum InactivenessReason {
    Unknown,
    UnableToConnect,
    ChokedForTooLong,
}

struct PeerEntry {
    active: bool,
    inactiveness_reason: InactivenessReason,
    last_connection_instant: Option<Instant>,
}

pub struct Peers {
    data: Arc<StdMutex<HashMap<SocketAddr, PeerEntry>>>,
}

impl Peers {
    pub fn new() -> Self {
        Self {
            data: Arc::new(StdMutex::new(HashMap::new())),
        }
    }

    pub fn get_random(&self) -> Option<SocketAddr> {
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

    pub fn closed(&self, sa: &SocketAddr, reason: InactivenessReason) {
        let mut data = self.data.lock().unwrap();

        if let Some(entry) = data.get_mut(sa) {
            entry.active = false;
            entry.inactiveness_reason = reason;
            entry.last_connection_instant = Some(Instant::now());
        } // TODO else should be impossible
    }

    pub fn insert_list(&self, list: &Vec<SocketAddr>) {
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
}
