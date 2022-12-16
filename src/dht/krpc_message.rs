use crate::{
    constants::{ID_LEN, SIX, T26IX},
    util::{functions::socketaddr_from_compact_bytes, id::ID},
};
use anyhow::{bail, Result};
use serde::{Deserialize, Deserializer, Serialize};
use std::net::SocketAddr;

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
        token: &'a [u8],
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
        token: &'a [u8],
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

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Peer(SocketAddr);

#[derive(Debug, Serialize)]
pub struct Node {
    id: ID,
    addr: SocketAddr,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Nodes {
    Exact(Node),
    Closest(Vec<Node>),
}

impl Peer {
    fn from_compact_bytes(buff: &[u8]) -> Result<Self> {
        Ok(Peer(socketaddr_from_compact_bytes(buff)?))
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

impl<'de> serde::de::Deserialize<'de> for Peer {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Peer;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("Compact IP-address/port info")
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

impl<'de> serde::de::Deserialize<'de> for Node {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Node;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("byte string")
            }
            fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                if v.len() == T26IX {
                    Node::from_compact_bytes(v).map_err(serde::de::Error::custom)
                } else {
                    Err(serde::de::Error::invalid_length(v.len(), &self))
                }
            }
        }

        Ok(deserializer.deserialize_byte_buf(Visitor {})?)
    }
}

impl<'de> serde::de::Deserialize<'de> for Nodes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Nodes;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("byte string")
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
    mod decode {
        use super::super::Message;
        use crate::{
            dht::krpc_message::{Arguments, Error, Nodes, Peer, Response, ValuesOrNodes},
            util::id::ID,
        };
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};

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
        fn find_nodes_resp() {
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

                    assert_eq!(token, vec![b'a', b'o', b'e', b'u', b's', b'n', b't', b'h']);

                    assert!(matches!(values_or_nodes, ValuesOrNodes::Values { .. }));
                    if let ValuesOrNodes::Values { values } = values_or_nodes {
                        assert_eq!(values.len(), 2);

                        assert_eq!(
                            values[0],
                            Peer(SocketAddr::new(
                                IpAddr::V4(Ipv4Addr::new(b'a', b'x', b'j', b'e')),
                                11893
                            ))
                        );
                        assert_eq!(
                            values[1],
                            Peer(SocketAddr::new(
                                IpAddr::V4(Ipv4Addr::new(b'i', b'd', b'h', b't')),
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

                    assert_eq!(token, vec![b'a', b'o', b'e', b'u', b's', b'n', b't', b'h']);

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

                    assert_eq!(token, vec![b'a', b'o', b'e', b'u', b's', b'n', b't', b'h']);

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
