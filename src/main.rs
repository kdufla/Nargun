pub mod config;
pub mod metainfo;

use crate::config::Config;
use crate::metainfo::parse_benfile;

fn main() {
    let config = Config::new();
    let torrent = parse_benfile(&config.file);

    println!("{}", torrent);
}
