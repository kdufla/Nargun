use super::handshake::initiate_handshake;
use super::message::{Message, Piece, Request, BYTES_IN_LEN};
use crate::data_structures::NoSizeBytes;
use crate::data_structures::ID;
use crate::peers::peer::Peer;
use anyhow::Result;
use std::mem::discriminant;
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

#[instrument(skip_all, fields(peer = peer.to_string()))]
pub async fn manage_tcp(
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
            warn!(?e);
            if let Err(e) = manager_messenger.send(ConMessageType::Unreachable).await {
                warn!(?e);
            }
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

    if let Err(e) = manager_messenger.send(ConMessageType::Disconnected).await {
        warn!(?e);
    }
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

async fn manager_receiver(mut stream: ReadHalf<&mut TcpStream>, manager_messenger: &MessageSender) {
    let mut buf = vec![0u8; MAX_MESSAGE_BYTES];
    let mut data_start = 0;
    let mut data_end = 0;

    loop {
        let tmp = match stream.read(&mut buf[data_end..]).await {
            Ok(len) => len,
            Err(e) => {
                error!(?e);
                return;
            }
        };

        // TODO remove these comments. for now, we might still need detailed logs here

        // debug!("read {tmp} bytes");
        data_end += tmp;

        let mut data_len = data_end - data_start;

        // debug!("cur data len {:?} ({}..{})", data_len, data_start, data_end);

        while data_len >= BYTES_IN_LEN {
            let message_len = Message::len(&buf[data_start..]) + BYTES_IN_LEN;
            // debug!(message_len);

            // if data_len > 128 {
            //     debug!("[start..+64]{:?}", &buf[data_start..data_start + 64]);
            //     debug!("[-64..end]{:?}", &buf[data_end - 64..data_end]);
            // } else {
            //     debug!("{:?}", &buf[data_start..data_start + message_len]);
            // }

            if message_len <= data_len {
                if let Some(message) = Message::from_buf(&buf) {
                    trace!("received msg {:?}", discriminant(&message));
                    if let Err(e) = manager_messenger.send(message.into()).await {
                        warn!(?e);
                    }
                };

                data_start += message_len;
                data_len -= message_len;

                // debug!("left data {}..{}", data_start, data_end);
                if data_end > data_start {
                    buf.copy_within(data_start..data_end, 0);
                }

                data_end -= data_start;
                data_start = 0;
            } else {
                break;
            }
        }
    }
}

struct MessageSender {
    peer: Peer,
    managers: ManagerChannels,
}

impl MessageSender {
    async fn send(&self, message: ConMessageType) -> Result<()> {
        let m = ConnectionMessage {
            peer: self.peer,
            message,
        };

        // TODO do something with this
        match m.message {
            ConMessageType::Unreachable => self.managers.connection_status.send(m).await?,
            ConMessageType::Disconnected => self.managers.connection_status.send(m).await?,
            ConMessageType::KeepAlive => (), // TODO impl timeout
            ConMessageType::Choke => self.managers.connection_status.send(m).await?,
            ConMessageType::Unchoke => self.managers.connection_status.send(m).await?,
            ConMessageType::Interested => self.managers.connection_status.send(m).await?,
            ConMessageType::NotInterested => self.managers.connection_status.send(m).await?,
            ConMessageType::Have(_) => self.managers.peer_bitmap.send(m).await?,
            ConMessageType::Bitfield(_) => self.managers.peer_bitmap.send(m).await?,
            ConMessageType::Request(_) => self.managers.uploader.send(m).await?,
            ConMessageType::Piece(_) => self.managers.downloader.send(m).await?,
            ConMessageType::Cancel(_) => (), // TODO but nut really unless you're trying to implement end game algorithm
            ConMessageType::Port(port) => {
                let mut peer_addr = m.peer.addr().to_owned();
                peer_addr.set_port(port);
                self.managers.dht.send(peer_addr).await?
            }
        };

        Ok(())
    }
}

#[derive(Clone)]
pub struct ManagerChannels {
    pub connection_status: mpsc::Sender<ConnectionMessage>,
    pub peer_bitmap: mpsc::Sender<ConnectionMessage>,
    pub downloader: mpsc::Sender<ConnectionMessage>,
    pub uploader: mpsc::Sender<ConnectionMessage>,
    pub dht: mpsc::Sender<SocketAddrV4>,
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
