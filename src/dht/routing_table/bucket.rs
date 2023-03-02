use super::{COMPACT_NODE_LEN, K_NODE_PER_BUCKET};
use crate::data_structures::{ID, ID_LEN};
use crate::transcoding::socketaddr_from_compact_bytes;
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt;
use std::net::SocketAddrV4;
use std::time::Duration;
use tokio::time::Instant;

const NODE_VALIDITY_SECS: u64 = 15 * 60;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Bucket {
    depth: usize,
    data: [Option<Node>; K_NODE_PER_BUCKET],
}

#[derive(Eq, Clone, Deserialize, Serialize, PartialOrd)]
pub struct Node {
    pub id: ID,
    pub addr: SocketAddrV4,
    #[serde(skip)]
    last_seen: Option<Instant>,
}

impl Ord for Node {
    fn cmp(&self, other: &Self) -> Ordering {
        self.id.cmp(&other.id)
    }
}

impl Bucket {
    pub fn new(depth: usize) -> Self {
        Self {
            depth,
            data: [(); K_NODE_PER_BUCKET].map(|_| Option::<Node>::default()),
        }
    }

    pub fn get_node(&self, id: &ID) -> Option<&Node> {
        self.data
            .iter()
            .find(|node| Node::is_some_with_id(node, id).is_some())?
            .as_ref()
    }

    pub fn get_node_mut(&mut self, id: &ID) -> Option<&mut Node> {
        self.data
            .iter_mut()
            .find(|node| Node::is_some_with_id(node, id).is_some())?
            .as_mut()
    }

    pub fn try_insert(&mut self, id: &ID, addr: &SocketAddrV4) -> Result<()> {
        if let Some(node) = self.get_node_mut(id) {
            node.touch();
            return Ok(());
        }

        let replaceable_node = self
            .find_replaceable_node()
            .ok_or(anyhow!("bucket if full"))?;

        *replaceable_node = Some(Node {
            id: id.to_owned(),
            addr: addr.to_owned(),
            last_seen: Some(Instant::now()),
        });

        Ok(())
    }

    pub fn split(self) -> (Self, Self) {
        let mut one = Bucket::new(self.depth + 1);
        let mut zero = Bucket::new(self.depth + 1);

        let mut cur_left_idx = 0;
        let mut cur_right_idx = 0;

        for node in self.data.into_iter().flatten() {
            if node.id.get_bit(self.depth) {
                one.data[cur_right_idx] = Some(node);
                cur_right_idx += 1;
            } else {
                zero.data[cur_left_idx] = Some(node);
                cur_left_idx += 1;
            }
        }

        (zero, one)
    }

    pub fn contains(&self, id: &ID) -> bool {
        self.get_node(id).is_some()
    }

    pub fn is_full(&self) -> bool {
        self.find_replaceable_node_idx().is_none()
    }

    pub fn depth(&self) -> usize {
        self.depth
    }

    pub fn iter_nodes(&self) -> impl Iterator<Item = &Node> {
        self.data.iter().flatten()
    }

    pub fn iter_over_goods(&self) -> impl Iterator<Item = &Node> {
        self.data.iter().filter_map(Node::is_some_and_good)
    }

    pub fn id_within_bucket_range(&self, own_id: &ID, id: &ID) -> bool {
        match own_id.first_diff_bit_idx(id) {
            None => true,
            Some(first_diff) => first_diff > self.depth,
        }
    }

    fn find_replaceable_node(&mut self) -> Option<&mut Option<Node>> {
        let node_idx = self.find_replaceable_node_idx()?;
        Some(&mut self.data[node_idx])
    }

    fn find_replaceable_node_idx(&self) -> Option<usize> {
        if let Some(vacant_node) = self.data.iter().position(|x| x.is_none()) {
            return Some(vacant_node);
        }

        if let Some(bad_node) = self
            .data
            .iter()
            .position(|x| Node::is_some_and_good(x).is_none())
        {
            return Some(bad_node);
        }

        None
    }

    pub fn _count_bads(&self) -> usize {
        let exists_and_is_bad = |node_opt: &Option<Node>| {
            let node = node_opt.as_ref()?;
            let last_seen = node.last_seen.as_ref()?;
            (last_seen.elapsed() > Duration::from_secs(15 * 60)).then_some(())
        };

        self.data.iter().fold(0, |acc, x| {
            if exists_and_is_bad(x).is_some() {
                acc + 1
            } else {
                acc
            }
        })
    }

    pub fn _count_empty(&self) -> usize {
        self.data
            .iter()
            .fold(0, |acc, x| if x.is_none() { acc + 1 } else { acc })
    }
}

impl Node {
    pub fn new_good(id: ID, addr: SocketAddrV4) -> Self {
        Self {
            id,
            addr,
            last_seen: Some(Instant::now()),
        }
    }

