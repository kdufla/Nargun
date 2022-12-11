use crate::{
    constants::COMPACT_NODE_LEN,
    util::{
        functions::{ok_or_missing_field, vec_to_id},
        id::ID,
    },
};
use anyhow::Result;
use bendy::{
    decoding::{FromBencode, Object},
    encoding::AsString,
};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

#[derive(Debug)]
pub struct Message {
    transaction_id: Vec<u8>,
    content: TypeOfMessage,
}

#[derive(Debug)]
enum TypeOfMessage {
    Query { args: Arguments },
    Response { resp: Response },
    Error { err: Error },
}

#[derive(Debug)]
enum Arguments {
    Ping {
        id: ID,
    },
    FindNode {
        id: ID,
        target: ID,
    },
    GetPeers {
        id: ID,
        info_hash: ID,
    },
    AnnouncePeer {
        id: ID,
        info_hash: ID,
        port: u16,
        token: Vec<u8>,
    },
}

#[derive(Debug)]
enum Response {
    Ping {
        id: ID,
    },
    FindNode {
        id: ID,
        nodes: Nodes,
    },
    GetPeers {
        id: ID,
        token: Vec<u8>,
        values_or_nodes: ValuesOrNodes,
    },
    AnnouncePeer {
        id: ID,
    },
}

#[derive(Debug)]
enum Nodes {
    Exact(Node),
    Closest(Vec<Node>),
}

#[derive(Debug)]
struct Node {
    id: ID,
    addr: SocketAddr,
}

#[derive(Debug, PartialEq, Eq)]
struct Peer(SocketAddr);

#[derive(Debug)]
enum ValuesOrNodes {
    Values(Vec<Peer>),
    Nodes(Nodes),
}

#[derive(Debug)]
struct Error {
    code: u32,
    desc: String,
}

impl FromBencode for Message {
    const EXPECTED_RECURSION_DEPTH: usize = 10;

    fn decode_bencode_object(object: Object) -> Result<Self, bendy::decoding::Error> {
        let mut t = None;
        let mut a: Option<Arguments> = None;
        let mut r: Option<Response> = None;
        let mut e: Option<Error> = None;

        let mut message = object.try_into_dictionary()?;
        while let Some(kv) = message.next_pair()? {
            match kv {
                (b"t", value) => {
                    t = AsString::decode_bencode_object(value).map(|bytes| Some(bytes.0))?;
                }
                (b"a", value) => {
                    a = Some(Arguments::decode_bencode_object(value)?);
                }
                (b"r", value) => {
                    r = Some(Response::decode_bencode_object(value)?);
                }
                (b"e", value) => {
                    e = Some(Error::decode_bencode_object(value)?);
                }
                _ => (),
            }
        }

        Ok(Self {
            transaction_id: t.ok_or_else(|| bendy::decoding::Error::missing_field("t"))?,
            content: if let Some(aa) = a {
                TypeOfMessage::Query { args: aa }
            } else if let Some(rr) = r {
                TypeOfMessage::Response { resp: rr }
            } else if let Some(ee) = e {
                TypeOfMessage::Error { err: ee }
            } else {
                Err(bendy::decoding::Error::missing_field("a/r/e"))?
            },
        })
    }
}

impl FromBencode for Arguments {
    const EXPECTED_RECURSION_DEPTH: usize = 10;

