pub const ANNOUNCE_RETRY: u8 = 3;
pub const BLOCK_SIZE: u32 = 16384;
pub const COMPACT_NODE_LEN: usize = ID_LEN + COMPACT_SOCKADDR_LEN;
pub const COMPACT_SOCKADDR_LEN: usize = 6;
pub const COMPACT_PEER_LEN: usize = COMPACT_SOCKADDR_LEN;
pub const ID_LEN: usize = 20;
pub const KEEP_ALIVE_INTERVAL_SECS: u64 = 100;
pub const MAX_CONCURRENT_REQUESTS: u32 = 3;
