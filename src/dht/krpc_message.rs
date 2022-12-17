use crate::{
    constants::{ID_LEN, SIX, T26IX},
    util::{functions::socketaddr_from_compact_bytes, id::ID},
};
use anyhow::{bail, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::net::SocketAddrV4;

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Message<'a> {
    Query {
        #[serde(rename = "t")]
        transaction_id: &'a str,
        #[serde(rename = "y")]
        msg_type: &'a str,
        #[serde(rename = "q")]
        method_name: &'a str,
        #[serde(rename = "a")]
        #[serde(borrow)]
        arguments: Arguments<'a>,
    },
    Response {
        #[serde(rename = "t")]
        transaction_id: &'a str,
        #[serde(rename = "y")]
        msg_type: &'a str,
        #[serde(borrow)]
        #[serde(rename = "r")]
        response: Response<'a>,
    },
    Error {
        #[serde(rename = "t")]
        transaction_id: &'a str,
        #[serde(rename = "y")]
        msg_type: &'a str,
        #[serde(rename = "e")]
        #[serde(borrow)]
        error: Vec<Error<'a>>,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Arguments<'a> {
    AnnouncePeer {
        id: ID,
        info_hash: ID,
        port: u16,
        token: &'a str,
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
pub enum Response<'a> {
    GetPeers {
        id: ID,
        token: &'a str,
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
pub enum Error<'a> {
    Code(u32),
    Desc(&'a str),
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

#[derive(Debug)]
pub struct Node {
    id: ID,
    addr: SocketAddrV4,
}

impl Peer {
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
}

impl Node {
    fn from_compact_bytes(buff: &[u8]) -> Result<Self> {
        if buff.len() == T26IX {
            let (id, addr) = buff.split_at(ID_LEN);

            Ok(Node {
                id: ID(id.try_into()?),
                addr: socketaddr_from_compact_bytes(addr)?,
            })
        } else {
            bail!(
                "Node::from_compact buff size is {}, expected {}",
                buff.len(),
                T26IX
            )
        }
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

impl Serialize for Nodes {
    fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut v = Vec::new();

        match self {
            Self::Exact(node) => {
                v.extend_from_slice(node.id.as_bytes());
                v.extend_from_slice(&node.addr.ip().octets());
                v.push((node.addr.port() >> 8) as u8);
                v.push((node.addr.port() & 0xff) as u8);
            }
            Self::Closest(nodes) => {
                for node in nodes {
                    v.extend_from_slice(node.id.as_bytes());
                    v.extend_from_slice(&node.addr.ip().octets());
                    v.push((node.addr.port() >> 8) as u8);
                    v.push((node.addr.port() & 0xff) as u8);
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
                if v.len() % T26IX != 0 {
                    Err(serde::de::Error::invalid_length(
                        v.len(),
                        &format!("k*{}", T26IX).as_str(),
                    ))
                } else {
                    if v.len() / T26IX == 1 {
                        let node = Node::from_compact_bytes(v).map_err(serde::de::Error::custom)?;
                        Ok(Nodes::Exact(node))
                    } else {
                        let mut vec = Vec::with_capacity(v.len() / T26IX);

                        for chunk in v.chunks(T26IX) {
                            vec.push(
                                Node::from_compact_bytes(chunk)
                                    .map_err(serde::de::Error::custom)?,
                            )
                        }

                        Ok(Nodes::Closest(vec))
                    }
                }
            }
        }

        Ok(deserializer.deserialize_byte_buf(Visitor {})?)
    }
}

#[cfg(test)]
mod krpc_tests {
    use crate::{dht::krpc_message::Node, util::id::ID};

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
            dht::krpc_message::{Arguments, Error, Node, Nodes, Peer, Response, ValuesOrNodes},
            util::id::ID,
        };
        use std::net::{Ipv4Addr, SocketAddrV4};

        #[test]
        fn ping_query() {
            let data = Message::Query {
                transaction_id: "aa",
                msg_type: "q",
                method_name: "ping",
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
                transaction_id: "aa",
                msg_type: "r",
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
                transaction_id: "aa",
                msg_type: "q",
                method_name: "find_node",
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
                transaction_id: "aa",
                msg_type: "r",
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
                transaction_id: "aa",
                msg_type: "r",
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
                transaction_id: "aa",
                msg_type: "q",
                method_name: "get_peers",
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
                transaction_id: "aa",
                msg_type: "r",
                response: Response::GetPeers {
                    id: ID([
                        97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 48, 49, 50, 51, 52, 53, 54,
                        55, 56, 57,
                    ]),
                    token: "aoeusnth",
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
                transaction_id: "aa",
                msg_type: "r",
                response: Response::GetPeers {
                    id: ID([
                        97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 48, 49, 50, 51, 52, 53, 54,
                        55, 56, 57,
                    ]),
                    token: "aoeusnth",
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
                transaction_id: "aa",
                msg_type: "q",
                method_name: "announce_peer",
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
                    token: "aoeusnth",
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
                transaction_id: "aa",
                msg_type: "r",
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
                transaction_id: "aa",
                msg_type: "e",
                error: vec![Error::Code(201), Error::Desc("A Generic Error Ocurred")],
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
                assert_eq!(y, "q");
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
            let raw_data ="d1:ad2:id20:abcdefghij01234567896:target20:mnopqrstuvwxyz123456e1:q9:find_node1:t2:aa1:y1:qe".as_bytes();
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
                assert_eq!(y, "q");
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
            let raw_data =
                "d1:rd2:id20:0123456789abcdefghij5:nodes26:rdYAxWC9Zi!A97zKJUbH9HVcgPe1:t2:aa1:y1:re".as_bytes();
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
            let raw_data ="d1:ad2:id20:abcdefghij01234567899:info_hash20:mnopqrstuvwxyz123456e1:q9:get_peers1:t2:aa1:y1:qe".as_bytes();
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
                assert_eq!(y, "q");
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
            let raw_data = "d1:rd2:id20:abcdefghij01234567895:token8:aoeusnth6:valuesl6:axje.u6:idhtnmee1:t2:aa1:y1:re".as_bytes();
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
                assert_eq!(y, "q");
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

                assert!(matches!(e[1], Error::Desc { .. }));
                if let Error::Desc(desc) = e[1] {
                    assert_eq!(desc, "A Generic Error Ocurred");
                }
            } else {
                assert!(false);
            }
        }
    }
}
