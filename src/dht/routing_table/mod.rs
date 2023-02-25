mod bucket;
mod table;

use crate::{data_structures::ID_LEN, transcoding::COMPACT_SOCKADDR_LEN};

pub use bucket::Node;
pub use table::RoutingTable;

pub const COMPACT_NODE_LEN: usize = ID_LEN + COMPACT_SOCKADDR_LEN;
const K_NODE_PER_BUCKET: usize = 8;
