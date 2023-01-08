use super::routing_table::Node;
use crate::{
    constants::COMPACT_NODE_LEN, peer::Peer, peer_message::SerializableBytes, util::id::ID,
};
use anyhow::{anyhow, Result};
use bytes::Bytes;
use serde::{de::Unexpected, Deserialize, Deserializer, Serialize, Serializer};
use std::str;

// TODO fuck bendy! it's the worst decision I've made. I need to impl my own parser for serde.
#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Message {
    Query {
        #[serde(rename = "t")]
        transaction_id: SerializableBytes,
        #[serde(rename = "y")]
        msg_type: MessageType,
        #[serde(rename = "q")]
        method_name: String,
        #[serde(rename = "a")]
        arguments: Arguments,
    },
    Response {
        #[serde(rename = "t")]
        transaction_id: SerializableBytes,
        #[serde(rename = "y")]
        msg_type: MessageType,
        #[serde(rename = "r")]
        response: Response,
    },
    Error {
        #[serde(rename = "t")]
        transaction_id: SerializableBytes,
        #[serde(rename = "y")]
        msg_type: MessageType,
        #[serde(rename = "e")]
        error: Vec<Error>,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Arguments {
    AnnouncePeer {
        id: ID,
        info_hash: ID,
        port: u16,
        token: SerializableBytes,
    },
    FindNode {
        id: ID,
        target: ID,
    },
    GetPeers {
        id: ID,
        info_hash: ID,
    },
    Ping {
        id: ID,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Response {
    GetPeers {
        id: ID,
        token: SerializableBytes,
        #[serde(flatten)]
        values_or_nodes: ValuesOrNodes,
    },
    FindNode {
        id: ID,
        nodes: Nodes,
    },
    Ping {
        id: ID,
    },
    AnnouncePeer {
        id: ID,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Error {
    Code(u32),
    Desc(String),
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ValuesOrNodes {
    Values { values: Vec<Peer> },
    Nodes { nodes: Nodes },
}

// #[derive(Debug, PartialEq, Eq)]
// pub struct Peer(SocketAddrV4);

// impl From<Peer> for SocketAddrV4 {
//     fn from(item: Peer) -> Self {
//         item.0
//     }
// }

#[derive(Debug)]
pub enum Nodes {
    Exact(Node),
    Closest(Vec<Node>),
}

pub type TID = [u8; 5];

impl Message {
    pub fn to_bytes(self) -> Result<Vec<u8>> {
        bendy::serde::to_bytes(&self).map_err(|e| anyhow!("{}", e))
    }

    pub fn from_bytes(buf: &[u8]) -> Result<Self> {
        bendy::serde::from_bytes::<Message>(buf).map_err(|e| anyhow!("{}", e))
    }

    pub fn get_tid(&self) -> Bytes {
        match self {
            Message::Query {
                transaction_id,
                msg_type: _,
                method_name: _,
                arguments: _,
            } => transaction_id,
            Message::Response {
                transaction_id,
                msg_type: _,
                response: _,
            } => transaction_id,
            Message::Error {
                transaction_id,
                msg_type: _,
                error: _,
            } => transaction_id,
        }
        .as_bytes()
    }

    pub fn ping_query(id: &ID) -> (Bytes, Self) {
        let transaction_id = Bytes::from(Vec::from(rand::random::<TID>()));

        (
            transaction_id.clone(),
            Message::Query {
                transaction_id: SerializableBytes::new(transaction_id),
                msg_type: MessageType(b'q'),
                method_name: "ping".to_string(),
                arguments: Arguments::Ping { id: id.clone() },
            },
        )
    }

    pub fn ping_resp(id: &ID, transaction_id: Bytes) -> Self {
        Message::Response {
            transaction_id: SerializableBytes::new(transaction_id),
            msg_type: MessageType(b'r'),
            response: Response::Ping { id: id.clone() },
        }
    }

    pub fn find_nodes_query(id: &ID, target: ID) -> (Bytes, Self) {
        let transaction_id = Bytes::from(Vec::from(rand::random::<TID>()));

        (
            transaction_id.clone(),
            Message::Query {
                transaction_id: SerializableBytes::new(transaction_id),
                msg_type: MessageType(b'q'),
                method_name: "find_node".to_string(),
                arguments: Arguments::FindNode {
                    id: id.clone(),
                    target: target.clone(),
                },
            },
        )
    }

    pub fn find_nodes_resp(id: &ID, nodes: Nodes, transaction_id: Bytes) -> Self {
        Message::Response {
            transaction_id: SerializableBytes::new(transaction_id),
            msg_type: MessageType(b'r'),
            response: Response::FindNode {
                id: id.clone(),
                nodes,
            },
        }
    }

    pub fn get_peers_query(id: &ID, info_hash: ID) -> (Bytes, Self) {
        let transaction_id = Bytes::from(Vec::from(rand::random::<TID>()));

        (
            transaction_id.clone(),
            Message::Query {
                transaction_id: SerializableBytes::new(transaction_id),
                msg_type: MessageType(b'q'),
                method_name: "get_peers".to_string(),
                arguments: Arguments::GetPeers {
                    id: id.clone(),
                    info_hash: info_hash.clone(),
                },
            },
        )
    }

    pub fn get_peers_resp(
        id: &ID,
        token: Bytes,
        values_or_nodes: ValuesOrNodes,
        transaction_id: Bytes,
    ) -> Self {
        Message::Response {
            transaction_id: SerializableBytes::new(transaction_id),
            msg_type: MessageType(b'r'),
            response: Response::GetPeers {
                id: id.clone(),
                token: SerializableBytes::new(token),
                values_or_nodes,
            },
        }
    }

    pub fn announce_peer_query(id: &ID, info_hash: ID, port: u16, token: Bytes) -> (Bytes, Self) {
        let transaction_id = Bytes::from(Vec::from(rand::random::<TID>()));

        (
            transaction_id.clone(),
            Message::Query {
                transaction_id: SerializableBytes::new(transaction_id),
                msg_type: MessageType(b'q'),
                method_name: "announce_peer".to_string(),
                arguments: Arguments::AnnouncePeer {
                    id: id.clone(),
                    info_hash,
                    port,
                    token: SerializableBytes::new(token),
                },
            },
        )
    }

    pub fn announce_peer_resp(id: &ID, transaction_id: Bytes) -> Self {
        Message::Response {
            transaction_id: SerializableBytes::new(transaction_id),
            msg_type: MessageType(b'r'),
            response: Response::Ping { id: id.clone() },
        }
    }

    pub fn error_resp(error: Vec<Error>, transaction_id: Bytes) -> Self {
        Message::Error {
            transaction_id: SerializableBytes::new(transaction_id),
            msg_type: MessageType(b'e'),
            error,
        }
    }
}

#[derive(Debug)]
pub struct MessageType(u8);

impl Serialize for MessageType {
    fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self.0 {
            b'q' => s.serialize_str("q"),
            b'r' => s.serialize_str("r"),
            _ => s.serialize_str("e"),
        }
    }
}

impl<'de> Deserialize<'de> for MessageType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = MessageType;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("q/r/e")
            }

            fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                if v.len() != 1 {
                    return Err(serde::de::Error::invalid_length(v.len(), &self));
                }

                match v[0] {
                    b'q' | b'r' | b'e' => Ok(MessageType(v[0])),
                    _ => Err(serde::de::Error::invalid_value(
                        Unexpected::Char(v[0] as char),
                        &self,
                    )),
                }
            }
        }

        Ok(deserializer.deserialize_byte_buf(Visitor {})?)
    }
}

