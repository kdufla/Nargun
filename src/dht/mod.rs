mod connection;
mod dht_manager;
mod krpc_message;
mod peer_fetcher;
mod routing_table;

use tracing::error;

use crate::{client::Peers, data_structures::ID, shutdown};
use std::net::SocketAddrV4;

pub async fn start_dht(
    tcp_port: u16,
    peers: Peers,
    info_hash: ID,
    peer_with_dht: tokio::sync::mpsc::Receiver<SocketAddrV4>,
    shutdown_rx: shutdown::Receiver,
) {
    let routing_table = routing_table::RoutingTable::new().await;

    let connection = connection::Connection::new(*routing_table.own_id());

    tokio::spawn(async move {
        let res = dht_manager::dht(
            info_hash,
            tcp_port,
            routing_table,
            peers,
            peer_with_dht,
            shutdown_rx,
            connection,
        )
        .await;

        if let Err(e) = res {
            error!(?e);
        }
    });
}
