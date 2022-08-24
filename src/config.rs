use clap::Parser;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)] // Read from `Cargo.toml`
pub struct Config {
    /// metainfo (.torrent) file
    #[clap(short, long, value_parser)]
    pub file: String,
}

impl Config {
    pub fn new() -> Config {
        Config::parse()
    }
}
