use super::handshake::initiate_handshake;
use super::message::{Message, Piece, Request, BYTES_IN_LEN_PREFIX};
use super::pending_pieces::FinishedPiece;
use crate::data_structures::SerializableBuf;
use crate::data_structures::ID;
use crate::peers::peer::Peer;
use crate::shutdown;
use anyhow::bail;
use std::net::SocketAddrV4;
use tokio::io::{split, AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::select;
use tokio::sync::mpsc;
use tokio::time::{interval, Duration};
use tracing::log::trace;
use tracing::{error, instrument, warn};

pub const BLOCK_SIZE: usize = 1 << 14;
pub const DESCRIPTIVE_DATA_SIZE: usize = 1 << 7;
pub const MAX_MESSAGE_BYTES: usize = BLOCK_SIZE + DESCRIPTIVE_DATA_SIZE;
pub const KEEP_ALIVE_INTERVAL_SECS: u64 = 100;

pub fn spawn_connection_manager(
    own_peer_id: ID,
    peer: Peer,
    info_hash: ID,
    piece_handler: mpsc::Sender<Piece>,
    dht_handler: mpsc::Sender<SocketAddrV4>,
    peer_handler: mpsc::Sender<ConnectionMessage>,
    send_rx: mpsc::Receiver<Message>,
    shutdown_rx: shutdown::Receiver,
) {
    tokio::spawn(async move {
        manage_tcp(
            own_peer_id,
            peer,
            info_hash,
            piece_handler,
            dht_handler,
            peer_handler,
            send_rx,
            shutdown_rx,
        )
        .await;
    });
}

#[instrument(skip_all, fields(peer = peer.to_string()))]
async fn manage_tcp(
    own_peer_id: ID,
    peer: Peer,
    info_hash: ID,
    piece_handler: mpsc::Sender<Piece>,
    dht_handler: mpsc::Sender<SocketAddrV4>,
    peer_handler: mpsc::Sender<ConnectionMessage>,
    send_rx: mpsc::Receiver<Message>,
    mut shutdown_rx: shutdown::Receiver,
) {
    let Ok(mut stream) = initiate_handshake(&peer.into(), &info_hash, &own_peer_id).await else {
        // I can ignore it because if rec is dead, there's not much to do other than shutting down
        let _ = peer_handler
            .send(ConnectionMessage {
                peer,
                message: ConMessageType::Unreachable,
            })
            .await;
        return;
    };

    let (read_stream, write_stream) = split(&mut stream);

    select! {
        _ = shutdown_rx.recv() => {},
        _ = manager_sender(write_stream, send_rx) => {},
        _ = manager_receiver(peer, read_stream, piece_handler, dht_handler, peer_handler) => {},
    };
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

        trace!("send {:?}", message);

        if let Err(e) = stream.write_all(message.into_bytes().as_ref()).await {
            error!(?e);
        }
    }
}

async fn manager_receiver(
    peer: Peer,
    mut stream: ReadHalf<&mut TcpStream>,
    piece_handler: mpsc::Sender<Piece>,
    dht_handler: mpsc::Sender<SocketAddrV4>,
    peer_handler: mpsc::Sender<ConnectionMessage>,
) {
    let mut buf = vec![0u8; MAX_MESSAGE_BYTES];
    let mut data_start = 0;
    let mut data_len = 0;

    loop {
        data_len += match stream.read(&mut buf[data_start + data_len..]).await {
            Ok(len) => len,
            Err(e) => {
                error!(?e);
                return;
            }
        };

        while data_len >= BYTES_IN_LEN_PREFIX {
            let message_len = Message::len(&buf[data_start..]) + BYTES_IN_LEN_PREFIX;

            if message_len <= data_len {
                let Some(message) = Message::from_buf(&buf) else {
                    error!("message_len <= data_len but buf can't be deserialize into message");
                    data_start = 0;
                    data_len = 0;
                    break;
                };

                data_start += message_len;
                data_len -= message_len;

                if data_len > 0 {
                    buf.copy_within(data_start..data_start + data_len, 0);
                }

                data_start = 0;

                handle_message(peer, message, &piece_handler, &dht_handler, &peer_handler).await;
            } else {
                break;
            }
        }
    }
}

async fn handle_message(
    peer: Peer,
    message: Message,
    piece_handler: &mpsc::Sender<Piece>,
    dht_handler: &mpsc::Sender<SocketAddrV4>,
    peer_handler: &mpsc::Sender<ConnectionMessage>,
) {
    match message {
        Message::Piece(piece) => {
            piece_handler.send(piece).await;
        }
        Message::Port(port) => {
            let mut peer_addr = *peer.addr();
            peer_addr.set_port(port);
            dht_handler.send(peer_addr).await;
        }
        _ => {
            let message = match message.try_into() {
                Ok(m) => m,
                Err(e) => {
                    error!(?e);
                    return;
                }
            };

            peer_handler.send(ConnectionMessage { peer, message }).await;
        }
    }
}

#[derive(Debug)]
pub struct ConnectionMessage {
    pub peer: Peer,
    pub message: ConMessageType,
}

#[derive(Debug)]
pub enum ConMessageType {
    Unreachable,
    Disconnected,
    KeepAlive,
    Choke,
    Unchoke,
    Interested,
    NotInterested,
    Have(u32),
    Bitfield(SerializableBuf),
    Request(Request),
    FinishedPiece(FinishedPiece),
    Cancel(Request),
}

impl TryFrom<Message> for ConMessageType {
    type Error = anyhow::Error;

    fn try_from(value: Message) -> std::result::Result<Self, Self::Error> {
        let rv = match value {
            Message::KeepAlive => Self::KeepAlive,
            Message::Choke => Self::Choke,
            Message::Unchoke => Self::Unchoke,
            Message::Interested => Self::Interested,
            Message::NotInterested => Self::NotInterested,
            Message::Have(v) => Self::Have(v),
            Message::Bitfield(v) => Self::Bitfield(v),
            Message::Request(v) => Self::Request(v),
            Message::Piece(v) => bail!("piece is not handled receiver of this msg"),
            Message::Cancel(v) => Self::Cancel(v),
            Message::Port(v) => bail!("port is not handled receiver of this msg"),
        };

        Ok(rv)
    }
}

async fn _keep_alive_sender(conn_send_tx: mpsc::Sender<Message>) {
    let mut interval = interval(Duration::from_secs(KEEP_ALIVE_INTERVAL_SECS));
    interval.tick().await;

    loop {
        interval.tick().await;

        if conn_send_tx.send(Message::KeepAlive).await.is_err() {
            break;
        }
    }
}