impl Nodes {
    pub fn iter(&self) -> impl Iterator<Item = &Node> {
        NodesIter { data: self, idx: 0 }
    }
}

struct NodesIter<'a> {
    data: &'a Nodes,
    idx: usize,
}

impl<'a> Iterator for NodesIter<'a> {
    type Item = &'a Node;

    fn next(&mut self) -> Option<Self::Item> {
        match self.data {
            Nodes::Exact(node) if self.idx == 0 => {
                self.idx += 1;
                Some(node)
            }
            Nodes::Closest(nodes) if nodes.len() > self.idx => {
                let rv = &nodes[self.idx];
                self.idx += 1;
                Some(rv)
            }
            _ => None,
        }
    }
}

macro_rules! push_node_bytes_to_vec {
    ($vec:ident, $node:ident) => {{
        $vec.extend_from_slice($node.id.as_bytes());
        $vec.extend_from_slice(&$node.addr.ip().octets());
        $vec.push(($node.addr.port() >> 8) as u8);
        $vec.push(($node.addr.port() & 0xff) as u8);
    }};
}

impl Serialize for Nodes {
    fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut v = Vec::new();

        match self {
            Self::Exact(node) => {
                push_node_bytes_to_vec!(v, node);
            }
            Self::Closest(nodes) => {
                for node in nodes {
                    push_node_bytes_to_vec!(v, node);
                }
            }
        }

