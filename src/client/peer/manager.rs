use tokio::{select, sync::mpsc};

use super::connection::{Connection, ConnectionMessage, Message, PendingPieces};

pub async fn manage_peer(
    mut peer_manager_rx: mpsc::Receiver<usize>,
    pending_pieces: PendingPieces,
    connection: Connection,
    mut download_manager_rx: mpsc::Receiver<ConnectionMessage>,
) {
    // get commands with peer_manager_rx to download pieces
    // get blocks from pending_pieces
    // send requests with connection
    // get downloaded blocks from download_manager_rx

    let _blocks = pending_pieces.take_not_good_blocks(&[1, 2, 3], 2);

    let message = Message::KeepAlive;
    let _ = connection.send(message).await;

    select! {
        rv = download_manager_rx.recv() => {},
        rv = peer_manager_rx.recv() => {},
    };
}
