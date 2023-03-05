mod capped_growing_interval;
mod config;
mod data_structures;
mod dht;
mod fs;
mod gateway_device;
mod macros;
mod peers;
mod shutdown;
mod torrent_manager;
mod tracker;
mod transcoding;

use crate::peers::active_peers::Peers;
use anyhow::Result;
use data_structures::ID;
use dht::start_dht;
use std::sync::{atomic::AtomicU64, Arc};
use tokio::{
    signal,
    sync::{broadcast, mpsc},
};
use torrent_manager::start_client;
use tracing::info;
use transcoding::metainfo::Torrent;

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
        .with(tracing_subscriber::fmt::layer().with_line_number(true))
        // .with(...)
        .init();

    // let subscriber = tracing_subscriber::FmtSubscriber::new();
    // // let subscriber = console_subscriber::init();
    // tracing::subscriber::set_global_default(subscriber)?;
    // console_subscriber::init();

    // env::set_var("RUST_BACKTRACE", "1");
    let own_peer_id = ID::new(rand::random());

    let (shutdown_tx, shutdown_rx) = shutdown::channel();

    let config = config::Config::new();
    let torrent = Torrent::from_file(&config.file).unwrap();
    let peers = Peers::new(&torrent.info_hash);
    let (tcp_port, _udp_port) = gateway_device::open_any_port()?;
    let (dht_tx, dht_rx) = mpsc::channel(1 << 5);
    let pieces_downloaded = Arc::new(AtomicU64::new(0));

    start_dht(
        tcp_port,
        peers.to_owned(),
        torrent.info_hash,
        dht_rx,
        shutdown_rx.clone(),
    )
    .await;

    let (that_unknown_tx, _rx) = broadcast::channel(1 << 5);
    start_client(
        own_peer_id,
        peers,
        dht_tx,
        torrent,
        pieces_downloaded,
        &that_unknown_tx,
        tcp_port,
        shutdown_rx.clone(),
    );

    // let _ = fs::FS::new("base_dir".to_string(), &torrent.info).await;

    // if let Mode::Multi { files } = &torrent.info.mode {
    //     let now = Instant::now();
    //     // let x = build_dir_tree("/home/gvelesa/tmp/", "/asimov/", files).await;
    //     let elapsed = now.elapsed();
    //     // debug!(?x);
    //     debug!(?elapsed);
    // }

    // debug!(?directories);

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

    drop(shutdown_rx);

    match signal::ctrl_c().await {
        Ok(()) => {
            info!("shutting down");
            let active_task_waiter = shutdown_tx.send();
            active_task_waiter.wait().await;
        }
        Err(err) => {
            eprintln!("Unable to listen for shutdown signal: {err}");
        }
    }

    // sleep(time::Duration::from_secs(1000)).await;

    Ok(())
}