        s.serialize_bytes(&v)
    }
}

impl<'de> Deserialize<'de> for Nodes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Nodes;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("Compact <id=20><ip=4><port=2> bytes")
            }

            fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                let data_len_is_multiple_of_node_len = v.len() % COMPACT_NODE_LEN != 0;
                let number_of_nodes = v.len() / COMPACT_NODE_LEN;

                if data_len_is_multiple_of_node_len {
                    return Err(serde::de::Error::invalid_length(
                        v.len(),
                        &format!("k*{}", COMPACT_NODE_LEN).as_str(),
                    ));
                }

                if number_of_nodes == 1 {
                    let node = Node::from_compact_bytes(v).map_err(serde::de::Error::custom)?;
                    Ok(Nodes::Exact(node))
                } else {
                    let mut nodes = Vec::with_capacity(number_of_nodes);

                    for raw_node in v.chunks(COMPACT_NODE_LEN) {
                        nodes.push(
                            Node::from_compact_bytes(raw_node).map_err(serde::de::Error::custom)?,
                        )
                    }

                    Ok(Nodes::Closest(nodes))
                }
            }
        }

        Ok(deserializer.deserialize_byte_buf(Visitor {})?)
    }
}

#[cfg(test)]
mod krpc_tests {
    use crate::{dht::routing_table::Node, util::id::ID};

    macro_rules! encoded_with_custom_tid {
        ($start:expr, $tid:expr) => {{
            let mut encoded = $start.as_bytes().to_vec();

            encoded.push(0x30 + $tid.len() as u8);
            encoded.push(b':');
            for b in $tid {
                encoded.push(b);
            }

            encoded.extend_from_slice("1:y1:qe".as_bytes());
            encoded
        }};
    }

    #[test]
    fn node_from_compact() {
        let data = "qwertyuiopasdfghjklzyhf5aa".as_bytes();
        let result = Node::from_compact_bytes(data);
        assert!(result.is_ok());
        let result = result.unwrap();
        assert_eq!(result.addr.to_string(), "121.104.102.53:24929");
        assert_eq!(
            result.id,
            ID([
                b'q', b'w', b'e', b'r', b't', b'y', b'u', b'i', b'o', b'p', b'a', b's', b'd', b'f',
                b'g', b'h', b'j', b'k', b'l', b'z'
            ])
        );

        let long_data = "qwertyuiopasdfghjklzyhf5aayhf5aa++".as_bytes();
        let result = Node::from_compact_bytes(long_data);
        assert!(result.is_err());

        let short_data = "yhf".as_bytes();
        let result = Node::from_compact_bytes(short_data);
        assert!(result.is_err());
    }

    mod encode {
        use bytes::Bytes;

        use super::super::Message;
        use crate::{
            dht::{
                krpc_message::{Error, Nodes, ValuesOrNodes},
                routing_table::Node,
            },
            peer::Peer,
            util::id::ID,
        };
        use std::net::{Ipv4Addr, SocketAddrV4};

        #[test]
        fn ping_query() {
            let (tid, message) = Message::ping_query(&ID([
                97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 48, 49, 50, 51, 52, 53, 54, 55, 56,
                57,
            ]));

            assert_eq!(
                bendy::serde::to_bytes(&message).unwrap(),
                encoded_with_custom_tid!("d1:ad2:id20:abcdefghij0123456789e1:q4:ping1:t", tid)
            );
        }

        #[test]
        fn ping_resp() {
            let message = Message::ping_resp(
                &ID([
                    109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 49, 50,
                    51, 52, 53, 54,
                ]),
                Bytes::from_static(b"aa"),
            );

            assert_eq!(
                bendy::serde::to_bytes(&message).unwrap(),
                "d1:rd2:id20:mnopqrstuvwxyz123456e1:t2:aa1:y1:re".as_bytes()
            );
        }

