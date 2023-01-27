use crate::data_structures::ID;
use crate::dht::krpc_message::{Message, Nodes, ValuesOrNodes};
use bytes::Bytes;
use std::net::SocketAddrV4;

#[derive(Debug)]
pub struct ConCommand {
    pub command_type: CommandType,
    pub target: SocketAddrV4,
}

#[derive(Debug)]
pub enum CommandType {
    Query(QueryCommand),
    Resp(RespCommand, Bytes),
}

#[derive(Debug)]
pub enum QueryCommand {
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
pub enum RespCommand {
    Ping,
    FindNode { nodes: Nodes },
    GetPeers { values_or_nodes: ValuesOrNodes },
    AnnouncePeer,
}

impl ConCommand {
    pub fn new(command: CommandType, target: SocketAddrV4) -> Self {
        Self {
            command_type: command,
            target,
        }
    }

    pub fn into_message(self, own_node_id: &ID, secret: &ID) -> Message {
        match self.command_type {
            CommandType::Query(query_command) => query_command.into_message(own_node_id),
            CommandType::Resp(resp_command, transaction_id) => {
                let token = secret.hash_as_bytes(&self.target.ip().octets());
                resp_command.into_message(own_node_id, transaction_id, token)
            }
        }
    }
}

impl QueryCommand {
    fn into_message(self, own_node_id: &ID) -> Message {
        match self {
            Self::Ping => Message::ping_query(own_node_id),
            Self::FindNode { target } => Message::find_nodes_query(own_node_id, &target),
            Self::GetPeers { info_hash } => Message::get_peers_query(own_node_id, &info_hash),
            Self::AnnouncePeer {
                info_hash,
                port,
                token,
            } => Message::announce_peer_query(own_node_id, &info_hash, port, token),
        }
    }
}

impl RespCommand {
    fn into_message(self, own_node_id: &ID, transaction_id: Bytes, token: Bytes) -> Message {
        match self {
            RespCommand::Ping => Message::ping_resp(own_node_id, transaction_id),
            RespCommand::FindNode { nodes } => {
                Message::find_nodes_resp(own_node_id, nodes, transaction_id)
            }
            RespCommand::GetPeers { values_or_nodes } => {
                Message::get_peers_resp(own_node_id, token, values_or_nodes, transaction_id)
            }
            RespCommand::AnnouncePeer => Message::announce_peer_resp(own_node_id, transaction_id),
        }
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use crate::{
        data_structures::ID,
        dht::{
            connection::RespCommand,
            krpc_message::{Message, Nodes},
            routing_table::Node,
        },
    };

    use super::{CommandType, ConCommand, QueryCommand};

    #[test]
    fn query_into_message() {
        let info_hash = ID::new(rand::random());
        let own_node_id = ID::new(rand::random());
        let secret = ID::new(rand::random());

        let query_command = ConCommand::new(
            CommandType::Query(QueryCommand::GetPeers {
                info_hash: info_hash.to_owned(),
            }),
            "127.0.0.1:6969".parse().unwrap(),
        );

        let message = query_command.into_message(&own_node_id, &secret);

        let exp_message = Message::get_peers_query(&own_node_id, &info_hash);

        // transaction_id is going to be different
        assert_eq!(
            exp_message.to_bytes().unwrap()[..70],
            message.to_bytes().unwrap()[..70]
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

        let resp_command = ConCommand::new(
            CommandType::Resp(
                RespCommand::FindNode {
                    nodes: nodes.to_owned(),
                },
                tid.clone(),
            ),
            "127.0.0.1:6969".parse().unwrap(),
        );

        let message = resp_command.into_message(&own_node_id, &secret);

        let exp_message = Message::find_nodes_resp(&own_node_id, nodes, tid);

        assert_eq!(exp_message.to_bytes().unwrap(), message.to_bytes().unwrap());
    }
}
