mod client;
mod config;
mod data_structures;
mod dht;
// mod fs;
mod gateway_device;
mod macros;
mod transcoding;

use anyhow::Result;
use client::Peers;
use core::time;
use data_structures::ID;
use dht::start_dht;
// use fs::build_dir_tree;
use std::net::SocketAddrV4;
use tokio::{
    sync::mpsc,
    time::{sleep, Instant},
};
use tracing::{debug, error, info, warn};
use transcoding::metainfo::{Mode, Torrent};

// comments
// because
// vscode/rustfmt
// loves
// to
// fuck
// with
// tokio::main

#[tokio::main]
async fn main() -> Result<()> {
    use tracing_subscriber::prelude::*;

    // spawn the console server in the background,
    // returning a `Layer`:
    let console_layer = console_subscriber::spawn();

    // build a `Subscriber` by combining layers with a
    // `tracing_subscriber::Registry`:
    tracing_subscriber::registry()
        // add the console layer to the subscriber
        .with(console_layer)
        // add other layers...
        .with(tracing_subscriber::fmt::layer())
        // .with(...)
        .init();

    // let subscriber = tracing_subscriber::FmtSubscriber::new();
    // // let subscriber = console_subscriber::init();
    // tracing::subscriber::set_global_default(subscriber)?;
    // console_subscriber::init();

    // env::set_var("RUST_BACKTRACE", "1");
    // let peer_id = ID::new(rand::random());

    let config = config::Config::new();
    let torrent = Torrent::from_file(&config.file).unwrap();

    // let _ = fs::FS::new("base_dir".to_string(), &torrent.info).await;

    // if let Mode::Multi { files } = &torrent.info.mode {
    //     let now = Instant::now();
    //     // let x = build_dir_tree("/home/gvelesa/tmp/", "/asimov/", files).await;
    //     let elapsed = now.elapsed();
    //     // debug!(?x);
    //     debug!(?elapsed);
    // }

    // debug!(?directories);

    // let (tcp_port, udp_port) = gateway_device::open_any_port()?;
    // info!(?tcp_port, ?udp_port);

    // let addr: SocketAddrV4 = "44.242.152.222:8850".parse().unwrap();
    // let addr: SocketAddrV4 = "121.142.222.29:59493".parse().unwrap();
    // let addr: SocketAddrV4 = "77.254.210.215:36028".parse().unwrap();
    // torrent: &Torrent,
    // peer_id: &[u8; 20],
    // peers: Peers,
    // pieces_downloaded: Arc<AtomicU64>,
    // tx: &broadcast::Sender<bool>,

    // let peers = Peers::new(&torrent.info_hash);
    // let (tx, rx) = mpsc::channel(12);

    // tokio::spawn(async move {
    //     dht(peers, torrent.info_hash.clone(), rx).await;
    // });

    // let _ = tx.send(addr).await;
    // let pieces_downloaded = Arc::new(AtomicU64::new(0));
    // let (tx, _) = broadcast::channel(3);

    // tracker::spawns_tracker_managers(&torrent, &peer_id, &peers, pieces_downloaded, &tx).await;

    // sleep(Duration::from_secs(10)).await;

    // let _ = tx.send(false);

    // sleep(Duration::from_secs(300)).await;

    // let peer_ip = Ipv4Addr::new(5, 135, 157, 164);
    // let peer_port = LOCAL_PORT_TCP;

    // let peer_sa = SocketAddr::new(IpAddr::V4(peer_ip), peer_port);
    // // let peer_sa = peers.get_random().unwrap();

    // let info = PeerConnectionInfo::new();

    // let pieces = Bitmap::new(torrent.info.number_of_pieces() as u32);

    // let pc = PeerConnection::new(
    //     peer_id,
    //     peer_sa,
    //     ID::new(torrent.info_hash),
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
    // }
    sleep(time::Duration::from_secs(1000)).await;

    Ok(())
}