        #[test]
        fn find_nodes_query() {
            let (tid, message) = Message::find_nodes_query(
                &ID([
                    97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 48, 49, 50, 51, 52, 53, 54, 55,
                    56, 57,
                ]),
                ID([
                    109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 49, 50,
                    51, 52, 53, 54,
                ]),
            );

            assert_eq!(
                bendy::serde::to_bytes(&message).unwrap(),
                encoded_with_custom_tid!(
                    "d1:ad2:id20:abcdefghij01234567896:target20:mnopqrstuvwxyz123456e1:q9:find_node1:t",
                    tid
                )
            );
        }

        #[test]
        fn find_nodes_resp_closest() {
            let message = Message::find_nodes_resp(
                &ID([
                    48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 97, 98, 99, 100, 101, 102, 103, 104,
                    105, 106,
                ]),
                Nodes::Closest(vec![
                    Node::from_compact_bytes("rdYAxWC9Zi!A97zKJUbH9HVcgP".as_bytes()).unwrap(),
                    Node::from_compact_bytes("7Z5cQScmZcC4M2hYKy!JrcYPtT".as_bytes()).unwrap(),
                    Node::from_compact_bytes("9^jy^pm8sZQy3dukB$CF9^o@of".as_bytes()).unwrap(),
                ]),
                Bytes::from_static(b"aa"),
            );

            assert_eq!(
                bendy::serde::to_bytes(&message).unwrap(),
                "d1:rd2:id20:0123456789abcdefghij5:nodes78:rdYAxWC9Zi!A97zKJUbH9HVcgP7Z5cQScmZcC4M2hYKy!JrcYPtT9^jy^pm8sZQy3dukB$CF9^o@ofe1:t2:aa1:y1:re".as_bytes()
            );
        }

        #[test]
        fn find_nodes_resp_exact() {
            let message = Message::find_nodes_resp(
                &ID([
                    48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 97, 98, 99, 100, 101, 102, 103, 104,
                    105, 106,
                ]),
                Nodes::Exact(
                    Node::from_compact_bytes("rdYAxWC9Zi!A97zKJUbH9HVcgP".as_bytes()).unwrap(),
                ),
                Bytes::from_static(b"aa"),
            );

            assert_eq!(
                bendy::serde::to_bytes(&message).unwrap(),
                "d1:rd2:id20:0123456789abcdefghij5:nodes26:rdYAxWC9Zi!A97zKJUbH9HVcgPe1:t2:aa1:y1:re".as_bytes()
            );
        }

        #[test]
        fn get_peers_query() {
            let (tid, message) = Message::get_peers_query(
                &ID([
                    97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 48, 49, 50, 51, 52, 53, 54, 55,
                    56, 57,
                ]),
                ID([
                    109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 49, 50,
                    51, 52, 53, 54,
                ]),
            );

            assert_eq!(
                bendy::serde::to_bytes(&message).unwrap(),
                encoded_with_custom_tid!(
                    "d1:ad2:id20:abcdefghij01234567899:info_hash20:mnopqrstuvwxyz123456e1:q9:get_peers1:t",
                    tid
                )
            );
        }

        #[test]
        fn get_peers_resp_vals() {
            let message = Message::get_peers_resp(
                &ID([
                    97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 48, 49, 50, 51, 52, 53, 54, 55,
                    56, 57,
                ]),
                Bytes::from_static(b"aoeusnth"),
                ValuesOrNodes::Values {
                    values: vec![
                        Peer::new(SocketAddrV4::new(
                            Ipv4Addr::new(b'a', b'x', b'j', b'e'),
                            11893,
                        )),
                        Peer::new(SocketAddrV4::new(
                            Ipv4Addr::new(b'i', b'd', b'h', b't'),
                            28269,
                        )),
                    ],
                },
                Bytes::from_static(b"aa"),
            );

            assert_eq!(
                bendy::serde::to_bytes(&message).unwrap(),
                "d1:rd2:id20:abcdefghij01234567895:token8:aoeusnth6:valuesl6:axje.u6:idhtnmee1:t2:aa1:y1:re".as_bytes()
            );
        }

