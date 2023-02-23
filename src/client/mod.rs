mod manager;
mod peer;
mod tracker;

use std::{
    net::SocketAddrV4,
    sync::{atomic::AtomicU64, Arc},
};

use crate::{data_structures::ID, shutdown, transcoding::metainfo::Torrent};
use tokio::sync::{broadcast, mpsc};

pub use peer::{
    connection::{FinishedPiece, BLOCK_SIZE}, // TODO block size being a u32 is not good
    Peer,
    Peers,
    COMPACT_PEER_LEN,
};

use self::peer::connection::ConnectionMessage;

// TODO probably before committing, figure this shit out!
pub fn start_client(
    client_id: ID,
    peers: Peers,
    dht_tx: mpsc::Sender<SocketAddrV4>,
    torrent: Torrent,
    pieces_downloaded: Arc<AtomicU64>,
    tx: &broadcast::Sender<()>,
    tcp_port: u16,
    shutdown_rx: shutdown::Receiver,
) {
    tracker::spawn_tracker_managers(
        &torrent,
        &client_id,
        &peers,
        pieces_downloaded,
        tx,
        tcp_port,
        shutdown_rx,
    );

    manager::TorrentManager::spawn(torrent, client_id, peers, dht_tx);
}
