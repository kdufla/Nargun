use super::handshake::initiate_handshake;
use super::message::{Message, Piece, Request};
use crate::client::Peer;
use crate::data_structures::NoSizeBytes;
use crate::data_structures::ID;
use anyhow::Result;
use tokio::io::{split, AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::select;
use tokio::sync::mpsc;
use tokio::time::{interval, Duration};
use tracing::{error, warn};

pub const BLOCK_SIZE: usize = 1 << 14;
pub const DESCRIPTIVE_DATA_SIZE: usize = 1 << 7;
pub const MAX_MESSAGE_BYTES: usize = BLOCK_SIZE + DESCRIPTIVE_DATA_SIZE;
pub const KEEP_ALIVE_INTERVAL_SECS: u64 = 100;
pub const MAX_CONCURRENT_REQUESTS: u32 = 3;
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

async fn manage_tcp(
    client_id: ID,
    peer: Peer,
    info_hash: ID,
    managers: ManagerChannels,
    send_tx: mpsc::Sender<Message>,
    send_rx: mpsc::Receiver<Message>,
) {
    let manager_messenger = MessageSender {
        peer: peer.to_owned(),
        managers,
    };

    let mut stream = match initiate_handshake(&peer.into(), &info_hash, &client_id).await {
        Ok(s) => s,
        Err(e) => {
            manager_messenger.send(ConMessageType::Unreachable).await;
            warn!(?e);
            return;
        }
    };

    let (read_stream, write_stream) = split(&mut stream);

    tokio::spawn(async move {
        keep_alive_sender(send_tx).await;
    });

    select! {
        rv = manager_sender(write_stream, send_rx) => {rv},
        rv = manager_receiver( read_stream, &manager_messenger) => {rv},
    };

    manager_messenger.send(ConMessageType::Disconnected).await;
}

async fn manager_sender(
    mut stream: WriteHalf<&mut TcpStream>,
    mut send_rx: mpsc::Receiver<Message>,
) {
    loop {
        let Some(message) = send_rx.recv().await else {
                warn!("should this be how this manager exits? TODO and see when this happens");
                return;
            };

        if let Err(e) = stream.write_all(message.into_bytes().as_ref()).await {
            error!(?e);
        }
    }
}

async fn manager_receiver(mut stream: ReadHalf<&mut TcpStream>, manager_messenger: &MessageSender) {
    let mut buf = vec![0u8; MAX_MESSAGE_BYTES];
    loop {
        let Some(message) = read_message(&mut buf,&mut stream).await else{
            continue;
        };

        manager_messenger.send(message.into()).await;
    }
}

async fn read_message(buf: &mut [u8], stream: &mut ReadHalf<&mut TcpStream>) -> Option<Message> {
    let message_len = match stream.read(buf).await {
        Ok(len) => len,
        Err(e) => {
            error!(?e);
            return None;
        }
    };

    match Message::from_buf(buf, message_len) {
        Ok(m) => Some(m),
        Err(e) => {
            error!(?e);
            None
        }
    }
}

struct MessageSender {
    peer: Peer,
    managers: ManagerChannels,
}

impl MessageSender {
    async fn send(&self, message: ConMessageType) {
        let m = ConnectionMessage {
            peer: self.peer.clone(),
            message,
        };

        // TODO do something with this
        let _ = match m.message {
            ConMessageType::Unreachable => self.managers.connection_status.send(m).await,
            ConMessageType::Disconnected => self.managers.connection_status.send(m).await,
            ConMessageType::KeepAlive => Ok(()), // TODO impl timeout
            ConMessageType::Choke => self.managers.connection_status.send(m).await,
            ConMessageType::Unchoke => self.managers.connection_status.send(m).await,
            ConMessageType::Interested => self.managers.connection_status.send(m).await,
            ConMessageType::NotInterested => self.managers.connection_status.send(m).await,
            ConMessageType::Have(_) => self.managers.peer_bitmap.send(m).await,
            ConMessageType::Bitfield(_) => self.managers.peer_bitmap.send(m).await,
            ConMessageType::Request(_) => self.managers.uploader.send(m).await,
            ConMessageType::Piece(_) => self.managers.downloader.send(m).await,
            ConMessageType::Cancel(_) => Ok(()), // TODO but nut really unless you're trying to implement end game algorithm
            ConMessageType::Port(_) => self.managers.dht.send(m).await,
        };
    }
}

#[derive(Clone)]
pub struct ManagerChannels {
    pub connection_status: mpsc::Sender<ConnectionMessage>,
    pub peer_bitmap: mpsc::Sender<ConnectionMessage>,
    pub downloader: mpsc::Sender<ConnectionMessage>,
    pub uploader: mpsc::Sender<ConnectionMessage>,
    pub dht: mpsc::Sender<ConnectionMessage>,
}

#[derive(Clone, Debug)]
pub struct ConnectionMessage {
    pub peer: Peer,
    pub message: ConMessageType,
}

#[derive(Clone, Debug)]
pub enum ConMessageType {
    Unreachable,
    Disconnected,
    KeepAlive,
    Choke,
    Unchoke,
    Interested,
    NotInterested,
    Have(u32),
    Bitfield(NoSizeBytes),
    Request(Request),
    Piece(Piece),
    Cancel(Request),
    Port(u16),
}

impl From<Message> for ConMessageType {
    fn from(message: Message) -> Self {
        match message {
            Message::KeepAlive => Self::KeepAlive,
            Message::Choke => Self::Choke,
            Message::Unchoke => Self::Unchoke,
            Message::Interested => Self::Interested,
            Message::NotInterested => Self::NotInterested,
            Message::Have(v) => Self::Have(v),
            Message::Bitfield(v) => Self::Bitfield(v),
            Message::Request(v) => Self::Request(v),
            Message::Piece(v) => Self::Piece(v),
            Message::Cancel(v) => Self::Cancel(v),
            Message::Port(v) => Self::Port(v),
        }
    }
}

async fn keep_alive_sender(conn_send_tx: mpsc::Sender<Message>) {
    let mut interval = interval(Duration::from_secs(KEEP_ALIVE_INTERVAL_SECS));
    interval.tick().await;

    loop {
        interval.tick().await;

        if conn_send_tx.send(Message::KeepAlive).await.is_err() {
            break;
        }
    }
}