        #[test]
        fn get_peers_resp_nodes() {
            let message = Message::get_peers_resp(
                &ID([
                    97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 48, 49, 50, 51, 52, 53, 54, 55,
                    56, 57,
                ]),
                Bytes::from_static(b"aoeusnth"),
                ValuesOrNodes::Nodes {
                    nodes: Nodes::Closest(vec![
                        Node::from_compact_bytes("rdYAxWC9Zi!A97zKJUbH9HVcgP".as_bytes()).unwrap(),
                        Node::from_compact_bytes("7Z5cQScmZcC4M2hYKy!JrcYPtT".as_bytes()).unwrap(),
                        Node::from_compact_bytes("9^jy^pm8sZQy3dukB$CF9^o@of".as_bytes()).unwrap(),
                    ]),
                },
                Bytes::from_static(b"aa"),
            );

            assert_eq!(
                bendy::serde::to_bytes(&message).unwrap(),
                "d1:rd2:id20:abcdefghij01234567895:nodes78:rdYAxWC9Zi!A97zKJUbH9HVcgP7Z5cQScmZcC4M2hYKy!JrcYPtT9^jy^pm8sZQy3dukB$CF9^o@of5:token8:aoeusnthe1:t2:aa1:y1:re".as_bytes()
            );
        }

        #[test]
        fn announce_peer_query() {
            let (tid, message) = Message::announce_peer_query(
                &ID([
                    97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 48, 49, 50, 51, 52, 53, 54, 55,
                    56, 57,
                ]),
                ID([
                    109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 49, 50,
                    51, 52, 53, 54,
                ]),
                6881,
                Bytes::from_static(b"aoeusnth"),
            );

            assert_eq!(
                bendy::serde::to_bytes(&message).unwrap(),
                encoded_with_custom_tid!(
                    "d1:ad2:id20:abcdefghij01234567899:info_hash20:mnopqrstuvwxyz1234564:porti6881e5:token8:aoeusnthe1:q13:announce_peer1:t",
                    tid
                )
            );
        }

        #[test]
        fn announce_peer_resp() {
            let message = Message::announce_peer_resp(
                &ID([
                    109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 49, 50,
                    51, 52, 53, 54,
                ]),
                Bytes::from_static(b"aa"),
            );

            assert_eq!(
                bendy::serde::to_bytes(&message).unwrap(),
                "d1:rd2:id20:mnopqrstuvwxyz123456e1:t2:aa1:y1:re".as_bytes()
            );
        }

        #[test]
        fn error() {
            let message = Message::error_resp(
                vec![
                    Error::Code(201),
                    Error::Desc("A Generic Error Ocurred".to_string()),
                ],
                Bytes::from_static(b"aa"),
            );

            assert_eq!(
                bendy::serde::to_bytes(&message).unwrap(),
                "d1:eli201e23:A Generic Error Ocurrede1:t2:aa1:y1:ee".as_bytes()
            );
        }
    }
    mod decode {

        use super::super::Message;
        use crate::{
            dht::krpc_message::{Arguments, Error, Nodes, Response, ValuesOrNodes},
            peer::Peer,
            util::id::ID,
        };
        use std::net::{Ipv4Addr, SocketAddrV4};

        #[test]
        fn ping_query() {
            let raw_data = "d1:ad2:id20:abcdefghij0123456789e1:q4:ping1:t2:aa1:y1:qe".as_bytes();
            let data = bendy::serde::from_bytes::<Message>(raw_data).unwrap();

            assert!(matches!(data, Message::Query { .. }));
            if let Message::Query {
                transaction_id: t,
                msg_type: y,
                method_name: q,
                arguments: a,
            } = data
            {
                assert_eq!(t.into_bytes(), "aa".as_bytes());
                assert_eq!(y.0, b'q',);
                assert_eq!(q, "ping");

                assert!(matches!(a, Arguments::Ping { .. }));
                if let Arguments::Ping { id } = a {
                    assert_eq!(
                        ID([
                            97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 48, 49, 50, 51, 52, 53,
                            54, 55, 56, 57
                        ]),
                        id
                    );
                } else {
                    assert!(false);
                }
            } else {
                assert!(false);
            }
        }

