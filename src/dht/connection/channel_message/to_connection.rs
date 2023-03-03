use super::FromConResp;
use crate::data_structures::ID;
use crate::dht::connection::QueryId;
use crate::dht::krpc_message::{rand_tid, Message, Nodes, ValuesOrNodes};
use bytes::Bytes;
use std::net::SocketAddrV4;
use tokio::sync::mpsc;

#[derive(Debug)]
pub enum ToCon {
    Query {
        resp_returner: mpsc::Sender<FromConResp>,
        target: SocketAddrV4,
        variant: ToConQuery,
    },
    Resp {
        query_id: QueryId,
        variant: ToConResp,
    },
}

#[derive(Debug)]
pub enum ToConQuery {
    Ping,
    FindNode {
        target: ID,
    },
    GetPeers {
        info_hash: ID,
    },
    AnnouncePeer {
        info_hash: ID,
        port: u16,
        token: Bytes,
    },
}

#[derive(Debug)]
pub enum ToConResp {
    Ping,
    FindNode { nodes: Nodes },
    GetPeers { values_or_nodes: ValuesOrNodes },
    AnnouncePeer,
}

impl ToConQuery {
    pub fn into_message(self, own_node_id: &ID) -> Message {
        let transaction_id = rand_tid();
        match self {
            Self::Ping => Message::ping_query(own_node_id, transaction_id),
            Self::FindNode { target } => {
                Message::find_nodes_query(own_node_id, &target, transaction_id)
            }
            Self::GetPeers { info_hash } => {
                Message::get_peers_query(own_node_id, &info_hash, transaction_id)
            }
            Self::AnnouncePeer {
                info_hash,
                port,
                token,
            } => Message::announce_peer_query(own_node_id, &info_hash, port, token, transaction_id),
        }
    }
}

impl ToConResp {
    pub fn into_message(self, own_node_id: &ID, transaction_id: Bytes, token: Bytes) -> Message {
        match self {
            ToConResp::Ping => Message::ping_resp(own_node_id, transaction_id),
            ToConResp::FindNode { nodes } => {
                Message::find_nodes_resp(own_node_id, nodes, transaction_id)
            }
            ToConResp::GetPeers { values_or_nodes } => {
                Message::get_peers_resp(own_node_id, token, values_or_nodes, transaction_id)
            }
            ToConResp::AnnouncePeer => Message::announce_peer_resp(own_node_id, transaction_id),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddrV4;

    use super::{ToConQuery, ToConResp};
    use crate::{
        data_structures::ID,
        dht::{
            krpc_message::{rand_tid, Message, Nodes},
            routing_table::Node,
        },
    };
    use bytes::Bytes;

    #[test]
    fn query_into_message() {
        let info_hash = ID::new(rand::random());
        let own_node_id = ID::new(rand::random());

        let query_command = ToConQuery::GetPeers {
            info_hash: info_hash.to_owned(),
        };

        let message = query_command.into_message(&own_node_id);

        let exp_message = Message::get_peers_query(&own_node_id, &info_hash, rand_tid());

        // transaction_id is going to be different
        assert_eq!(
            exp_message.into_bytes().unwrap()[..70],
            message.into_bytes().unwrap()[..70]
        );
    }

    #[test]
    fn resp_into_message() {
        let own_node_id = ID::new(rand::random());
        let secret = ID::new(rand::random());

        let tid = Bytes::from_static(b"bytes");
        let nodes = Nodes::Exact(
            Node::from_compact_bytes("rdYAxWC9Zi!A97zKJUbH9HVcgP".as_bytes()).unwrap(),
        );

        let resp_command = ToConResp::FindNode {
            nodes: nodes.to_owned(),
        };
        let token = secret.hash_as_bytes(
            &"127.0.0.1:6969"
                .parse::<SocketAddrV4>()
                .unwrap()
                .ip()
                .octets(),
        );

        let message = resp_command.into_message(&own_node_id, tid.clone(), token);

        let exp_message = Message::find_nodes_resp(&own_node_id, nodes, tid);

        assert_eq!(
            exp_message.into_bytes().unwrap(),
            message.into_bytes().unwrap()
        );
    }
}
