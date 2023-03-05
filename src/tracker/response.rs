use crate::ok_or_missing_field;
use crate::peers::peer::Peer;
use crate::transcoding::{socketaddr_from_compact_bytes, COMPACT_SOCKADDR_LEN};
use anyhow::Result;
use bendy::decoding::{FromBencode, Object};
use bendy::encoding::AsString;
use tracing::warn;

#[derive(Debug, Clone)]
pub struct Response {
    pub warning_message: Option<String>,
    pub interval: u64,
    pub min_interval: Option<u64>,
    pub tracker_id: Option<String>,
    pub complete: u64,
    pub incomplete: u64,
    pub peers: Vec<Peer>,
}

impl FromBencode for Response {
    const EXPECTED_RECURSION_DEPTH: usize = 2;

    fn decode_bencode_object(object: Object) -> Result<Self, bendy::decoding::Error> {
        let mut warning_message = None;
        let mut interval = None;
        let mut min_interval = None;
        let mut tracker_id = None;
        let mut complete = None;
        let mut incomplete = None;
        let mut peers = None;

        let mut info = object.try_into_dictionary()?;
        while let Some(kv) = info.next_pair()? {
            match kv {
                (b"warning message", value) => {
                    warning_message = Some(String::decode_bencode_object(value)?);
                }
                (b"interval", value) => {
                    interval = Some(u64::decode_bencode_object(value)?);
                }
                (b"min interval", value) => {
                    min_interval = Some(u64::decode_bencode_object(value)?);
                }
                (b"tracker id", value) => {
                    tracker_id = Some(String::decode_bencode_object(value)?);
                }
                (b"complete", value) => {
                    complete = Some(u64::decode_bencode_object(value)?);
                }
                (b"incomplete", value) => {
                    incomplete = Some(u64::decode_bencode_object(value)?);
                }
                (b"peers", value) => {
                    if let Some(peer_bytes) =
                        AsString::decode_bencode_object(value).map(|bytes| Some(bytes.0))?
                    {
                        let x = peer_bytes
                            .chunks_exact(COMPACT_SOCKADDR_LEN)
                            .filter_map(|compact_peer| {
                                match socketaddr_from_compact_bytes(compact_peer) {
                                    Ok(peer_addr) => Some(Peer::new(peer_addr)),
                                    Err(e) => {
                                        warn!(?e);
                                        None
                                    }
                                }
                            })
                            .collect();

                        peers = Some(x);
                    }
                }
                _ => (),
            }
        }

        Ok(Response {
            warning_message,
            interval: ok_or_missing_field!(interval)?,
            min_interval,
            tracker_id,
            complete: ok_or_missing_field!(complete)?,
            incomplete: ok_or_missing_field!(incomplete)?,
            peers: ok_or_missing_field!(peers)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Response;
    use crate::peers::peer::Peer;
    use anyhow::anyhow;
    use bendy::decoding::{Decoder, FromBencode};
    use std::net::{Ipv4Addr, SocketAddrV4};

    #[test]
    fn parse_response() {
        let bytes = vec![
            0x64, 0x38, 0x3a, 0x63, 0x6f, 0x6d, 0x70, 0x6c, 0x65, 0x74, 0x65, 0x69, 0x35, 0x65,
            0x31, 0x30, 0x3a, 0x64, 0x6f, 0x77, 0x6e, 0x6c, 0x6f, 0x61, 0x64, 0x65, 0x64, 0x69,
            0x35, 0x33, 0x65, 0x31, 0x30, 0x3a, 0x69, 0x6e, 0x63, 0x6f, 0x6d, 0x70, 0x6c, 0x65,
            0x74, 0x65, 0x69, 0x31, 0x65, 0x38, 0x3a, 0x69, 0x6e, 0x74, 0x65, 0x72, 0x76, 0x61,
            0x6c, 0x69, 0x31, 0x39, 0x31, 0x34, 0x65, 0x31, 0x32, 0x3a, 0x6d, 0x69, 0x6e, 0x20,
            0x69, 0x6e, 0x74, 0x65, 0x72, 0x76, 0x61, 0x6c, 0x69, 0x39, 0x35, 0x37, 0x65, 0x35,
            0x3a, 0x70, 0x65, 0x65, 0x72, 0x73, 0x33, 0x36, 0x3a, 0x9f, 0x45, 0x41, 0x9d, 0x1a,
            0xe7, 0x9f, 0x45, 0x41, 0x9d, 0xfe, 0x72, 0x9f, 0x45, 0x41, 0x9d, 0xc8, 0x70, 0x9f,
            0x45, 0x41, 0x9d, 0xab, 0x24, 0x9f, 0x45, 0x41, 0x9d, 0x4c, 0xb7, 0x9f, 0x45, 0x41,
            0x9d, 0x37, 0x02, 0x65,
        ];

        let response = match Decoder::new(&bytes).next_object() {
            Ok(decoder_object_option) => match decoder_object_option {
                Some(decoder_object) => match Response::decode_bencode_object(decoder_object) {
                    Ok(tracker_response) => Ok(tracker_response),
                    Err(_) => Err(anyhow!("tracker response is not struct Response")),
                },
                None => Err(anyhow!("should be unreachable for the first object")),
            },
            Err(_) => Err(anyhow!("tracker response: not bencoded")),
        }
        .unwrap();

        assert_eq!(response.warning_message, None);
        assert_eq!(response.interval, 1914);
        assert_eq!(response.min_interval, Some(957));
        assert_eq!(response.tracker_id, None);
        assert_eq!(response.complete, 5);
        assert_eq!(response.incomplete, 1);
        assert_eq!(
            response.peers[0],
            Peer::new(SocketAddrV4::new(
                Ipv4Addr::new(0x9f, 0x45, 0x41, 0x9d),
                6887
            ))
        );
        assert_eq!(
            response.peers[1],
            Peer::new(SocketAddrV4::new(
                Ipv4Addr::new(0x9f, 0x45, 0x41, 0x9d),
                65138
            ))
        );
        assert_eq!(
            response.peers[2],
            Peer::new(SocketAddrV4::new(
                Ipv4Addr::new(0x9f, 0x45, 0x41, 0x9d),
                51312
            ))
        );
        assert_eq!(
            response.peers[3],
            Peer::new(SocketAddrV4::new(
                Ipv4Addr::new(0x9f, 0x45, 0x41, 0x9d),
                43812
            ))
        );
        assert_eq!(
            response.peers[4],
            Peer::new(SocketAddrV4::new(
                Ipv4Addr::new(0x9f, 0x45, 0x41, 0x9d),
                19639
            ))
        );
        assert_eq!(
            response.peers[5],
            Peer::new(SocketAddrV4::new(
                Ipv4Addr::new(0x9f, 0x45, 0x41, 0x9d),
                14082
            ))
        );
    }
}
