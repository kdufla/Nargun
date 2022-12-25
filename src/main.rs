pub mod config;
pub mod constants;
pub mod dht;
pub mod metainfo;
pub mod peer;
pub mod peer_connection;
pub mod peer_message;
pub mod tracker;
pub mod util;

use anyhow::Result;
use constants::ID_LEN;
use core::time;
use dht::{dht, routing_table::RoutingTable};
use peer::Peers;
use peer_connection::{
    command::{Command, CommandType},
    info::PeerConnectionInfo,
    PeerConnection,
};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4},
    sync::{atomic::AtomicU64, Arc},
    time::Duration,
};
use tokio::sync::broadcast;
use tokio::{sync::mpsc, time::sleep};
use util::{bitmap::Bitmap, id::ID};

#[tokio::main]
async fn main() -> Result<()> {
    // env::set_var("RUST_BACKTRACE", "1");
    let _peer_id: [u8; ID_LEN] = rand::random();
    let peer_id = ID(_peer_id);

    let config = config::Config::new();
    let torrent = metainfo::from_file(&config.file);

    // torrent: &Torrent,
    // peer_id: &[u8; 20],
    // peers: Peers,
    // pieces_downloaded: Arc<AtomicU64>,
    // tx: &broadcast::Sender<bool>,

    let peers = Peers::new();
    let (tx, rx) = mpsc::channel(12);

    dht(peers, ID(torrent.info_hash.clone()), rx).await;
    // let pieces_downloaded = Arc::new(AtomicU64::new(0));
    // let (tx, _) = broadcast::channel(3);

    // tracker::spawns_tracker_managers(&torrent, &peer_id, &peers, pieces_downloaded, &tx).await;

    // sleep(Duration::from_secs(10)).await;

    // let _ = tx.send(false);

    // sleep(Duration::from_secs(300)).await;

    // let peer_ip = Ipv4Addr::new(5, 135, 157, 164);
    // let peer_port = 51413;

    // let peer_sa = SocketAddr::new(IpAddr::V4(peer_ip), peer_port);
    // // let peer_sa = peers.get_random().unwrap();

    // let info = PeerConnectionInfo::new();

    // let pieces = Bitmap::new(torrent.info.number_of_pieces() as u32);

    // let (tx, rx) = mpsc::channel(32);

    // let pc = PeerConnection::new(
    //     peer_id,
    //     peer_sa,
    //     ID(torrent.info_hash),
    //     info,
    //     torrent.info.piece_length as u32,
    //     pieces,
    //     tx,
    //     rx,
    // );

    // loop {
    //     let (command, rx) = Command::new(CommandType::Request(0));
    //     if let Ok(_) = pc.command_sender.send(command).await {
    //         match rx.await {
    //             Ok(r) => {
    //                 println!("rx in main: {r}");
    //             }
    //             Err(e) => {
    //                 println!("rx.err in main: {e}");
    //             }
    //         }
    //     }
    //     sleep(time::Duration::from_secs(1000)).await;
    // }

    Ok(())
}
