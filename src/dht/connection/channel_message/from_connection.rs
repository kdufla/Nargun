use crate::data_structures::{NoSizeBytes, ID};
use crate::dht::connection::QueryId;
use crate::dht::krpc_message::{Nodes, ValuesOrNodes};
use crate::peers::peer::Peer;
use std::net::SocketAddrV4;

#[derive(Debug)]
pub enum FromCon {
    Query(FromConQuery),
    Resp(FromConResp),
}

#[derive(Debug)]
pub enum FromConQuery {
    FindNode { target: ID, query_id: QueryId },
    GetPeers { info_hash: ID, query_id: QueryId },
    NewPeer { new_peer: Peer, info_hash: ID },
}

#[derive(Debug)]
pub enum FromConResp {
    Touch {
        id: ID,
        from: SocketAddrV4,
    },
    NewNodes {
        nodes: Nodes,
        from: SocketAddrV4,
        id: ID,
    },
    GetPeers {
        from: SocketAddrV4,
        token: NoSizeBytes,
        v_or_n: ValuesOrNodes,
    },
}
