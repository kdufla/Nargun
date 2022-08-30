pub mod config;
pub mod metainfo;
pub mod tracker;

use crate::config::Config;
// use crate::metainfo;

use anyhow::Result;
// use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    // env::set_var("RUST_BACKTRACE", "1");

    let config = Config::new();
    let torrent = metainfo::from_file(&config.file);

    tracker::announce(torrent).await?;

    Ok(())
}
