pub mod config;
pub mod constants;
pub mod metainfo;
pub mod peer;
pub mod peer_message;
pub mod tracker;

use core::time;
use std::thread::sleep;

use anyhow::Result;
use tokio::io::AsyncWriteExt;
// use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    // env::set_var("RUST_BACKTRACE", "1");
    let peer_id: [u8; 20] = rand::random();

    let config = config::Config::new();
    let torrent = metainfo::from_file(&config.file);

    let tracker_responses = tracker::announce(&torrent).await?;

    dbg!(&tracker_responses);

    let sa = tracker_responses[2].peers[1];

    dbg!(&sa);

    let mut stream = peer::initiate_handshake(&sa, &torrent.info_hash, &peer_id).await?;

    let message = peer_message::Message::Interested;

    stream.write_all(message.into_bytes()?.as_ref()).await?;

    let ten_millis = time::Duration::from_millis(1000);

    sleep(ten_millis);
    Ok(())
}
