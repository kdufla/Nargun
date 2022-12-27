use super::routing_table::Node;
use crate::{
    constants::{SIX, T26IX},
    util::{functions::socketaddr_from_compact_bytes, id::ID},
};
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::net::SocketAddrV4;
use std::str;

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Message {
    Query {
        #[serde(rename = "t")]
        transaction_id: String,
        #[serde(rename = "y")]
        msg_type: String,
        #[serde(rename = "q")]
        method_name: String,
        #[serde(rename = "a")]
        arguments: Arguments,
    },
    Response {
        #[serde(rename = "t")]
        transaction_id: String,
        #[serde(rename = "y")]
        msg_type: String,
        #[serde(rename = "r")]
        response: Response,
    },
    Error {
        #[serde(rename = "t")]
        transaction_id: String,
        #[serde(rename = "y")]
        msg_type: String,
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
        token: String,
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
        token: String,
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

#[derive(Debug, PartialEq, Eq)]
pub struct Peer(SocketAddrV4);

#[derive(Debug)]
pub enum Nodes {
    Exact(Node),
    Closest(Vec<Node>),
}

impl Message {
    pub fn to_bytes(self) -> Result<Vec<u8>> {
        bendy::serde::to_bytes(&self).map_err(|e| anyhow!("{}", e))
    }

    pub fn from_bytes(buf: &[u8]) -> Result<Self> {
        bendy::serde::from_bytes::<Message>(buf).map_err(|e| anyhow!("{}", e))
    }

    pub fn get_tid(&self) -> String {
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
        .clone()
    }

    pub fn ping_query(id: &ID) -> (String, Self) {
        let tid: [u8; 2] = rand::random();
        let tid = str::from_utf8(&tid).unwrap().to_owned();

        (
            tid.clone(),
            Message::Query {
                transaction_id: tid,
                msg_type: "q".to_string(),
                method_name: "ping".to_string(),
                arguments: Arguments::Ping { id: id.clone() },
            },
        )
    }

    pub fn ping_resp(id: &ID, transaction_id: String) -> Self {
        Message::Response {
            transaction_id,
            msg_type: "r".to_string(),
            response: Response::Ping { id: id.clone() },
        }
    }

    pub fn find_nodes_query(id: &ID, target: ID) -> (String, Self) {
        let tid: [u8; 2] = rand::random();
        let tid = str::from_utf8(&tid).unwrap().to_owned();

        (
            tid.clone(),
            Message::Query {
                transaction_id: tid,
                msg_type: "q".to_string(),
                method_name: "find_node".to_string(),
                arguments: Arguments::FindNode {
                    id: id.clone(),
                    target: target.clone(),
                },
            },
        )
    }

    pub fn find_nodes_resp(id: &ID, nodes: Nodes, transaction_id: String) -> Self {
        Message::Response {
            transaction_id,
            msg_type: "r".to_string(),
            response: Response::FindNode {
                id: id.clone(),
                nodes,
            },
        }
    }

    pub fn get_peers_query(id: &ID, info_hash: ID) -> (String, Self) {
        let tid: [u8; 2] = rand::random();
        let tid = str::from_utf8(&tid).unwrap().to_owned();

        (
            tid.clone(),
            Message::Query {
                transaction_id: tid,
                msg_type: "q".to_string(),
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
        token: String,
        values_or_nodes: ValuesOrNodes,
        transaction_id: String,
    ) -> Self {
        Message::Response {
            transaction_id,
            msg_type: "r".to_string(),
            response: Response::GetPeers {
                id: id.clone(),
                token,
                values_or_nodes,
            },
        }
    }

    pub fn announce_peer_query(id: &ID, info_hash: ID, port: u16, token: String) -> (String, Self) {
        let tid: [u8; 2] = rand::random();
        let tid = str::from_utf8(&tid).unwrap().to_owned();

        (
            tid.clone(),
            Message::Query {
                transaction_id: tid,
                msg_type: "q".to_string(),
                method_name: "announce_peer".to_string(),
                arguments: Arguments::AnnouncePeer {
                    id: id.clone(),
                    info_hash,
                    port,
                    token,
                },
            },
        )
    }

    pub fn announce_peer_resp(id: &ID, transaction_id: String) -> Self {
        Message::Response {
            transaction_id,
            msg_type: "r".to_string(),
            response: Response::Ping { id: id.clone() },
        }
    }
}

impl Peer {
    pub fn new(addr: SocketAddrV4) -> Self {
        Self(addr)
    }

    fn from_compact_bytes(buff: &[u8]) -> Result<Self> {
        if buff.len() == SIX {
            Ok(Peer(socketaddr_from_compact_bytes(buff)?))
        } else {
            bail!(
                "Peer::from_compact buff size is {}, expected {}",
                buff.len(),
                SIX
            )
        }
    }

    pub fn addr(&self) -> &SocketAddrV4 {
        &self.0
    }
}

impl Serialize for Peer {
    fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut v = Vec::new();

        v.extend_from_slice(&self.0.ip().octets());
        v.push((self.0.port() >> 8) as u8);
        v.push((self.0.port() & 0xff) as u8);

        s.serialize_bytes(&v)
    }
}

impl<'de> Deserialize<'de> for Peer {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Peer;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("Compact <ip=4><port=2> bytes")
            }

            fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                if v.len() == SIX {
                    Peer::from_compact_bytes(v).map_err(serde::de::Error::custom)
                } else {
                    Err(serde::de::Error::invalid_length(v.len(), &self))
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
                let data_len_is_multiple_of_node_len = v.len() % T26IX != 0;
                let number_of_nodes = v.len() / T26IX;

                if data_len_is_multiple_of_node_len {
                    return Err(serde::de::Error::invalid_length(
                        v.len(),
                        &format!("k*{}", T26IX).as_str(),
                    ));
                }

                if number_of_nodes == 1 {
                    let node = Node::from_compact_bytes(v).map_err(serde::de::Error::custom)?;
                    Ok(Nodes::Exact(node))
                } else {
                    let mut nodes = Vec::with_capacity(number_of_nodes);

                    for raw_node in v.chunks(T26IX) {
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

    use super::Peer;

    #[test]
    fn peer_from_compact() {
        let data = "yhf5aa".as_bytes();
        let result = Peer::from_compact_bytes(data);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().0.to_string(), "121.104.102.53:24929");

        let long_data = "yhf5aa++".as_bytes();
        let result = Peer::from_compact_bytes(long_data);
        assert!(result.is_err());

        let short_data = "yhf".as_bytes();
        let result = Peer::from_compact_bytes(short_data);
        assert!(result.is_err());
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
        use super::super::Message;
        use crate::{
            dht::{
                krpc_message::{Arguments, Error, Nodes, Peer, Response, ValuesOrNodes},
                routing_table::Node,
            },
            util::id::ID,
        };
        use std::net::{Ipv4Addr, SocketAddrV4};

        #[test]
        fn ping_query() {
            let data = Message::Query {
                transaction_id: "aa".to_string(),
                msg_type: "q".to_string(),
                method_name: "ping".to_string(),
                arguments: Arguments::Ping {
                    id: ID([
                        97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 48, 49, 50, 51, 52, 53, 54,
                        55, 56, 57,
                    ]),
                },
            };

            assert_eq!(
                bendy::serde::to_bytes(&data).unwrap(),
                "d1:ad2:id20:abcdefghij0123456789e1:q4:ping1:t2:aa1:y1:qe".as_bytes()
            );
        }

        #[test]
        fn ping_resp() {
            let data = Message::Response {
                transaction_id: "aa".to_string(),
                msg_type: "r".to_string(),
                response: Response::Ping {
                    id: ID([
                        109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 49,
                        50, 51, 52, 53, 54,
                    ]),
                },
            };

            assert_eq!(
                bendy::serde::to_bytes(&data).unwrap(),
                "d1:rd2:id20:mnopqrstuvwxyz123456e1:t2:aa1:y1:re".as_bytes()
            );
        }

        #[test]
        fn find_nodes_query() {
            let data = Message::Query {
                transaction_id: "aa".to_string(),
                msg_type: "q".to_string(),
                method_name: "find_node".to_string(),
                arguments: Arguments::FindNode {
                    id: ID([
                        97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 48, 49, 50, 51, 52, 53, 54,
                        55, 56, 57,
                    ]),
                    target: ID([
                        109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 49,
                        50, 51, 52, 53, 54,
                    ]),
                },
            };

            assert_eq!(
                bendy::serde::to_bytes(&data).unwrap(),
                "d1:ad2:id20:abcdefghij01234567896:target20:mnopqrstuvwxyz123456e1:q9:find_node1:t2:aa1:y1:qe".as_bytes()
            );
        }

        #[test]
        fn find_nodes_resp_closest() {
            let data = Message::Response {
                transaction_id: "aa".to_string(),
                msg_type: "r".to_string(),
                response: Response::FindNode {
                    id: ID([
                        48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 97, 98, 99, 100, 101, 102, 103,
                        104, 105, 106,
                    ]),
                    nodes: Nodes::Closest(vec![
                        Node::from_compact_bytes("rdYAxWC9Zi!A97zKJUbH9HVcgP".as_bytes()).unwrap(),
                        Node::from_compact_bytes("7Z5cQScmZcC4M2hYKy!JrcYPtT".as_bytes()).unwrap(),
                        Node::from_compact_bytes("9^jy^pm8sZQy3dukB$CF9^o@of".as_bytes()).unwrap(),
                    ]),
                },
            };

            assert_eq!(
                bendy::serde::to_bytes(&data).unwrap(),
                "d1:rd2:id20:0123456789abcdefghij5:nodes78:rdYAxWC9Zi!A97zKJUbH9HVcgP7Z5cQScmZcC4M2hYKy!JrcYPtT9^jy^pm8sZQy3dukB$CF9^o@ofe1:t2:aa1:y1:re".as_bytes()
            );
        }

        #[test]
        fn find_nodes_resp_exact() {
            let data = Message::Response {
                transaction_id: "aa".to_string(),
                msg_type: "r".to_string(),
                response: Response::FindNode {
                    id: ID([
                        48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 97, 98, 99, 100, 101, 102, 103,
                        104, 105, 106,
                    ]),
                    nodes: Nodes::Exact(
                        Node::from_compact_bytes("rdYAxWC9Zi!A97zKJUbH9HVcgP".as_bytes()).unwrap(),
                    ),
                },
            };

            assert_eq!(
                bendy::serde::to_bytes(&data).unwrap(),
                "d1:rd2:id20:0123456789abcdefghij5:nodes26:rdYAxWC9Zi!A97zKJUbH9HVcgPe1:t2:aa1:y1:re".as_bytes()
            );
        }

        #[test]
        fn get_peers_query() {
            let data = Message::Query {
                transaction_id: "aa".to_string(),
                msg_type: "q".to_string(),
                method_name: "get_peers".to_string(),
                arguments: Arguments::GetPeers {
                    id: ID([
                        97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 48, 49, 50, 51, 52, 53, 54,
                        55, 56, 57,
                    ]),
                    info_hash: ID([
                        109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 49,
                        50, 51, 52, 53, 54,
                    ]),
                },
            };

            assert_eq!(
                bendy::serde::to_bytes(&data).unwrap(),
                "d1:ad2:id20:abcdefghij01234567899:info_hash20:mnopqrstuvwxyz123456e1:q9:get_peers1:t2:aa1:y1:qe".as_bytes()
            );
        }

        #[test]
        fn get_peers_resp_vals() {
            let data = Message::Response {
                transaction_id: "aa".to_string(),
                msg_type: "r".to_string(),
                response: Response::GetPeers {
                    id: ID([
                        97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 48, 49, 50, 51, 52, 53, 54,
                        55, 56, 57,
                    ]),
                    token: "aoeusnth".to_string(),
                    values_or_nodes: ValuesOrNodes::Values {
                        values: vec![
                            Peer(SocketAddrV4::new(
                                Ipv4Addr::new(b'a', b'x', b'j', b'e'),
                                11893,
                            )),
                            Peer(SocketAddrV4::new(
                                Ipv4Addr::new(b'i', b'd', b'h', b't'),
                                28269,
                            )),
                        ],
                    },
                },
            };

            assert_eq!(
                bendy::serde::to_bytes(&data).unwrap(),
                "d1:rd2:id20:abcdefghij01234567895:token8:aoeusnth6:valuesl6:axje.u6:idhtnmee1:t2:aa1:y1:re".as_bytes()
            );
        }

        #[test]
        fn get_peers_resp_nodes() {
            let data = Message::Response {
                transaction_id: "aa".to_string(),
                msg_type: "r".to_string(),
                response: Response::GetPeers {
                    id: ID([
                        97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 48, 49, 50, 51, 52, 53, 54,
                        55, 56, 57,
                    ]),
                    token: "aoeusnth".to_string(),
                    values_or_nodes: ValuesOrNodes::Nodes {
                        nodes: Nodes::Closest(vec![
                            Node::from_compact_bytes("rdYAxWC9Zi!A97zKJUbH9HVcgP".as_bytes())
                                .unwrap(),
                            Node::from_compact_bytes("7Z5cQScmZcC4M2hYKy!JrcYPtT".as_bytes())
                                .unwrap(),
                            Node::from_compact_bytes("9^jy^pm8sZQy3dukB$CF9^o@of".as_bytes())
                                .unwrap(),
                        ]),
                    },
                },
            };

            assert_eq!(
                bendy::serde::to_bytes(&data).unwrap(),
                "d1:rd2:id20:abcdefghij01234567895:nodes78:rdYAxWC9Zi!A97zKJUbH9HVcgP7Z5cQScmZcC4M2hYKy!JrcYPtT9^jy^pm8sZQy3dukB$CF9^o@of5:token8:aoeusnthe1:t2:aa1:y1:re".as_bytes()
            );
        }

        #[test]
        fn announce_peer_query() {
            let data = Message::Query {
                transaction_id: "aa".to_string(),
                msg_type: "q".to_string(),
                method_name: "announce_peer".to_string(),
                arguments: Arguments::AnnouncePeer {
                    id: ID([
                        97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 48, 49, 50, 51, 52, 53, 54,
                        55, 56, 57,
                    ]),
                    info_hash: ID([
                        109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 49,
                        50, 51, 52, 53, 54,
                    ]),
                    port: 6881,
                    token: "aoeusnth".to_string(),
                },
            };

            assert_eq!(
                bendy::serde::to_bytes(&data).unwrap(),
                "d1:ad2:id20:abcdefghij01234567899:info_hash20:mnopqrstuvwxyz1234564:porti6881e5:token8:aoeusnthe1:q13:announce_peer1:t2:aa1:y1:qe".as_bytes()
            );
        }

        #[test]
        fn announce_peer_resp() {
            let data = Message::Response {
                transaction_id: "aa".to_string(),
                msg_type: "r".to_string(),
                response: Response::AnnouncePeer {
                    id: ID([
                        109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 49,
                        50, 51, 52, 53, 54,
                    ]),
                },
            };

            assert_eq!(
                bendy::serde::to_bytes(&data).unwrap(),
                "d1:rd2:id20:mnopqrstuvwxyz123456e1:t2:aa1:y1:re".as_bytes()
            );
        }

        #[test]
        fn error() {
            let data = Message::Error {
                transaction_id: "aa".to_string(),
                msg_type: "e".to_string(),
                error: vec![
                    Error::Code(201),
                    Error::Desc("A Generic Error Ocurred".to_string()),
                ],
            };

            assert_eq!(
                bendy::serde::to_bytes(&data).unwrap(),
                "d1:eli201e23:A Generic Error Ocurrede1:t2:aa1:y1:ee".as_bytes()
            );
        }
    }
    mod decode {
        use super::super::Message;
        use crate::{
            dht::krpc_message::{Arguments, Error, Nodes, Peer, Response, ValuesOrNodes},
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
                assert_eq!(t, "aa");
                assert_eq!(y, "q".to_string(),);
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
                assert_eq!(t, "aa");
                assert_eq!(y, "r");

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
                assert_eq!(t, "aa");
                assert_eq!(y, "q".to_string(),);
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
                assert_eq!(t, "aa");
                assert_eq!(y, "r");

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
                assert_eq!(t, "aa");
                assert_eq!(y, "r");

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
                assert_eq!(t, "aa");
                assert_eq!(y, "q".to_string(),);
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
                assert_eq!(t, "aa");
                assert_eq!(y, "r");

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

                    assert_eq!(token, "aoeusnth");

                    assert!(matches!(values_or_nodes, ValuesOrNodes::Values { .. }));
                    if let ValuesOrNodes::Values { values } = values_or_nodes {
                        assert_eq!(values.len(), 2);

                        assert_eq!(
                            values[0],
                            Peer(SocketAddrV4::new(
                                Ipv4Addr::new(b'a', b'x', b'j', b'e'),
                                11893
                            ))
                        );
                        assert_eq!(
                            values[1],
                            Peer(SocketAddrV4::new(
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
                assert_eq!(t, "aa");
                assert_eq!(y, "r");

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

                    assert_eq!(token, "aoeusnth");

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
                assert_eq!(t, "aa");
                assert_eq!(y, "q".to_string(),);
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

                    assert_eq!(token, "aoeusnth");

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
                assert_eq!(t, "aa");
                assert_eq!(y, "e");

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
