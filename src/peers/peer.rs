use crate::transcoding::{socketaddr_from_compact_bytes, COMPACT_SOCKADDR_LEN};
use anyhow::{bail, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::net::{SocketAddr, SocketAddrV4};

pub const COMPACT_PEER_LEN: usize = COMPACT_SOCKADDR_LEN;

#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub struct Peer(SocketAddrV4);

impl Peer {
    pub fn new(addr: SocketAddrV4) -> Self {
        Self(addr)
    }

    fn from_compact_bytes(buff: &[u8]) -> Result<Self> {
        if buff.len() == COMPACT_PEER_LEN {
            Ok(Peer(socketaddr_from_compact_bytes(buff)?))
        } else {
            bail!(
                "Peer::from_compact buff size is {}, expected {}",
                buff.len(),
                COMPACT_PEER_LEN
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
        let mut v = Vec::with_capacity(COMPACT_PEER_LEN);

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
                if v.len() == COMPACT_SOCKADDR_LEN {
                    Peer::from_compact_bytes(v).map_err(serde::de::Error::custom)
                } else {
                    Err(serde::de::Error::invalid_length(v.len(), &self))
                }
            }
        }

        deserializer.deserialize_byte_buf(Visitor {})
    }
}

impl ToString for Peer {
    fn to_string(&self) -> String {
        self.0.to_string()
    }
}

impl From<&Peer> for Peer {
    fn from(value: &Peer) -> Self {
        value.to_owned()
    }
}

impl From<SocketAddrV4> for Peer {
    fn from(value: SocketAddrV4) -> Self {
        Self(value)
    }
}

impl From<SocketAddr> for Peer {
    fn from(value: SocketAddr) -> Self {
        match value {
            SocketAddr::V4(addr) => Self(addr),
            SocketAddr::V6(addr) => panic!("no support for IPv6 ({addr:?})"),
        }
    }
}

impl From<Peer> for SocketAddrV4 {
    fn from(value: Peer) -> Self {
        value.0
    }
}

impl From<Peer> for SocketAddr {
    fn from(value: Peer) -> Self {
        SocketAddr::V4(value.0)
    }
}

#[cfg(test)]
mod tests {
    use super::{socketaddr_from_compact_bytes, Peer};
    use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};

    #[test]
    fn peer_from_compact() {
        let data = "yhf5aa".as_bytes();
        let result = Peer::from_compact_bytes(data);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().addr().to_string(), "121.104.102.53:24929");

        let long_data = "yhf5aa++".as_bytes();
        let result = Peer::from_compact_bytes(long_data);
        assert!(result.is_err());

        let short_data = "yhf".as_bytes();
        let result = Peer::from_compact_bytes(short_data);
        assert!(result.is_err());

        let short_data = "yhf".as_bytes();
        let result = socketaddr_from_compact_bytes(short_data);
        assert!(result.is_err());
    }

    #[test]
    fn from_impls() {
        let _: Peer = (&Peer("127.0.0.1:52".parse().unwrap())).into();
        let peer: Peer = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 53).into();
        let _: SocketAddrV4 = peer.into();
        let peer: Peer = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 54).into();
        let _: SocketAddr = peer.into();
    }
}