    pub fn from_compact_bytes(buff: &[u8]) -> Result<Self> {
        if buff.len() == COMPACT_NODE_LEN {
            let (id, addr) = buff.split_at(ID_LEN);

            Ok(Self {
                id: ID::new(id.try_into()?),
                addr: socketaddr_from_compact_bytes(addr)?,
                last_seen: None,
            })
        } else {
            bail!(
                "Node::from_compact buff size is {}, expected {}",
                buff.len(),
                COMPACT_NODE_LEN
            )
        }
    }

    pub fn touch(&mut self) {
        self.last_seen = Some(Instant::now());
    }

    fn eq_by_id(&self, id: &ID) -> bool {
        self.id == *id
    }

    pub fn cmp_by_id(&self, id: &ID) -> Ordering {
        self.id.cmp(id)
    }

    pub fn cmp_by_distance_to_id(&self, other: &Self, id: &ID) -> Ordering {
        let distance_to_self = &self.id - id;
        let distance_to_other = &other.id - id;

        distance_to_self.cmp(&distance_to_other)
    }

    fn is_some_and_good(node_opt: &Option<Self>) -> Option<&Self> {
        let node = node_opt.as_ref()?;
        let last_seen_instant = node.last_seen.as_ref()?;

        if last_seen_instant.elapsed() < Duration::from_secs(NODE_VALIDITY_SECS) {
            Some(node)
        } else {
            None
        }
    }

    fn is_some_with_id(node: &Option<Node>, id: &ID) -> Option<()> {
        node.as_ref()?.eq_by_id(id).then_some(())
    }
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl fmt::Debug for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut ds = f.debug_struct("Node");
        let beginning = ds.field("id", &self.id).field("addr", &self.addr);

        match &self.last_seen {
            Some(instant) => beginning.field("last_seen", &instant.elapsed()).finish(),
            None => beginning.finish_non_exhaustive(),
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::time::Instant;
    use tracing::debug;
    use tracing_test::traced_test;

    use crate::{
        data_structures::{ID, ID_LEN},
        dht::routing_table::bucket::NODE_VALIDITY_SECS,
        transcoding::{socketaddr_from_compact_bytes, COMPACT_SOCKADDR_LEN},
    };
    use std::{net::SocketAddrV4, time::Duration};

    use super::{Bucket, K_NODE_PER_BUCKET};

    fn random_node() -> (ID, SocketAddrV4) {
        let id = ID::new(rand::random());

        let raw_addr: [u8; COMPACT_SOCKADDR_LEN] = rand::random();
        let addr = socketaddr_from_compact_bytes(&raw_addr).unwrap();

        (id, addr)
    }

    #[test]
    fn insert() {
        let depth = 0;
        let mut bucket = Bucket::new(depth);

        let nodes: Vec<(ID, SocketAddrV4)> =
            (0..K_NODE_PER_BUCKET).map(|_| random_node()).collect();

        for (id, addr) in nodes.iter() {
            assert!(bucket.try_insert(id, addr).is_ok());
        }

        // reinsert does touch
        assert!(bucket.try_insert(&nodes[0].0, &nodes[0].1).is_ok());
        assert!(
            bucket.data[0].as_ref().unwrap().last_seen.unwrap()
                > bucket.data[1].as_ref().unwrap().last_seen.unwrap()
        );
        assert!(
            bucket.data[2].as_ref().unwrap().last_seen.unwrap()
                > bucket.data[1].as_ref().unwrap().last_seen.unwrap()
        );

        // can't insert in full
        let (id, addr) = random_node();
        assert!(bucket.try_insert(&id, &addr).is_err());

        // can insert in expired
        let node_idx = 2;
        let node = bucket.data[node_idx].as_mut().unwrap();
        node.last_seen = Some(Instant::now() - Duration::from_secs(NODE_VALIDITY_SECS));
        let (id, addr) = &nodes[node_idx];
        assert!(bucket.try_insert(id, addr).is_ok());
    }

    #[test]
    #[traced_test]
    fn split() {
        let depth = 3;
        let mut bucket = Bucket::new(depth);

        let nodes = [
            0b0110_0000,
            0b0110_0100,
            0b0110_1000,
            0b0110_1100,
            0b0111_0000,
            0b0111_0100,
            0b0111_1100,
        ]
        .map(|first_byte| {
            let mut id_array = [0; ID_LEN];
            id_array[0] = first_byte;

            let id = ID::new(id_array);

            let raw_addr: [u8; COMPACT_SOCKADDR_LEN] = rand::random();
            let addr = socketaddr_from_compact_bytes(&raw_addr).unwrap();

            (id, addr)
        });

        for (id, addr) in nodes.iter() {
            assert!(bucket.try_insert(id, addr).is_ok());
        }

        let (zero, one) = bucket.split();

        assert_eq!(4, zero.iter_over_goods().count());
        assert_eq!(4, zero._count_empty());

        assert_eq!(3, one.iter_over_goods().count());
        assert_eq!(5, one._count_empty());

        assert_eq!(depth + 1, zero.depth());
        assert_eq!(depth + 1, one.depth());
    }
}
