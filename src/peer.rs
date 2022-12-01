use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex as StdMutex};

pub struct Peers {
    data: Arc<StdMutex<HashSet<SocketAddr>>>,
}

impl Peers {
    pub fn new() -> Self {
        Self {
            data: Arc::new(StdMutex::new(HashSet::new())),
        }
    }

    pub fn insert(&self, list: &Vec<SocketAddr>) {
        let mut data = self.data.lock().unwrap();

        for socket_address in list {
            data.insert(socket_address.clone());
        }
    }
}