        #[test]
        fn ping_resp() {
            let raw_data = "d1:rd2:id20:mnopqrstuvwxyz123456e1:t2:aa1:y1:re".as_bytes();
            let data = bendy::serde::from_bytes::<Message>(raw_data).unwrap();

            assert!(matches!(data, Message::Response { .. }));
            if let Message::Response {
                transaction_id: t,
                msg_type: y,
                response: r,
            } = data
            {
                assert_eq!(t.into_bytes(), "aa".as_bytes());
                assert_eq!(y.0, b'r');

                assert!(matches!(r, Response::Ping { .. }));
                if let Response::Ping { id } = r {
                    assert_eq!(
                        ID([
                            109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122,
                            49, 50, 51, 52, 53, 54
                        ]),
                        id
                    );
                } else {
                    assert!(false);
                }
            } else {
                assert!(false);
            }
        }

        #[test]
        fn find_nodes_query() {
            let raw_data =
                "d1:ad2:id20:abcdefghij01234567896:target20:mnopqrstuvwxyz123456e1:q9:find_node1:t2:aa1:y1:qe".as_bytes();
            let data = bendy::serde::from_bytes::<Message>(raw_data).unwrap();

            assert!(matches!(data, Message::Query { .. }));
            if let Message::Query {
                transaction_id: t,
                msg_type: y,
                method_name: q,
                arguments: a,
            } = data
            {
                assert_eq!(t.into_bytes(), "aa".as_bytes());
                assert_eq!(y.0, b'q',);
                assert_eq!(q, "find_node");

                assert!(matches!(a, Arguments::FindNode { .. }));
                if let Arguments::FindNode { id, target } = a {
                    assert_eq!(
                        ID([
                            97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 48, 49, 50, 51, 52, 53,
                            54, 55, 56, 57
                        ]),
                        id
                    );

                    assert_eq!(
                        ID([
                            109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122,
                            49, 50, 51, 52, 53, 54
                        ]),
                        target
                    );
                } else {
                    assert!(false);
                }
            } else {
                assert!(false);
            }
        }

        #[test]
        fn find_nodes_resp_closest() {
            let raw_data =
                "d1:rd2:id20:0123456789abcdefghij5:nodes78:rdYAxWC9Zi!A97zKJUbH9HVcgP7Z5cQScmZcC4M2hYKy!JrcYPtT9^jy^pm8sZQy3dukB$CF9^o@ofe1:t2:aa1:y1:re".as_bytes();
            let data = bendy::serde::from_bytes::<Message>(raw_data).unwrap();

            assert!(matches!(data, Message::Response { .. }));
            if let Message::Response {
                transaction_id: t,
                msg_type: y,
                response: r,
            } = data
            {
                assert_eq!(t.into_bytes(), "aa".as_bytes());
                assert_eq!(y.0, b'r');

                assert!(matches!(r, Response::FindNode { .. }));
                if let Response::FindNode { id, nodes } = r {
                    assert_eq!(
                        ID([
                            48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 97, 98, 99, 100, 101, 102, 103,
                            104, 105, 106
                        ]),
                        id
                    );

                    assert!(matches!(nodes, Nodes::Closest { .. }));
                    if let Nodes::Closest(node_list) = nodes {
                        assert_eq!(node_list.len(), 3);
                        assert_eq!(
                            node_list[1].id,
                            ID([
                                b'7', b'Z', b'5', b'c', b'Q', b'S', b'c', b'm', b'Z', b'c', b'C',
                                b'4', b'M', b'2', b'h', b'Y', b'K', b'y', b'!', b'J',
                            ])
                        );
                        assert_eq!(Ok(node_list[2].addr), "57.94.111.64:28518".parse())
                    } else {
                        assert!(false);
                    }
                } else {
                    assert!(false);
                }
            } else {
                assert!(false);
            }
        }

