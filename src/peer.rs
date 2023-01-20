use crate::data_structures::id::ID;
use anyhow::{bail, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;
use std::mem::discriminant;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::{Arc, Mutex as StdMutex};

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

        Ok(deserializer.deserialize_byte_buf(Visitor {})?)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    // TODO these need Instants and weighted sort based on elapsed time
    Active,
    Unknown,
    UnableToConnect,
    ChokedForTooLong,
    NoUsefulPieces,
}

#[derive(Clone, Debug)]
pub struct Peers {
    info_hash: ID,
    data: Arc<StdMutex<HashMap<Peer, Status>>>,
}

impl Peers {
    pub fn new(info_hash: &ID) -> Self {
        Self {
            info_hash: info_hash.to_owned(),
            data: Arc::new(StdMutex::new(HashMap::new())),
        }
    }

    // TODO this name is bad. `peers.serve(info_hash)` is clear but only seeing it here is laughably bad.
    pub fn serve(&self, info_hash: &ID) -> bool {
        *info_hash == self.info_hash
    }

    pub fn peer_addresses(&self) -> Vec<Peer> {
        self.data.lock().unwrap().keys().cloned().collect()
    }

    // I know this name/function is shit but I don't want to block peers more than it's necessary
    // TODO for later but it's chabuduo for now
    pub fn return_batch_of_bad_peers_and_get_new_batch(
        &mut self,
        bad_peers: &Vec<(Peer, Status)>,
        limit: usize,
    ) -> Vec<Peer> {
        let status_priority_order = [
            Status::Unknown,
            Status::NoUsefulPieces,
            Status::ChokedForTooLong,
            Status::UnableToConnect,
        ];

        let mut data = self.data.lock().unwrap();

        self.update_locked_data_with_bad_peers(&mut data, bad_peers);

        self.activate_peers_in_locked_data(&mut data, limit, &status_priority_order)
    }

    fn update_locked_data_with_bad_peers(
        &self,
        data: &mut HashMap<Peer, Status>,
        bad_peers: &Vec<(Peer, Status)>,
    ) {
        for (peer, status) in bad_peers.iter().cloned() {
            data.insert(peer, status);
        }
    }

    fn activate_peers_in_locked_data(
        &self,
        data: &mut HashMap<Peer, Status>,
        limit: usize,
        status_priority_order: &[Status],
    ) -> Vec<Peer> {
        let mut newly_activated_peers = Vec::with_capacity(std::cmp::min(data.len(), limit));

        for cur_status in status_priority_order {
            let iter = data
                .iter_mut()
                .filter_map(|(peer, status)| {
                    if discriminant(status) == discriminant(&cur_status) {
                        *status = Status::Active;
                        Some(peer.to_owned())
                    } else {
                        None
                    }
                })
                .take(limit - newly_activated_peers.len());

            newly_activated_peers.extend(iter);
        }

        newly_activated_peers
    }
}

impl<P> Extend<P> for Peers
where
    P: Into<Peer>,
{
    fn extend<T: IntoIterator<Item = P>>(&mut self, iter: T) {
        let mut data = self.data.lock().unwrap();

        data.extend(iter.into_iter().map(|peer| (peer.into(), Status::Unknown)));
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
            SocketAddr::V6(addr) => panic!("no support for IPv6 ({:?})", addr),
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
    use super::Peers;
    use crate::data_structures::id::ID;
    use crate::peer::{socketaddr_from_compact_bytes, Peer, Status};
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

    // TODO I don't like this test, but most of the logic is dependent on previous lines.
    // So if I want to split into 3 tests, I have to just write 1/3 same, 2/3 same and same.
    // Splitting up into multiple non-test functions might help, I guess, but not really
    #[test]
    fn peers() {
        let info_hash = ID::new(rand::random());
        let mut peers = Peers::new(&info_hash);

        assert!(peers.serve(&info_hash));
        assert!(!peers.serve(&ID::new(rand::random())));

        let new_peers: [Peer; 3] = [
            Peer("127.0.0.1:52".parse().unwrap()),
            Peer("127.0.0.1:53".parse().unwrap()),
            Peer("127.0.0.1:54".parse().unwrap()),
        ];

        // data = [(52, unknown),(53, unknown),(54, unknown)]
        peers.extend(new_peers.iter());

        let data = peers.data.lock().unwrap();
        assert_eq!(&Status::Unknown, data.get(&new_peers[0]).unwrap());
        assert_eq!(&Status::Unknown, data.get(&new_peers[1]).unwrap());
        assert_eq!(&Status::Unknown, data.get(&new_peers[2]).unwrap());
        drop(data);

        // data is two actives and one unknown
        let unknown_peers = peers.return_batch_of_bad_peers_and_get_new_batch(&Vec::new(), 2);
        let data = peers.data.lock().unwrap();

        // got two peers, in data they're marked as active
        assert_eq!(2, unknown_peers.len());
        assert_eq!(&Status::Active, data.get(&unknown_peers[0]).unwrap());
        assert_eq!(&Status::Active, data.get(&unknown_peers[1]).unwrap());

        let return_peers = vec![(unknown_peers[0].clone(), Status::UnableToConnect)];

        drop(data);
        let unknown_peers_2 = peers.return_batch_of_bad_peers_and_get_new_batch(&return_peers, 100);
        let data = peers.data.lock().unwrap();

        // returned 1 and asked for 100 but 1 left + 1 just_returned = 2
        assert_eq!(2, unknown_peers_2.len());
        assert_eq!(&Status::Active, data.get(&unknown_peers_2[0]).unwrap());
        assert_eq!(&Status::Active, data.get(&unknown_peers_2[1]).unwrap());
        drop(data);

        // returned had known status with lower priority, it's last/second in the list
        assert_ne!(return_peers[0].0, unknown_peers_2[0]);
        assert_eq!(return_peers[0].0, unknown_peers_2[1]);

        assert_eq!(3, peers.peer_addresses().len());
    }
}
