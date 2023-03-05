use super::connection_manager::{spawn_connection_manager, ConMessageType, ConnectionMessage};
use super::downloader::{spawn_piece_downloader, PeerManagerCommand};
use super::message::{Message, Piece, Request};
use super::pending_pieces::PendingPieces;
use crate::data_structures::{NoSizeBytes, ID};
use crate::peers::peer::Peer;
use crate::shutdown;
use anyhow::{bail, Result};
use std::collections::{
    hash_map::Entry::{Occupied, Vacant},
    HashMap,
};
use std::net::SocketAddrV4;
use std::sync::Arc;
use tokio::sync::mpsc;

const MESSAGE_CHANNEL_BUFFER: usize = 1 << 5;

pub struct Connection {
    own_peer_id: ID,
    info_hash: ID,
    pending_pieces: PendingPieces,
    peers: HashMap<Peer, PeerConnection>,
    rec: mpsc::Receiver<ConnectionMessage>,
    rec_tx: mpsc::Sender<ConnectionMessage>,
    dht_handler: mpsc::Sender<SocketAddrV4>,
}

struct PeerConnection {
    send_tx: mpsc::Sender<Message>,
    shutdown_tx: shutdown::Sender,
    choker: ChokeLocker,
    download_new_piece_tx: mpsc::Sender<PeerManagerCommand>,
}

fn choke_lock() -> (ChokeLocker, UnchokeWaiter) {
    let (tx, rx) = mpsc::channel(1 << 4);

    (ChokeLocker { tx }, UnchokeWaiter { rx, choking: true })
}

enum ChokingStatus {
    Choke,
    Unchoke,
}

struct ChokeLocker {
    tx: mpsc::Sender<ChokingStatus>,
}

impl ChokeLocker {
    fn choke(&self) {
        self.tx.send(ChokingStatus::Choke);
    }

    fn unchoke(&self) {
        self.tx.send(ChokingStatus::Unchoke);
    }
}

pub struct UnchokeWaiter {
    rx: mpsc::Receiver<ChokingStatus>,
    choking: bool,
}

impl UnchokeWaiter {
    pub async fn wait_if_choking(&mut self) {
        loop {
            while self.choking {
                match self.rx.recv().await.unwrap() {
                    ChokingStatus::Choke => self.choking = true,
                    ChokingStatus::Unchoke => self.choking = false,
                }
            }

            match self.rx.try_recv() {
                Ok(m) => match m {
                    ChokingStatus::Choke => self.choking = true,
                    ChokingStatus::Unchoke => self.choking = false,
                },
                Err(_) => break,
            }
        }
    }
}

impl Connection {
    pub fn new(
        own_peer_id: ID,
        info_hash: ID,
        piece_length: usize,
        dht_handler: mpsc::Sender<SocketAddrV4>,
    ) -> Self {
        let (rec_tx, rec) = mpsc::channel(MESSAGE_CHANNEL_BUFFER);

        Self {
            info_hash,
            own_peer_id,
            dht_handler,
            pending_pieces: PendingPieces::new(piece_length),
            peers: HashMap::new(),
            rec,
            rec_tx,
        }
    }

    pub fn connect(&mut self, peer: Peer) {
        let (send_tx, send_rx) = mpsc::channel(MESSAGE_CHANNEL_BUFFER);
        let (shutdown_tx, shutdown_rx) = shutdown::channel();
        let (downloaded_block_tx, downloaded_block_rx) = mpsc::channel(MESSAGE_CHANNEL_BUFFER);
        let (download_new_piece_tx, download_new_piece_rx) = mpsc::channel(MESSAGE_CHANNEL_BUFFER);
        let (choker, unchoke_waiter) = choke_lock();

        match self.peers.entry(peer) {
            Occupied(_) => return,
            Vacant(entry) => {
                entry.insert(PeerConnection {
                    send_tx: send_tx.clone(),
                    shutdown_tx,
                    choker,
                    download_new_piece_tx,
                });
            }
        }

        spawn_connection_manager(
            self.own_peer_id,
            peer,
            self.info_hash,
            downloaded_block_tx,
            self.dht_handler.clone(),
            self.rec_tx.clone(),
            send_rx,
            shutdown_rx.clone(),
        );

        spawn_piece_downloader(
            peer,
            send_tx,
            self.rec_tx.clone(),
            downloaded_block_rx,
            download_new_piece_rx,
            self.pending_pieces.clone(),
            unchoke_waiter,
            shutdown_rx,
        );
    }

    pub fn disconnect(&mut self, peer: &Peer) {
        let Some(peer_con) = self.peers.remove(peer) else {
            return;
        };

        peer_con.shutdown_tx.send();
    }

    pub async fn recv(&mut self) -> ConnectionMessage {
        // None is impossible because one copy of tx is stored in self
        let msg = self.rec.recv().await.unwrap();

        if let Some(peer_con) = self.peers.get(&msg.peer) {
            match &msg.message {
                ConMessageType::Choke => peer_con.choker.choke(),
                ConMessageType::Unchoke => peer_con.choker.unchoke(),
                _ => (),
            }
        }

        msg
    }

    pub async fn download_piece(
        &mut self,
        peer: Peer,
        piece_idx: usize,
        piece_hash: ID,
    ) -> Result<()> {
        match self.peers.get(&peer) {
            None => bail!("peer {:?} is not connected", peer),
            Some(peer_connection) => {
                peer_connection
                    .download_new_piece_tx
                    .send(PeerManagerCommand::StartPiece((piece_hash, piece_idx)))
                    .await?;

                self.pending_pieces.add_piece(piece_idx);
            }
        }

        Ok(())
    }

    pub async fn choke(&self, yn: bool, peer: Peer) -> Result<()> {
        let message = if yn { Message::Choke } else { Message::Unchoke };

        self.send_message(peer, message).await
    }

    pub async fn interested(&self, yn: bool, peer: Peer) -> Result<()> {
        let message = if yn {
            Message::Interested
        } else {
            Message::NotInterested
        };

        self.send_message(peer, message).await
    }

    pub async fn have(&self, peer: Peer, v: u32) -> Result<()> {
        let message = Message::Have(v);
        self.send_message(peer, message).await
    }

    pub async fn bitfield(&self, peer: Peer, v: NoSizeBytes) -> Result<()> {
        let message = Message::Bitfield(v);
        self.send_message(peer, message).await
    }

    pub async fn piece(&self, peer: Peer, v: Piece) -> Result<()> {
        let message = Message::Piece(v);
        self.send_message(peer, message).await
    }

    pub async fn cancel(&self, peer: Peer, v: Request) -> Result<()> {
        let message = Message::Cancel(v);
        self.send_message(peer, message).await
    }

    pub async fn port(&self, peer: Peer, v: u16) -> Result<()> {
        let message = Message::Port(v);
        self.send_message(peer, message).await
    }

    async fn send_message(&self, peer: Peer, message: Message) -> Result<()> {
        match self.peers.get(&peer) {
            None => bail!("peer {:?} is not connected", peer),
            Some(peer_connection) => peer_connection.send_tx.send(message).await?,
        }

        Ok(())
    }

    // pub async fn send(&self, message: Message) -> Result<()> {
    //     Ok(self.manager_tx.send(message).await?)
    // }
}