        #[test]
        fn find_nodes_resp_exact() {
            let raw_data = "d1:rd2:id20:0123456789abcdefghij5:nodes26:rdYAxWC9Zi!A97zKJUbH9HVcgPe1:t2:aa1:y1:re".as_bytes();
            let data = bendy::serde::from_bytes::<Message>(raw_data).unwrap();

            assert!(matches!(data, Message::Response { .. }));
            if let Message::Response {
                transaction_id: t,
                msg_type: y,
                response: r,
            } = data
            {
                assert_eq!(t.into_bytes(), "aa".as_bytes());
                assert_eq!(y.0, b'r');

                assert!(matches!(r, Response::FindNode { .. }));
                if let Response::FindNode { id, nodes } = r {
                    assert_eq!(
                        ID([
                            48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 97, 98, 99, 100, 101, 102, 103,
                            104, 105, 106
                        ]),
                        id
                    );

                    assert!(matches!(nodes, Nodes::Exact { .. }));
                    if let Nodes::Exact(node) = nodes {
                        assert_eq!(
                            node.id,
                            ID([
                                b'r', b'd', b'Y', b'A', b'x', b'W', b'C', b'9', b'Z', b'i', b'!',
                                b'A', b'9', b'7', b'z', b'K', b'J', b'U', b'b', b'H'
                            ])
                        );
                        assert_eq!(node.addr, "57.72.86.99:26448".parse().unwrap())
                    } else {
                        assert!(false);
                    }
                } else {
                    assert!(false);
                }
            } else {
                assert!(false);
            }
        }

        #[test]
        fn get_peers_query() {
            let raw_data =
                "d1:ad2:id20:abcdefghij01234567899:info_hash20:mnopqrstuvwxyz123456e1:q9:get_peers1:t2:aa1:y1:qe".as_bytes();
            let data = bendy::serde::from_bytes::<Message>(raw_data).unwrap();

            assert!(matches!(data, Message::Query { .. }));
            if let Message::Query {
                transaction_id: t,
                msg_type: y,
                method_name: q,
                arguments: a,
            } = data
            {
                assert_eq!(t.into_bytes(), "aa".as_bytes());
                assert_eq!(y.0, b'q',);
                assert_eq!(q, "get_peers");

                assert!(matches!(a, Arguments::GetPeers { .. }));
                if let Arguments::GetPeers { id, info_hash } = a {
                    assert_eq!(
                        ID([
                            97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 48, 49, 50, 51, 52, 53,
                            54, 55, 56, 57
                        ]),
                        id
                    );

                    assert_eq!(
                        ID([
                            109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122,
                            49, 50, 51, 52, 53, 54
                        ]),
                        info_hash
                    );
                } else {
                    assert!(false);
                }
            } else {
                assert!(false);
            }
        }

        #[test]
        fn get_peers_resp_vals() {
            let raw_data =
                "d1:rd2:id20:abcdefghij01234567895:token8:aoeusnth6:valuesl6:axje.u6:idhtnmee1:t2:aa1:y1:re".as_bytes();
            let data = bendy::serde::from_bytes::<Message>(raw_data).unwrap();

            assert!(matches!(data, Message::Response { .. }));
            if let Message::Response {
                transaction_id: t,
                msg_type: y,
                response: r,
            } = data
            {
                assert_eq!(t.into_bytes(), "aa".as_bytes());
                assert_eq!(y.0, b'r');

                assert!(matches!(r, Response::GetPeers { .. }));
                if let Response::GetPeers {
                    id,
                    token,
                    values_or_nodes,
                } = r
                {
                    assert_eq!(
                        ID([
                            97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 48, 49, 50, 51, 52, 53,
                            54, 55, 56, 57
                        ]),
                        id
                    );

                    assert_eq!(token.into_bytes(), "aoeusnth".as_bytes());

                    assert!(matches!(values_or_nodes, ValuesOrNodes::Values { .. }));
                    if let ValuesOrNodes::Values { values } = values_or_nodes {
                        assert_eq!(values.len(), 2);

                        assert_eq!(
                            values[0],
                            Peer::new(SocketAddrV4::new(
                                Ipv4Addr::new(b'a', b'x', b'j', b'e'),
                                11893
                            ))
                        );
                        assert_eq!(
                            values[1],
                            Peer::new(SocketAddrV4::new(
                                Ipv4Addr::new(b'i', b'd', b'h', b't'),
                                28269
                            ))
                        );
                    }
                } else {
                    assert!(false);
                }
            } else {
                assert!(false);
            }
        }

