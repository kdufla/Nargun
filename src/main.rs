pub mod config;
pub mod constants;
pub mod metainfo;
pub mod peer;
pub mod peer_connection;
pub mod peer_message;
pub mod tracker;
pub mod util;

use core::time;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use anyhow::Result;
use constants::ID_LEN;
use peer_connection::{
    command::{Command, CommandType},
    info::PeerConnectionInfo,
    PeerConnection,
};
use tokio::{sync::mpsc, time::sleep};
use util::{bitmap::Bitmap, id::ID};
// use peer::Peers;

#[tokio::main]
async fn main() -> Result<()> {
    // env::set_var("RUST_BACKTRACE", "1");
    let _peer_id: [u8; ID_LEN] = rand::random();
    let peer_id = ID(_peer_id);

    let config = config::Config::new();
    let torrent = metainfo::from_file(&config.file);

    // let peers = Peers::new(torrent.count_pieces());

    // tracker::tracker_manager(&torrent, &peer_id, &peers).await;

    // dbg!(&tracker_responses);

    // let sa = tracker_responses[2].peers[1];

    // dbg!(&torrent.info_hash);

    // let mut stream = peer::initiate_handshake(&sa, &torrent.info_hash, &peer_id).await?;

    // let message = peer_message::Message::Interested;

    // stream.write_all(message.into_bytes()?.as_ref()).await?;

    // let ten_millis = time::Duration::from_millis(1000);

    // sleep(time::Duration::from_secs(1000)).await;

    // struct X {
    //     a: i32,
    //     b: i32,
    // }

    // async fn x(n: &mut i32) {
    //     println!("{}", n);
    // }

    // let xx = X { a: 1, b: 2 };

    // select! {
    //     rv = x(&mut xx.a) => {rv},
    //     rv = x(&mut xx.b) => {rv},
    // };

    // let info_hash = ID([
    //     0x3b, 0x24, 0x55, 0x04, 0xcf, 0x5f, 0x11, 0xbb, 0xdb, 0xe1, 0x20, 0x1c, 0xea, 0x6a, 0x6b,
    //     0xf4, 0x5a, 0xee, 0x1b, 0xc0,
    // ]);

    // let peer_ip = Ipv4Addr::new(185, 255, 237, 37);
    // let peer_port = 48536;

    let peer_ip = Ipv4Addr::new(5, 135, 157, 164);
    let peer_port = 51413;

    let peer_sa = SocketAddr::new(IpAddr::V4(peer_ip), peer_port);

    let info = PeerConnectionInfo::new();

    let pieces = Bitmap::new(torrent.info.number_of_pieces() as u32);

    let (tx, rx) = mpsc::channel(32);

    let pc = PeerConnection::new(
        peer_id,
        peer_sa,
        ID(torrent.info_hash),
        info,
        torrent.info.piece_length as u32,
        pieces,
        tx,
        rx,
    );

    loop {
        let (command, rx) = Command::new(CommandType::Request(0));
        if let Ok(_) = pc.command_sender.send(command).await {
            match rx.await {
                Ok(r) => {
                    println!("rx in main: {r}");
                }
                Err(e) => {
                    println!("rx.err in main: {e}");
                }
            }
        }
        sleep(time::Duration::from_secs(1000)).await;
    }

    // Ok(())
}
