use crate::data_structures::{id::ID, no_size_bytes::NoSizeBytes};
use crate::peer_connection::{pending_pieces::PendingPieces, ConnectionMessage, PeerConnection};
use crate::peer_message::Piece;
use std::net::SocketAddr;
use tokio::sync::mpsc;

pub async fn manage_peer(
    connection: PeerConnection,
    download_manager_rx: mpsc::Receiver<ConnectionMessage>,
    peer_manager_rx: mpsc::Receiver<usize>,
    pending_pieces: PendingPieces,
) {
    // get commands with peer_manager_rx to download pieces
    // get blocks from pending_pieces
    // send requests with connection
    // get downloaded blocks from download_manager_rx
}
