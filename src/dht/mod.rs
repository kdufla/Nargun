mod connection;
mod dht_manager;
mod krpc_message;
mod routing_table;

use crate::{client::Peers, data_structures::ID, shutdown};
use std::net::SocketAddrV4;

pub fn start_dht(
    peers: Peers,
    info_hash: ID,
    peer_with_dht: tokio::sync::mpsc::Receiver<SocketAddrV4>,
    shutdown_rx: shutdown::Receiver,
) {
    tokio::spawn(async move {
        dht_manager::dht(peers, info_hash, peer_with_dht, shutdown_rx).await;
    });
}