        #[test]
        fn get_peers_resp_nodes() {
            let raw_data ="d1:rd2:id20:abcdefghij01234567895:nodes78:rdYAxWC9Zi!A97zKJUbH9HVcgP7Z5cQScmZcC4M2hYKy!JrcYPtT9^jy^pm8sZQy3dukB$CF9^o@of5:token8:aoeusnthe1:t2:aa1:y1:re"
            .as_bytes();
            let data = bendy::serde::from_bytes::<Message>(raw_data).unwrap();

            assert!(matches!(data, Message::Response { .. }));
            if let Message::Response {
                transaction_id: t,
                msg_type: y,
                response: r,
            } = data
            {
                assert_eq!(t.into_bytes(), "aa".as_bytes());
                assert_eq!(y.0, b'r');

                assert!(matches!(r, Response::GetPeers { .. }));
                if let Response::GetPeers {
                    id,
                    token,
                    values_or_nodes,
                } = r
                {
                    assert_eq!(
                        ID([
                            97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 48, 49, 50, 51, 52, 53,
                            54, 55, 56, 57
                        ]),
                        id
                    );

                    assert_eq!(token.into_bytes(), "aoeusnth".as_bytes());

                    assert!(matches!(values_or_nodes, ValuesOrNodes::Nodes { .. }));
                    if let ValuesOrNodes::Nodes { nodes } = values_or_nodes {
                        assert!(matches!(nodes, Nodes::Closest { .. }));
                        if let Nodes::Closest(node_list) = nodes {
                            assert_eq!(node_list.len(), 3);
                            assert_eq!(
                                node_list[1].id,
                                ID([
                                    b'7', b'Z', b'5', b'c', b'Q', b'S', b'c', b'm', b'Z', b'c',
                                    b'C', b'4', b'M', b'2', b'h', b'Y', b'K', b'y', b'!', b'J',
                                ])
                            );
                            assert_eq!(Ok(node_list[2].addr), "57.94.111.64:28518".parse())
                        } else {
                            assert!(false);
                        }
                    } else {
                        assert!(false);
                    }
                } else {
                    assert!(false);
                }
            } else {
                assert!(false);
            }
        }

        #[test]
        fn announce_peer_query() {
            let raw_data = "d1:ad2:id20:abcdefghij012345678912:implied_porti1e9:info_hash20:mnopqrstuvwxyz1234564:porti6881e5:token8:aoeusnthe1:q13:announce_peer1:t2:aa1:y1:qe".as_bytes();
            let data = bendy::serde::from_bytes::<Message>(raw_data).unwrap();

            assert!(matches!(data, Message::Query { .. }));
            if let Message::Query {
                transaction_id: t,
                msg_type: y,
                method_name: q,
                arguments: a,
            } = data
            {
                assert_eq!(t.into_bytes(), "aa".as_bytes());
                assert_eq!(y.0, b'q',);
                assert_eq!(q, "announce_peer");

                assert!(matches!(a, Arguments::AnnouncePeer { .. }));
                if let Arguments::AnnouncePeer {
                    id,
                    info_hash,
                    port,
                    token,
                } = a
                {
                    assert_eq!(
                        ID([
                            97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 48, 49, 50, 51, 52, 53,
                            54, 55, 56, 57
                        ]),
                        id
                    );

                    assert_eq!(
                        ID([
                            109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122,
                            49, 50, 51, 52, 53, 54
                        ]),
                        info_hash
                    );

                    assert_eq!(token.into_bytes(), "aoeusnth".as_bytes());

                    assert_eq!(port, 6881);
                } else {
                    assert!(false);
                }
            } else {
                assert!(false);
            }
        }

        #[test]
        fn announce_peer_resp() {
            // I don't see a way to differentiate this and ping response
            // I doubt it matters tho, let's TODO but probably not
        }

        #[test]
        fn error() {
            let raw_data = "d1:eli201e23:A Generic Error Ocurrede1:t2:aa1:y1:ee".as_bytes();
            let data = bendy::serde::from_bytes::<Message>(raw_data).unwrap();

            assert!(matches!(data, Message::Error { .. }));
            if let Message::Error {
                transaction_id: t,
                msg_type: y,
                error: e,
            } = data
            {
                assert_eq!(t.into_bytes(), "aa".as_bytes());
                assert_eq!(y.0, b'e');

                assert!(matches!(e[0], Error::Code { .. }));
                if let Error::Code(code) = e[0] {
                    assert_eq!(code, 201);
                }

                assert!(matches!(&e[1], Error::Desc { .. }));
                if let Error::Desc(desc) = &e[1] {
                    assert_eq!(desc, "A Generic Error Ocurred");
                }
            } else {
                assert!(false);
            }
        }
    }
}
