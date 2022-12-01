pub const PSTRLEN: u8 = 19;
pub const PSTR: &str = "BitTorrent protocol";

pub const KEEP_ALIVE_INTERVAL_SECS: u64 = 100;
pub const BLOCK_SIZE: u32 = 16384;
pub const MAX_CONCURRENT_REQUESTS: u32 = 3;
pub const RETRY_COUNT: u32 = 3;
pub const HANDSHAKE_RETRY_LIMIT: u32 = 0;
pub const HANDSHAKE_LENGTH_FOR_BITTORRENT_PROTOCOL: usize = 68;
pub const ID_LEN: usize = 20;
