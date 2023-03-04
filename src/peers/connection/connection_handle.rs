use super::connection_manager::{manage_tcp, ManagerChannels};
use super::message::Message;
use crate::data_structures::ID;
use crate::peers::peer::Peer;
use anyhow::Result;
use tokio::sync::mpsc;

const MESSAGE_CHANNEL_BUFFER: usize = 1 << 5;

pub struct Connection {
    manager_tx: mpsc::Sender<Message>,
}

impl Connection {
    pub fn new(peer: Peer, client_id: ID, info_hash: ID, managers: ManagerChannels) -> Self {
        let (send_tx, send_rx) = mpsc::channel(MESSAGE_CHANNEL_BUFFER);

        let send_tx_clone = send_tx.clone();
        tokio::spawn(async move {
            manage_tcp(client_id, peer, info_hash, managers, send_tx_clone, send_rx).await;
        });

        Self {
            manager_tx: send_tx,
        }
    }

    pub async fn send(&self, message: Message) -> Result<()> {
        Ok(self.manager_tx.send(message).await?)
    }
}