    fn decode_bencode_object(object: Object) -> Result<Self, bendy::decoding::Error> {
        let mut id = None;
        let mut target = None;
        let mut info_hash = None;
        let mut port = None;
        let mut token = None;

        let mut args = object.try_into_dictionary()?;
        while let Some(kv) = args.next_pair()? {
            match kv {
                (b"id", value) => {
                    id = AsString::decode_bencode_object(value).map(|bytes| Some(bytes.0))?;
                }
                (b"target", value) => {
                    target = AsString::decode_bencode_object(value).map(|bytes| Some(bytes.0))?;
                }
                (b"info_hash", value) => {
                    info_hash =
                        AsString::decode_bencode_object(value).map(|bytes| Some(bytes.0))?;
                }
                (b"port", value) => {
                    port = Some(u16::decode_bencode_object(value)?);
                }
                (b"token", value) => {
                    token = AsString::decode_bencode_object(value).map(|bytes| Some(bytes.0))?;
                }
                _ => (),
            }
        }

        Ok(if let Some(target) = target {
            Self::FindNode {
                id: vec_to_id(ok_or_missing_field(id, "id")?),
                target: vec_to_id(target),
            }
        } else if let Some(port) = port {
            Self::AnnouncePeer {
                id: vec_to_id(ok_or_missing_field(id, "id")?),
                info_hash: vec_to_id(ok_or_missing_field(info_hash, "info_hash")?),
                port,
                token: ok_or_missing_field(token, "token")?,
            }
        } else if let Some(info_hash) = info_hash {
            Self::GetPeers {
                id: vec_to_id(ok_or_missing_field(id, "id")?),
                info_hash: vec_to_id(info_hash),
            }
        } else if let Some(id) = id {
            Self::Ping { id: vec_to_id(id) }
        } else {
            Err(bendy::decoding::Error::missing_field("arguments"))?
        })
    }
}

impl FromBencode for Response {
    const EXPECTED_RECURSION_DEPTH: usize = 10;

    fn decode_bencode_object(object: Object) -> Result<Self, bendy::decoding::Error> {
        let mut id = None;
        let mut token = None;
        let mut nodes = None;
        let mut values = None;

        let mut info = object.try_into_dictionary()?;
        while let Some(kv) = info.next_pair()? {
            match kv {
                (b"id", value) => {
                    id = AsString::decode_bencode_object(value).map(|bytes| Some(bytes.0))?;
                }
                (b"nodes", value) => {
                    nodes = Some(Nodes::decode_bencode_object(value)?);
                }
                (b"values", value) => {
                    values = Some(Vec::<Peer>::decode_bencode_object(value)?);
                }
                (b"token", value) => {
                    token = AsString::decode_bencode_object(value).map(|bytes| Some(bytes.0))?;
                }
                _ => (),
            }
        }

        Ok(if let Some(token) = token {
            Self::GetPeers {
                id: vec_to_id(ok_or_missing_field(id, "id")?),
                token: token,
                values_or_nodes: ok_or_missing_field(
                    if let Some(values) = values {
                        Some(ValuesOrNodes::Values(values))
                    } else if let Some(nodes) = nodes {
                        Some(ValuesOrNodes::Nodes(nodes))
                    } else {
                        None
                    },
                    "values or nodes",
                )?,
            }
        } else if let Some(nodes) = nodes {
            Self::FindNode {
                id: vec_to_id(ok_or_missing_field(id, "id")?),
                nodes,
            }
        } else if let Some(id) = id {
            Self::Ping { id: vec_to_id(id) }
        } else {
            Err(bendy::decoding::Error::missing_field("response args"))?
        })
    }
}

impl FromBencode for Error {
    const EXPECTED_RECURSION_DEPTH: usize = 10;

    fn decode_bencode_object(object: Object) -> Result<Self, bendy::decoding::Error> {
        Ok(Error {
            code: 69,
            desc: "bla bla bla".to_string(),
        })
    }
}

impl FromBencode for Nodes {
    const EXPECTED_RECURSION_DEPTH: usize = 10;

    fn decode_bencode_object(object: Object) -> Result<Self, bendy::decoding::Error> {
        let bytes = object.try_into_bytes()?;

        if bytes.len() == COMPACT_NODE_LEN {
            let (id, bytes) = bytes.split_at(20);
            let id: [u8; 20] = id.try_into().unwrap();

            let ip = Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]);
            let mut port = bytes[4] as u16;
            port = (port << 8) + bytes[5] as u16;

            Ok(Nodes::Exact(Node {
                id: ID(id),
                addr: SocketAddr::new(IpAddr::V4(ip), port),
            }))
        } else {
            let node_count = bytes.len() / COMPACT_NODE_LEN;
            let mut nodes = Vec::<Node>::with_capacity(node_count);

            for i in 0..node_count {
                let (_, bytes) = bytes.split_at(i * COMPACT_NODE_LEN);
                let (id, bytes) = bytes.split_at(20);

                let id: [u8; 20] = id.try_into().unwrap();

                let ip = Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]);
                let mut port = bytes[4] as u16;
                port = (port << 8) + bytes[5] as u16;

                nodes.push(Node {
                    id: ID(id),
                    addr: SocketAddr::new(IpAddr::V4(ip), port),
                });
            }
            Ok(Nodes::Closest(nodes))
        }
    }
}

