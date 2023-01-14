use anyhow::{bail, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::hash_map::Entry::Vacant;
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Instant;

use crate::data_structures::id::ID;

pub const COMPACT_SOCKADDR_LEN: usize = 6;
pub const COMPACT_PEER_LEN: usize = COMPACT_SOCKADDR_LEN;

#[derive(Debug, PartialEq, Eq, Clone, Hash)]
pub struct Peer(SocketAddrV4);

impl Peer {
    pub fn new(addr: SocketAddrV4) -> Self {
        Self(addr)
    }

    fn from_compact_bytes(buff: &[u8]) -> Result<Self> {
        if buff.len() == COMPACT_SOCKADDR_LEN {
            Ok(Peer(socketaddr_from_compact_bytes(buff)?))
        } else {
            bail!(
                "Peer::from_compact buff size is {}, expected {}",
                buff.len(),
                COMPACT_SOCKADDR_LEN
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
                if v.len() == COMPACT_SOCKADDR_LEN {
                    Peer::from_compact_bytes(v).map_err(serde::de::Error::custom)
                } else {
                    Err(serde::de::Error::invalid_length(v.len(), &self))
                }
            }
        }

        Ok(deserializer.deserialize_byte_buf(Visitor {})?)
    }
}

#[derive(Debug)]
pub enum InactivenessReason {
    Unknown,
    UnableToConnect,
    ChokedForTooLong,
}

#[derive(Debug)]
struct PeerEntry {
    active: bool,
    inactiveness_reason: InactivenessReason,
    last_connection_instant: Option<Instant>,
}

#[derive(Clone, Debug)]
pub struct Peers {
    info_hash: ID,
    data: Arc<StdMutex<HashMap<Peer, PeerEntry>>>,
}

impl Peers {
    pub fn new(info_hash: &ID) -> Self {
        Self {
            info_hash: info_hash.to_owned(),
            data: Arc::new(StdMutex::new(HashMap::new())),
        }
    }

    pub fn serve(&self, info_hash: &ID) -> bool {
        *info_hash == self.info_hash
    }

    pub fn get_random(&self) -> Option<Peer> {
        let mut data = self.data.lock().unwrap();

        let no_or_min_time_entry = data.iter_mut().min_by(|x, y| {
            x.1.last_connection_instant
                .cmp(&y.1.last_connection_instant)
        });

        match no_or_min_time_entry {
            Some((sock_addr, entry)) => {
                entry.active = true;
                Some(sock_addr.to_owned())
            }
            None => None,
        }
    }

    // pub fn closed(&self, sa: &SocketAddrV4, reason: InactivenessReason) {
    //     let mut data = self.data.lock().unwrap();

    //     if let Some(entry) = data.get_mut(sa) {
    //         entry.active = false;
    //         entry.inactiveness_reason = reason;
    //         entry.last_connection_instant = Some(Instant::now());
    //     } // TODO else should be impossible
    // }

    // TODO this should be mutable... probably?
    pub fn insert_list(&self, list: &Vec<Peer>) {
        let mut data = self.data.lock().unwrap();

        for socket_address in list {
            if let Vacant(entry) = data.entry(socket_address.clone()) {
                entry.insert(PeerEntry {
                    active: false,
                    inactiveness_reason: InactivenessReason::Unknown,
                    last_connection_instant: None,
                });
            }
        }
    }

    pub fn peer_addresses(&self) -> Vec<Peer> {
        self.data.lock().unwrap().keys().cloned().collect()
    }
}

pub fn socketaddr_from_compact_bytes(buf: &[u8]) -> Result<SocketAddrV4> {
    match buf.len() {
        COMPACT_SOCKADDR_LEN => Ok(SocketAddrV4::new(
            Ipv4Addr::new(buf[0], buf[1], buf[2], buf[3]),
            ((buf[4] as u16) << 8) | buf[5] as u16,
        )),
        _ => bail!(
            "socketaddr_from_compact_bytes: buffer len expected {} found {}",
            COMPACT_SOCKADDR_LEN,
            buf.len()
        ),
    }
}

#[cfg(test)]
mod tests {
    use crate::peer::Peer;

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
}
