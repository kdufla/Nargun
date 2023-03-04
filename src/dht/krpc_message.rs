use super::routing_table::Node;
use crate::{
    data_structures::{NoSizeBytes, ID},
    dht::routing_table::COMPACT_NODE_LEN,
    peers::peer::Peer,
};
use anyhow::{anyhow, Result};
use bytes::Bytes;
use serde::{de::Unexpected, Deserialize, Deserializer, Serialize, Serializer};
use std::str;

const MY_TID_LEN: usize = 5;

// TODO fuck bendy! it's the worst decision I've made. I need to impl my own parser for serde.
#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Message {
    Query {
        #[serde(rename = "t")]
        transaction_id: NoSizeBytes,
        #[serde(rename = "y")]
        msg_type: MessageType,
        #[serde(rename = "q")]
        method_name: String,
        #[serde(rename = "a")]
        arguments: Arguments,
    },
    Response {
        #[serde(rename = "t")]
        transaction_id: NoSizeBytes,
        #[serde(rename = "y")]
        msg_type: MessageType,
        #[serde(rename = "r")]
        response: Response,
    },
    Error {
        #[serde(rename = "t")]
        transaction_id: NoSizeBytes,
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
        token: NoSizeBytes,
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
        token: NoSizeBytes,
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

#[derive(Debug, Clone)]
pub enum Nodes {
    Exact(Node),
    Closest(Vec<Node>),
}

#[inline(always)]
pub fn rand_tid() -> NoSizeBytes {
    NoSizeBytes::new(Bytes::from(Vec::from(rand::random::<[u8; MY_TID_LEN]>())))
}

// TODO I'm not a fan of these boilerplate methods. I'm not sure if it's bad, but you can think about it.
impl Message {
    pub fn into_bytes(self) -> Result<Vec<u8>> {
        bendy::serde::to_bytes(&self).map_err(|e| anyhow!("{}", e))
    }

    pub fn from_bytes(buf: &[u8]) -> Result<Self> {
        bendy::serde::from_bytes::<Self>(buf).map_err(|e| anyhow!("{}", e))
    }

    pub fn tid(&self) -> &NoSizeBytes {
        match self {
            Message::Query { transaction_id, .. } => transaction_id,
            Message::Response { transaction_id, .. } => transaction_id,
            Message::Error { transaction_id, .. } => transaction_id,
        }
    }

    pub fn ping_query(id: &ID, transaction_id: NoSizeBytes) -> Self {
        Self::Query {
            transaction_id,
            msg_type: MessageType(b'q'),
            method_name: "ping".to_string(),
            arguments: Arguments::Ping { id: id.to_owned() },
        }
    }

    pub fn ping_resp(id: &ID, transaction_id: Bytes) -> Self {
        Self::Response {
            transaction_id: NoSizeBytes::new(transaction_id),
            msg_type: MessageType(b'r'),
            response: Response::Ping { id: id.to_owned() },
        }
    }

    pub fn find_nodes_query(id: &ID, target: &ID, transaction_id: NoSizeBytes) -> Self {
        Self::Query {
            transaction_id,
            msg_type: MessageType(b'q'),
            method_name: "find_node".to_string(),
            arguments: Arguments::FindNode {
                id: *id,
                target: target.to_owned(),
            },
        }
    }

    pub fn find_nodes_resp(id: &ID, nodes: Nodes, transaction_id: Bytes) -> Self {
        Self::Response {
            transaction_id: NoSizeBytes::new(transaction_id),
            msg_type: MessageType(b'r'),
            response: Response::FindNode { id: *id, nodes },
        }
    }

    pub fn get_peers_query(id: &ID, info_hash: &ID, transaction_id: NoSizeBytes) -> Self {
        Self::Query {
            transaction_id,
            msg_type: MessageType(b'q'),
            method_name: "get_peers".to_string(),
            arguments: Arguments::GetPeers {
                id: id.to_owned(),
                info_hash: info_hash.to_owned(),
            },
        }
    }

    pub fn get_peers_resp(
        id: &ID,
        token: Bytes,
        values_or_nodes: ValuesOrNodes,
        transaction_id: Bytes,
    ) -> Self {
        Self::Response {
            transaction_id: NoSizeBytes::new(transaction_id),
            msg_type: MessageType(b'r'),
            response: Response::GetPeers {
                id: id.to_owned(),
                token: NoSizeBytes::new(token),
                values_or_nodes,
            },
        }
    }

    pub fn announce_peer_query(
        id: &ID,
        info_hash: &ID,
        port: u16,
        token: Bytes,
        transaction_id: NoSizeBytes,
    ) -> Self {
        Self::Query {
            transaction_id,
            msg_type: MessageType(b'q'),
            method_name: "announce_peer".to_string(),
            arguments: Arguments::AnnouncePeer {
                id: id.to_owned(),
                info_hash: info_hash.to_owned(),
                port,
                token: NoSizeBytes::new(token),
            },
        }
    }

    pub fn announce_peer_resp(id: &ID, transaction_id: Bytes) -> Self {
        Self::Response {
            transaction_id: NoSizeBytes::new(transaction_id),
            msg_type: MessageType(b'r'),
            response: Response::AnnouncePeer { id: id.to_owned() },
        }
    }

    pub fn _error_resp(error: Vec<Error>, transaction_id: Bytes) -> Self {
        Self::Error {
            transaction_id: NoSizeBytes::new(transaction_id),
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

        deserializer.deserialize_byte_buf(Visitor {})
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
        $vec.extend_from_slice($node.id.as_byte_ref());
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
                        &format!("k*{COMPACT_NODE_LEN}").as_str(),
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

        deserializer.deserialize_byte_buf(Visitor {})
    }
}

#[cfg(test)]
mod krpc_tests {
    use crate::{data_structures::ID, dht::routing_table::Node};

    macro_rules! encoded_with_custom_tid {
        ($start:expr, $tid:expr) => {{
            let mut encoded = $start.as_bytes().to_vec();

            encoded.push(0x30 + $tid.len() as u8);
            encoded.push(b':');
            for b in $tid {
                encoded.push(b);
            }

            encoded.extend_from_slice(b"1:y1:qe");
            encoded
        }};
    }

    #[test]
    fn node_from_compact() {
        let data = b"qwertyuiopasdfghjklzyhf5aa";
        let result = Node::from_compact_bytes(data).unwrap();

        assert_eq!(result.addr.to_string(), "121.104.102.53:24929");
        assert_eq!(result.id, ID::new(b"qwertyuiopasdfghjklz".to_owned()));

        let long_data = b"qwertyuiopasdfghjklzyhf5aayhf5aa++";
        let result = Node::from_compact_bytes(long_data);
        assert!(result.is_err());

        let short_data = b"yhf";
        let result = Node::from_compact_bytes(short_data);
        assert!(result.is_err());
    }

    mod encode {
        use bytes::Bytes;

        use super::super::Message;
        use crate::{
            data_structures::ID,
            dht::{
                krpc_message::{rand_tid, Error, Nodes, ValuesOrNodes},
                routing_table::Node,
            },
            peers::peer::Peer,
        };
        use std::net::{Ipv4Addr, SocketAddrV4};

        #[test]
        fn ping_query() {
            let message =
                Message::ping_query(&ID::new(b"abcdefghij0123456789".to_owned()), rand_tid());

            let tid = match &message {
                Message::Query { transaction_id, .. } => transaction_id.as_bytes(),
                _ => panic!("expected Message::Query"),
            };

            assert_eq!(
                bendy::serde::to_bytes(&message).unwrap(),
                encoded_with_custom_tid!("d1:ad2:id20:abcdefghij0123456789e1:q4:ping1:t", tid)
            );
        }

        #[test]
        fn ping_resp() {
            let message = Message::ping_resp(
                &ID::new(b"mnopqrstuvwxyz123456".to_owned()),
                Bytes::from_static(b"aa"),
            );

            assert_eq!(
                bendy::serde::to_bytes(&message).unwrap(),
                b"d1:rd2:id20:mnopqrstuvwxyz123456e1:t2:aa1:y1:re"
            );
        }

        #[test]
        fn find_nodes_query() {
            let message = Message::find_nodes_query(
                &ID::new(b"abcdefghij0123456789".to_owned()),
                &ID::new(b"mnopqrstuvwxyz123456".to_owned()),
                rand_tid(),
            );

            let tid = match &message {
                Message::Query { transaction_id, .. } => transaction_id.as_bytes(),
                _ => panic!("expected Message::Query"),
            };

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
                &ID::new(b"0123456789abcdefghij".to_owned()),
                Nodes::Closest(vec![
                    Node::from_compact_bytes(b"rdYAxWC9Zi!A97zKJUbH9HVcgP").unwrap(),
                    Node::from_compact_bytes(b"7Z5cQScmZcC4M2hYKy!JrcYPtT").unwrap(),
                    Node::from_compact_bytes(b"9^jy^pm8sZQy3dukB$CF9^o@of").unwrap(),
                ]),
                Bytes::from_static(b"aa"),
            );

            assert_eq!(
                bendy::serde::to_bytes(&message).unwrap(),
                b"d1:rd2:id20:0123456789abcdefghij5:nodes78:rdYAxWC9Zi!A97zKJUbH9HVcgP7Z5cQScmZcC4M2hYKy!JrcYPtT9^jy^pm8sZQy3dukB$CF9^o@ofe1:t2:aa1:y1:re"
            );
        }

        #[test]
        fn find_nodes_resp_exact() {
            let message = Message::find_nodes_resp(
                &ID::new(b"0123456789abcdefghij".to_owned()),
                Nodes::Exact(Node::from_compact_bytes(b"rdYAxWC9Zi!A97zKJUbH9HVcgP").unwrap()),
                Bytes::from_static(b"aa"),
            );

            assert_eq!(
                bendy::serde::to_bytes(&message).unwrap(),
                b"d1:rd2:id20:0123456789abcdefghij5:nodes26:rdYAxWC9Zi!A97zKJUbH9HVcgPe1:t2:aa1:y1:re"
            );
        }

        #[test]
        fn get_peers_query() {
            let message = Message::get_peers_query(
                &ID::new(b"abcdefghij0123456789".to_owned()),
                &ID::new(b"mnopqrstuvwxyz123456".to_owned()),
                rand_tid(),
            );

            let tid = match &message {
                Message::Query { transaction_id, .. } => transaction_id.as_bytes(),
                _ => panic!("expected Message::Query"),
            };

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
                &ID::new(b"abcdefghij0123456789".to_owned()),
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
                b"d1:rd2:id20:abcdefghij01234567895:token8:aoeusnth6:valuesl6:axje.u6:idhtnmee1:t2:aa1:y1:re"
            );
        }

        #[test]
        fn get_peers_resp_nodes() {
            let message = Message::get_peers_resp(
                &ID::new(b"abcdefghij0123456789".to_owned()),
                Bytes::from_static(b"aoeusnth"),
                ValuesOrNodes::Nodes {
                    nodes: Nodes::Closest(vec![
                        Node::from_compact_bytes(b"rdYAxWC9Zi!A97zKJUbH9HVcgP").unwrap(),
                        Node::from_compact_bytes(b"7Z5cQScmZcC4M2hYKy!JrcYPtT").unwrap(),
                        Node::from_compact_bytes(b"9^jy^pm8sZQy3dukB$CF9^o@of").unwrap(),
                    ]),
                },
                Bytes::from_static(b"aa"),
            );

            assert_eq!(
                bendy::serde::to_bytes(&message).unwrap(),
                b"d1:rd2:id20:abcdefghij01234567895:nodes78:rdYAxWC9Zi!A97zKJUbH9HVcgP7Z5cQScmZcC4M2hYKy!JrcYPtT9^jy^pm8sZQy3dukB$CF9^o@of5:token8:aoeusnthe1:t2:aa1:y1:re"
            );
        }

        #[test]
        fn announce_peer_query() {
            let message = Message::announce_peer_query(
                &ID::new(b"abcdefghij0123456789".to_owned()),
                &ID::new(b"mnopqrstuvwxyz123456".to_owned()),
                6881,
                Bytes::from_static(b"aoeusnth"),
                rand_tid(),
            );

            let tid = match &message {
                Message::Query { transaction_id, .. } => transaction_id.as_bytes(),
                _ => panic!("expected Message::Query"),
            };

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
                &ID::new(b"mnopqrstuvwxyz123456".to_owned()),
                Bytes::from_static(b"aa"),
            );

            assert_eq!(
                bendy::serde::to_bytes(&message).unwrap(),
                b"d1:rd2:id20:mnopqrstuvwxyz123456e1:t2:aa1:y1:re"
            );
        }

        #[test]
        fn error() {
            let message = Message::_error_resp(
                vec![
                    Error::Code(201),
                    Error::Desc("A Generic Error Ocurred".to_string()),
                ],
                Bytes::from_static(b"aa"),
            );

            assert_eq!(
                bendy::serde::to_bytes(&message).unwrap(),
                b"d1:eli201e23:A Generic Error Ocurrede1:t2:aa1:y1:ee"
            );
        }
    }
    mod decode {
        use super::super::Message;
        use crate::{
            data_structures::ID,
            dht::krpc_message::{Arguments, Error, Nodes, Response, ValuesOrNodes},
            peers::peer::Peer,
        };
        use std::net::{Ipv4Addr, SocketAddrV4};

        #[test]
        fn ping_query() {
            let raw_data = b"d1:ad2:id20:abcdefghij0123456789e1:q4:ping1:t2:aa1:y1:qe";
            let data = bendy::serde::from_bytes::<Message>(raw_data).unwrap();

            let Message::Query {
                transaction_id: t,
                msg_type: y,
                method_name: q,
                arguments: a,
            } = data else {
                panic!("message should be Message::Query");
            };

            assert_eq!(t.as_ref(), b"aa");
            assert_eq!(y.0, b'q',);
            assert_eq!(q, "ping");

            let Arguments::Ping { id } = a else {
                panic!("arguments should be Arguments::Ping");
            };

            assert_eq!(ID::new(b"abcdefghij0123456789".to_owned()), id);
        }

        #[test]
        fn ping_resp() {
            let raw_data = b"d1:rd2:id20:mnopqrstuvwxyz123456e1:t2:aa1:y1:re";
            let data = bendy::serde::from_bytes::<Message>(raw_data).unwrap();

            let Message::Response {
                transaction_id: t,
                msg_type: y,
                response: r,
            } = data else {
                panic!("message should be Message::Response");
            };

            assert_eq!(t.as_ref(), b"aa");
            assert_eq!(y.0, b'r');

            let Response::Ping { id } = r else {
                panic!("arguments should be Response::Ping");
            };

            assert_eq!(ID::new(b"mnopqrstuvwxyz123456".to_owned()), id);
        }

        #[test]
        fn find_nodes_query() {
            let raw_data =
                b"d1:ad2:id20:abcdefghij01234567896:target20:mnopqrstuvwxyz123456e1:q9:find_node1:t2:aa1:y1:qe";
            let data = bendy::serde::from_bytes::<Message>(raw_data).unwrap();

            let Message::Query {
                transaction_id: t,
                msg_type: y,
                method_name: q,
                arguments: a,
            } = data else {
                panic!("message should be Message::Query");
            };

            assert_eq!(t.as_ref(), b"aa");
            assert_eq!(y.0, b'q',);
            assert_eq!(q, "find_node");

            let Arguments::FindNode { id, target } = a else {
                panic!("arguments should be Arguments::FindNode");
            };

            assert_eq!(ID::new(b"mnopqrstuvwxyz123456".to_owned()), target);
            assert_eq!(ID::new(b"abcdefghij0123456789".to_owned()), id);
        }

        #[test]
        fn find_nodes_resp_closest() {
            let raw_data =
                b"d1:rd2:id20:0123456789abcdefghij5:nodes78:rdYAxWC9Zi!A97zKJUbH9HVcgP7Z5cQScmZcC4M2hYKy!JrcYPtT9^jy^pm8sZQy3dukB$CF9^o@ofe1:t2:aa1:y1:re";
            let data = bendy::serde::from_bytes::<Message>(raw_data).unwrap();

            let Message::Response {
                transaction_id: t,
                msg_type: y,
                response: r,
            } = data else {
                panic!("message should be Message::Response");
            };

            assert_eq!(t.as_ref(), b"aa");
            assert_eq!(y.0, b'r');

            let Response::FindNode { id, nodes } = r else {
                panic!("response should be Response::FindNode");
            };

            assert_eq!(ID::new(b"0123456789abcdefghij".to_owned()), id);

            let Nodes::Closest(node_list) = nodes else {
                panic!("expected Nodes::Closest");
            };

            assert_eq!(node_list.len(), 3);
            assert_eq!(node_list[1].id, ID::new(b"7Z5cQScmZcC4M2hYKy!J".to_owned()));
            assert_eq!(Ok(node_list[2].addr), "57.94.111.64:28518".parse());
        }

        #[test]
        fn find_nodes_resp_exact() {
            let raw_data = b"d1:rd2:id20:0123456789abcdefghij5:nodes26:rdYAxWC9Zi!A97zKJUbH9HVcgPe1:t2:aa1:y1:re";
            let data = bendy::serde::from_bytes::<Message>(raw_data).unwrap();

            let Message::Response {
                transaction_id: t,
                msg_type: y,
                response: r,
            } = data else {
                panic!("message should be Message::Response");
            };

            assert_eq!(t.as_ref(), b"aa");
            assert_eq!(y.0, b'r');

            let Response::FindNode { id, nodes } = r else {
                panic!("response should be Response::FindNode");
            };
            assert_eq!(ID::new(b"0123456789abcdefghij".to_owned()), id);

            let Nodes::Exact(node) = nodes else {
                    panic!("expected Nodes::Exact");
                };

            assert_eq!(node.id, ID::new(b"rdYAxWC9Zi!A97zKJUbH".to_owned()));
            assert_eq!(node.addr, "57.72.86.99:26448".parse().unwrap())
        }

        #[test]
        fn get_peers_query() {
            let raw_data =
                b"d1:ad2:id20:abcdefghij01234567899:info_hash20:mnopqrstuvwxyz123456e1:q9:get_peers1:t2:aa1:y1:qe";
            let data = bendy::serde::from_bytes::<Message>(raw_data).unwrap();

            let Message::Query {
                transaction_id: t,
                msg_type: y,
                method_name: q,
                arguments: a,
            } = data else {
                panic!("message should be Message::Query");
            };

            assert_eq!(t.as_ref(), b"aa");
            assert_eq!(y.0, b'q',);
            assert_eq!(q, "get_peers");

            let Arguments::GetPeers { id, info_hash } = a else {
                panic!("arguments should be Arguments::GetPeers");
            };

            assert_eq!(ID::new(b"abcdefghij0123456789".to_owned()), id);
            assert_eq!(ID::new(b"mnopqrstuvwxyz123456".to_owned()), info_hash);
        }

        #[test]
        fn get_peers_resp_vals() {
            let raw_data =
                b"d1:rd2:id20:abcdefghij01234567895:token8:aoeusnth6:valuesl6:axje.u6:idhtnmee1:t2:aa1:y1:re";
            let data = bendy::serde::from_bytes::<Message>(raw_data).unwrap();

            let Message::Response {
                transaction_id: t,
                msg_type: y,
                response: r,
            } = data else {
                panic!("message should be Message::Response");
            };

            assert_eq!(t.as_ref(), b"aa");
            assert_eq!(y.0, b'r');

            let Response::GetPeers {
                id,
                token,
                values_or_nodes,
            } = r else {
                panic!("response should be Response::GetPeers");
            };

            assert_eq!(ID::new(b"abcdefghij0123456789".to_owned()), id);
            assert_eq!(token.as_ref(), b"aoeusnth");

            let ValuesOrNodes::Values { values } = values_or_nodes else {
                panic!("expected values (peers)");
            };

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

        #[test]
        fn get_peers_resp_nodes() {
            let raw_data = b"d1:rd2:id20:abcdefghij01234567895:nodes78:rdYAxWC9Zi!A97zKJUbH9HVcgP7Z5cQScmZcC4M2hYKy!JrcYPtT9^jy^pm8sZQy3dukB$CF9^o@of5:token8:aoeusnthe1:t2:aa1:y1:re"
            ;
            let data = bendy::serde::from_bytes::<Message>(raw_data).unwrap();

            let Message::Response {
                transaction_id: t,
                msg_type: y,
                response: r,
            } = data else {
                panic!("message should be Message::Response");
            };

            assert_eq!(t.as_ref(), b"aa");
            assert_eq!(y.0, b'r');

            let Response::GetPeers {
                id,
                token,
                values_or_nodes,
            } = r else {
                panic!("response should be Response::GetPeers");
            };

            assert_eq!(ID::new(b"abcdefghij0123456789".to_owned()), id);
            assert_eq!(token.as_ref(), b"aoeusnth");

            let ValuesOrNodes::Nodes { nodes } = values_or_nodes else {
                panic!("expected nodes");
            };

            let Nodes::Closest(node_list) = nodes else {
                panic!("expected list of nodes");
            };

            assert_eq!(node_list.len(), 3);
            assert_eq!(node_list[1].id, ID::new(b"7Z5cQScmZcC4M2hYKy!J".to_owned()));
            assert_eq!(Ok(node_list[2].addr), "57.94.111.64:28518".parse());
        }

        #[test]
        fn announce_peer_query() {
            let raw_data = b"d1:ad2:id20:abcdefghij012345678912:implied_porti1e9:info_hash20:mnopqrstuvwxyz1234564:porti6881e5:token8:aoeusnthe1:q13:announce_peer1:t2:aa1:y1:qe";
            let data = bendy::serde::from_bytes::<Message>(raw_data).unwrap();

            let Message::Query {
                transaction_id: t,
                msg_type: y,
                method_name: q,
                arguments: a,
            } = data else {
                panic!("message should be Message::Query");
            };

            assert_eq!(t.as_ref(), b"aa");
            assert_eq!(y.0, b'q',);
            assert_eq!(q, "announce_peer");

            let Arguments::AnnouncePeer {
                id,
                info_hash,
                port,
                token,
            } = a else {
                panic!("arguments should be Arguments::Ping");
            };

            assert_eq!(ID::new(b"abcdefghij0123456789".to_owned()), id);
            assert_eq!(ID::new(b"mnopqrstuvwxyz123456".to_owned()), info_hash);
            assert_eq!(b"aoeusnth", token.as_ref());
            assert_eq!(6881, port);
        }

        #[test]
        fn announce_peer_resp() {
            // I don't see a way to differentiate this and ping response
            // I doubt it matters tho, let's TODO but probably not
            // future G here: it can be identified with tid
            // everything else is self-describing..
            // I'm not a fan of this protocol
        }

        #[test]
        fn error() {
            let raw_data = b"d1:eli201e23:A Generic Error Ocurrede1:t2:aa1:y1:ee";
            let data = bendy::serde::from_bytes::<Message>(raw_data).unwrap();

            let Message::Error {
                transaction_id: t,
                msg_type: y,
                error: e,
            } = data else {
                panic!("message should be Message::Error");
            };

            assert_eq!(t.as_ref(), b"aa");
            assert_eq!(y.0, b'e');

            match e[0] {
                Error::Code(code) => assert_eq!(code, 201),
                _ => panic!("first element in error message list should be error code"),
            }

            match &e[1] {
                Error::Desc(desc) => assert_eq!(desc, "A Generic Error Ocurred"),
                _ => panic!("second element in error message list should be error message"),
            }
        }
    }
}