impl FromBencode for Peer {
    const EXPECTED_RECURSION_DEPTH: usize = 10;

    fn decode_bencode_object(object: Object) -> Result<Self, bendy::decoding::Error> {
        let bytes = object.try_into_bytes()?;

        let ip = Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]);
        let mut port = bytes[4] as u16;
        port = (port << 8) + bytes[5] as u16;

        Ok(Peer(SocketAddr::new(IpAddr::V4(ip), port)))
    }
}

#[cfg(test)]
mod krpc_tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use bendy::decoding::FromBencode;

    use crate::{
        dht::krpc_message::{Arguments, Nodes, Peer, Response, TypeOfMessage, ValuesOrNodes},
        util::id::ID,
    };

    use super::Message;

    #[test]
    fn ping_q() {
        let q = "d1:ad2:id20:abcdefghij0123456789e1:q4:ping1:t2:aa1:y1:qe".as_bytes();
        let query = Message::from_bencode(q).unwrap();

        assert_eq!(query.transaction_id, vec![97, 97]);
        assert!(matches!(query.content, TypeOfMessage::Query { .. }));
        if let TypeOfMessage::Query { args } = query.content {
            assert!(matches!(args, Arguments::Ping { .. }));
            if let Arguments::Ping { id } = args {
                assert_eq!(
                    ID([
                        97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 48, 49, 50, 51, 52, 53, 54,
                        55, 56, 57
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
    fn ping_r() {
        let r = "d1:rd2:id20:mnopqrstuvwxyz123456e1:t2:aa1:y1:re".as_bytes();
        let response = Message::from_bencode(r).unwrap();

        assert_eq!(response.transaction_id, vec![97, 97]);
        assert!(matches!(response.content, TypeOfMessage::Response { .. }));
        if let TypeOfMessage::Response { resp } = response.content {
            assert!(matches!(resp, Response::Ping { .. }));
            if let Response::Ping { id } = resp {
                assert_eq!(
                    ID([
                        109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 49,
                        50, 51, 52, 53, 54
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
    fn find_nodes_q() {
        let fnq = "d1:ad2:id20:abcdefghij01234567896:target20:mnopqrstuvwxyz123456e1:q9:find_node1:t2:aa1:y1:qe".as_bytes();
        let query = Message::from_bencode(fnq).unwrap();

        assert_eq!(query.transaction_id, vec![97, 97]);
        assert!(matches!(query.content, TypeOfMessage::Query { .. }));
        if let TypeOfMessage::Query { args } = query.content {
            assert!(matches!(args, Arguments::FindNode { .. }));
            if let Arguments::FindNode { id, target } = args {
                assert_eq!(
                    ID([
                        97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 48, 49, 50, 51, 52, 53, 54,
                        55, 56, 57
                    ]),
                    id
                );

                assert_eq!(
                    ID([
                        109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 49,
                        50, 51, 52, 53, 54
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
    fn find_nodes_r() {
        let fnr = "d1:rd2:id20:0123456789abcdefghij5:nodes9:def456...e1:t2:aa1:y1:re".as_bytes();
        let response = Message::from_bencode(fnr).unwrap();

        assert_eq!(response.transaction_id, vec![97, 97]);
        assert!(matches!(response.content, TypeOfMessage::Response { .. }));
        if let TypeOfMessage::Response { resp } = response.content {
            assert!(matches!(resp, Response::FindNode { .. }));
            if let Response::FindNode { id, nodes } = resp {
                assert_eq!(
                    ID([
                        48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 97, 98, 99, 100, 101, 102, 103,
                        104, 105, 106
                    ]),
                    id
                );

                // TODO nodes
            } else {
                assert!(false);
            }
        } else {
            assert!(false);
        }
    }

    #[test]
    fn get_peers_q() {
        let gpq = "d1:ad2:id20:abcdefghij01234567899:info_hash20:mnopqrstuvwxyz123456e1:q9:get_peers1:t2:aa1:y1:qe".as_bytes();
        let query = Message::from_bencode(gpq).unwrap();

        assert_eq!(query.transaction_id, vec![97, 97]);
        assert!(matches!(query.content, TypeOfMessage::Query { .. }));
        if let TypeOfMessage::Query { args } = query.content {
            assert!(matches!(args, Arguments::GetPeers { .. }));
            if let Arguments::GetPeers { id, info_hash } = args {
                assert_eq!(
                    ID([
                        97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 48, 49, 50, 51, 52, 53, 54,
                        55, 56, 57
                    ]),
                    id
                );

                assert_eq!(
                    ID([
                        109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 49,
                        50, 51, 52, 53, 54
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
    fn get_peers_vr() {
        let gprv = "d1:rd2:id20:abcdefghij01234567895:token8:aoeusnth6:valuesl6:axje.u6:idhtnmee1:t2:aa1:y1:re".as_bytes();
        let response = Message::from_bencode(gprv).unwrap();

        assert_eq!(response.transaction_id, vec![97, 97]);
        assert!(matches!(response.content, TypeOfMessage::Response { .. }));
        if let TypeOfMessage::Response { resp } = response.content {
            assert!(matches!(resp, Response::GetPeers { .. }));
            if let Response::GetPeers {
                id,
                token,
                values_or_nodes,
            } = resp
            {
                assert_eq!(
                    ID([
                        97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 48, 49, 50, 51, 52, 53, 54,
                        55, 56, 57
                    ]),
                    id
                );

                assert_eq!(token, vec![b'a', b'o', b'e', b'u', b's', b'n', b't', b'h']);

                assert!(matches!(values_or_nodes, ValuesOrNodes::Values { .. }));
                if let ValuesOrNodes::Values(values) = values_or_nodes {
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
    fn get_peers_nr() {
        let gprn =
            "d1:rd2:id20:abcdefghij01234567895:nodes78:rdYAxWC9Zi!A97zKJUbH9HVcgP7Z5cQScmZcC4M2hYKy!JrcYPtT9^jy^pm8sZQy3dukB$CF9^o@of5:token8:aoeusnthe1:t2:aa1:y1:re"
                .as_bytes();
        let response = Message::from_bencode(gprn).unwrap();

        assert_eq!(response.transaction_id, vec![97, 97]);
        assert!(matches!(response.content, TypeOfMessage::Response { .. }));
        if let TypeOfMessage::Response { resp } = response.content {
            assert!(matches!(resp, Response::GetPeers { .. }));
            if let Response::GetPeers {
                id,
                token,
                values_or_nodes,
            } = resp
            {
                assert_eq!(
                    ID([
                        97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 48, 49, 50, 51, 52, 53, 54,
                        55, 56, 57
                    ]),
                    id
                );

                assert_eq!(token, vec![b'a', b'o', b'e', b'u', b's', b'n', b't', b'h']);

                assert!(matches!(values_or_nodes, ValuesOrNodes::Nodes { .. }));
                if let ValuesOrNodes::Nodes(nodes) = values_or_nodes {
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
        } else {
            assert!(false);
        }
    }

    #[test]
    fn announce_peer_q() {
        let aq = "d1:ad2:id20:abcdefghij012345678912:implied_porti1e9:info_hash20:mnopqrstuvwxyz1234564:porti6881e5:token8:aoeusnthe1:q13:announce_peer1:t2:aa1:y1:qe".as_bytes();
        let query = Message::from_bencode(aq).unwrap();

        assert_eq!(query.transaction_id, vec![97, 97]);
        assert!(matches!(query.content, TypeOfMessage::Query { .. }));
        if let TypeOfMessage::Query { args } = query.content {
            assert!(matches!(args, Arguments::AnnouncePeer { .. }));
            if let Arguments::AnnouncePeer {
                id,
                info_hash,
                port,
                token,
            } = args
            {
                assert_eq!(
                    ID([
                        97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 48, 49, 50, 51, 52, 53, 54,
                        55, 56, 57
                    ]),
                    id
                );

                assert_eq!(
                    ID([
                        109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 49,
                        50, 51, 52, 53, 54
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
    fn announce_peer_r() {
        // I don't see a way to differentiate this and ping response
        // I doubt it matters tho, let's TODO but probably not
    }
}
