use std::sync::{Arc, RwLock};

struct InnerPCI {
    am_choking: bool,
    am_interested: bool,
    peer_choking: bool,
    peer_interested: bool,
    connection_is_active: bool,
}

#[derive(Clone)]
pub struct PeerConnectionInfo {
    data: Arc<RwLock<InnerPCI>>,
}

impl PeerConnectionInfo {
    pub fn new() -> Self {
        Self {
            data: Arc::new(RwLock::new(InnerPCI {
                am_choking: true,
                am_interested: false,
                peer_choking: true,
                peer_interested: false,
                connection_is_active: false,
            })),
        }
    }

    pub fn am_choking(&self) -> bool {
        self.data.read().unwrap().am_choking
    }
    pub fn set_am_choking(&self, v: bool) {
        self.data.write().unwrap().am_choking = v
    }

    pub fn am_interested(&self) -> bool {
        self.data.read().unwrap().am_interested
    }
    pub fn set_am_interested(&self, v: bool) {
        self.data.write().unwrap().am_interested = v
    }

    pub fn peer_choking(&self) -> bool {
        self.data.read().unwrap().peer_choking
    }
    pub fn set_peer_choking(&self, v: bool) {
        self.data.write().unwrap().peer_choking = v
    }

    pub fn peer_interested(&self) -> bool {
        self.data.read().unwrap().peer_interested
    }
    pub fn set_peer_interested(&self, v: bool) {
        self.data.write().unwrap().peer_interested = v
    }

    pub fn connection_is_active(&self) -> bool {
        self.data.read().unwrap().connection_is_active
    }
    pub fn set_connection_is_active(&self, v: bool) {
        self.data.write().unwrap().connection_is_active = v
    }
}
