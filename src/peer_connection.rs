pub mod handshake;
pub mod pending_pieces;

use self::handshake::initiate_handshake;
use crate::data_structures::id::ID;
use crate::data_structures::no_size_bytes::NoSizeBytes;
use crate::peer_message::{Message, Piece, Request};
use anyhow::Result;
use std::net::SocketAddr;
use tokio::io::{split, AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::select;
use tokio::sync::mpsc;
use tokio::time::{interval, Duration};
use tracing::{error, warn};

pub const BLOCK_SIZE: u32 = 1 << 14;
pub const DESCRIPTIVE_DATA_SIZE: usize = 1 << 7;
pub const MAX_MESSAGE_BYTES: usize = BLOCK_SIZE as usize + DESCRIPTIVE_DATA_SIZE;
pub const KEEP_ALIVE_INTERVAL_SECS: u64 = 100;
pub const MAX_CONCURRENT_REQUESTS: u32 = 3;
const MESSAGE_CHANNEL_BUFFER: usize = 1 << 5;

pub struct PeerConnection {
    manager_tx: mpsc::Sender<Message>,
}

impl PeerConnection {
    pub fn new(
        peer_id: ID,
        peer_address: SocketAddr,
        info_hash: ID,
        managers: ManagerChannels,
    ) -> Self {
        let (send_tx, send_rx) = mpsc::channel(MESSAGE_CHANNEL_BUFFER);

        let send_tx_clone = send_tx.clone();
        tokio::spawn(async move {
            manage_tcp(
                peer_id,
                peer_address,
                info_hash,
                managers,
                send_tx_clone,
                send_rx,
            )
            .await;
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
    peer_id: ID,
    peer_address: SocketAddr,
    info_hash: ID,
    managers: ManagerChannels,
    send_tx: mpsc::Sender<Message>,
    send_rx: mpsc::Receiver<Message>,
) {
    let mut stream = match initiate_handshake(&peer_address, &info_hash, &peer_id).await {
        Ok(s) => s,
        Err(e) => {
            let _ = managers.connection.send(ConnectionMessage::Unreachable);
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
        rv = manager_receiver( read_stream, &managers) => {rv},
    };

    let _ = managers.connection.send(ConnectionMessage::Disconnected);
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

async fn manager_receiver(mut stream: ReadHalf<&mut TcpStream>, managers: &ManagerChannels) {
    let mut buf = vec![0u8; MAX_MESSAGE_BYTES];
    loop {
        let Some(message) = read_message(&mut buf,&mut stream).await else{
                continue;
            };

        // TODO do something with this res
        let _ = match &message {
            Message::KeepAlive => Ok(()), // TODO impl timeout
            Message::Choke => managers.connection.send(message.into()).await,
            Message::Unchoke => managers.connection.send(message.into()).await,
            Message::Interested => managers.connection.send(message.into()).await,
            Message::NotInterested => managers.connection.send(message.into()).await,
            Message::Have(_) => managers.peer_data.send(message.into()).await,
            Message::Bitfield(_) => managers.peer_data.send(message.into()).await,
            Message::Request(_) => managers.uploader.send(message.into()).await,
            Message::Piece(_) => managers.downloader.send(message.into()).await,
            Message::Cancel(_) => Ok(()), // TODO but nut really unless you're trying to implement end game algorithm
            Message::Port(_) => managers.dht.send(message.into()).await,
        };
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

#[derive(Clone)]
pub struct ManagerChannels {
    pub connection: mpsc::Sender<ConnectionMessage>,
    pub peer_data: mpsc::Sender<ConnectionMessage>,
    pub downloader: mpsc::Sender<ConnectionMessage>,
    pub uploader: mpsc::Sender<ConnectionMessage>,
    pub dht: mpsc::Sender<ConnectionMessage>,
}

#[derive(Clone)]
pub enum ConnectionMessage {
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

impl From<Message> for ConnectionMessage {
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

        if let Err(_) = conn_send_tx.send(Message::KeepAlive).await {
            break;
        }
    }
}
