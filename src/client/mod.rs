mod manager;
mod peer;
mod tracker;

use std::sync::{atomic::AtomicU64, Arc};

use crate::{data_structures::ID, transcoding::metainfo::Torrent};
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
    info_hash: ID,
    piece_length: usize,
    number_of_pieces: usize,
    peers: Peers,
    dht_tx: mpsc::Sender<ConnectionMessage>, // TODO connection is small enough, dht doesn't need to know about whole peer message, this must be a sockaddr
    torrent: Torrent,
    peer_id: &ID,
    pieces_downloaded: Arc<AtomicU64>,
    tx: &broadcast::Sender<bool>,
    tcp_port: u16,
) {
    tracker::spawn_tracker_managers(&torrent, peer_id, &peers, pieces_downloaded, tx, tcp_port);

    manager::TorrentManager::spawn(
        torrent,
        client_id,
        info_hash,
        piece_length,
        number_of_pieces,
        peers,
        dht_tx,
    );
}
