use super::krpc_message::Nodes;
use crate::data_structures::{ID, ID_LEN};
use crate::transcoding::{socketaddr_from_compact_bytes, COMPACT_SOCKADDR_LEN};
use anyhow::{anyhow, bail, Result};
use rand::{thread_rng, Rng};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt;
use std::net::SocketAddrV4;
use std::path::PathBuf;
use std::time::Duration;
use tokio::fs::{DirBuilder, File};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::Instant;
use tracing::{debug, error};

pub const COMPACT_NODE_LEN: usize = ID_LEN + COMPACT_SOCKADDR_LEN;
const K_NODE_PER_BUCKET: usize = 8;
const APP_DIR_NAME: &str = ".nargun";
const CACHED_DHT_FILE: &str = "dht";

#[derive(Debug, Deserialize, Serialize)]
pub struct RoutingTable {
    own_id: ID,
    data: Vec<TreeNode>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
enum TreeNode {
    // when a bucket is split, 1 is stored as a right child and vice versa
    Parent(usize, usize),
    Leaf(Bucket),
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
struct Bucket {
    depth: usize,
    data: [Option<Node>; K_NODE_PER_BUCKET],
}

#[derive(Eq, Clone, Deserialize, Serialize)]
pub struct Node {
    pub id: ID,
    pub addr: SocketAddrV4,
    #[serde(skip)]
    last_seen: Option<Instant>,
}

impl RoutingTable {
    pub async fn new() -> Self {
        match Self::try_loading_from_disk().await {
            Ok(cache) => cache,
            Err(e) => {
                error!(?e);
                Self::default()
            }
        }
    }

    async fn try_loading_from_disk() -> Result<Self> {
        let mut path = home::home_dir().ok_or(anyhow!("can't find home dir"))?;
        path.push(APP_DIR_NAME);
        path.push(CACHED_DHT_FILE);

        let mut buf = Vec::new();

        let mut file = File::open(path).await?;
        file.read_to_end(&mut buf).await?;

        Ok(bincode::deserialize(&buf)?)
    }

    pub async fn store(&self) {
        let mut path = home::home_dir()
            .ok_or(anyhow!("can't find home dir"))
            .unwrap();
        path.push(APP_DIR_NAME);

        let mut builder = DirBuilder::new();
        builder.recursive(true).create(&path).await.unwrap();

        let encoded = bincode::serialize(&self).unwrap();

        path.push(CACHED_DHT_FILE);
        let mut file = File::create(path).await.unwrap();

        file.write_all(&encoded).await.unwrap();
    }

    pub fn own_id(&self) -> &ID {
        &self.own_id
    }

    pub fn find_node(&self, id: &ID) -> Option<Nodes> {
        let bucket = self.get_bucket(id);

        if let Some(exact_node) = bucket.get_node(id) {
            return Some(Nodes::Exact(exact_node.clone()));
        }

        self.get_closest_nodes(id, bucket)
    }

    pub fn iter_over_ids_within_fillable_buckets(&self) -> impl Iterator<Item = ID> + '_ {
        RTIter {
            data: self,
            depth: 0,
            idx: 0,
        }
    }

    pub fn touch_node(&mut self, id: ID, addr: SocketAddrV4) {
        let bucket = self.get_bucket_mut(&id);

        if let Some(node) = bucket.get_node_mut(&id) {
            node.last_seen = Some(Instant::now());
        } else {
            self.insert_good(id.to_owned(), addr);
        }
    }

    pub fn contains_node(&self, id: &ID) -> bool {
        self.get_bucket(id).contains(id)
    }

    fn get_closest_nodes<'a>(&'a self, id: &ID, mut bucket: &'a Bucket) -> Option<Nodes> {
        let mut closest = Vec::with_capacity(16);
        let mut prev_depth = usize::MAX;
        let mut sibling_id = id.to_owned();

        while closest.len() < K_NODE_PER_BUCKET && prev_depth != bucket.depth {
            prev_depth = bucket.depth;

            closest.extend(bucket.iter_over_goods().cloned());

            // next closest nodes are descendants of sibling TreeNode
            sibling_id.flip_bit(bucket.depth);
            bucket = self.get_bucket(&sibling_id);
        }

        if !closest.is_empty() {
            closest.sort_by(|a, b| a.cmp_by_distance_to_id(b, id));
            debug!("found closest for {:?}, closest: {:?}", id, closest);
            Some(Nodes::Closest(closest))
        } else {
            debug!("no closest nodes for {:?}", id);
            None
        }
    }

    fn get_bucket(&self, id: &ID) -> &Bucket {
        let bucket_idx = self.find_bucket_idx_by_id(id);

        let TreeNode::Leaf(bucket) = &self.data[bucket_idx] else {
            panic!("this should always be a leaf");
        };

        bucket
    }

    fn get_bucket_mut(&mut self, id: &ID) -> &mut Bucket {
        let bucket_idx = self.find_bucket_idx_by_id(id);

        let TreeNode::Leaf(bucket) = &mut self.data[bucket_idx] else {
            panic!("this should always be a leaf");
        };

        bucket
    }

    fn find_bucket_idx_by_id(&self, id: &ID) -> usize {
        let mut data_iter_idx = 0;

        for depth in 0..160 {
            match &self.data[data_iter_idx] {
                TreeNode::Parent(left_child_idx, right_child_idx) => {
                    data_iter_idx =
                        id.left_or_right_by_depth(depth, *left_child_idx, *right_child_idx)
                }
                TreeNode::Leaf(_) => return data_iter_idx,
            }
        }

        panic!("find_bucket_idx_by_id: could not find (impossible)");
    }

    fn insert_good(&mut self, id: ID, addr: SocketAddrV4) {
        if self.contains_node(&id) {
            self.touch_node(id, addr);
            return;
        }

        debug!("insert {:?}", id);

        let own_id = self.own_id.to_owned();
        let bucket = self.get_bucket_mut(&id);

        if bucket.try_insert(&id, &addr).is_ok() {
            return;
        }

        let own_id_within_the_bucket = own_id
            .cmp_first_n_bits(&bucket.data[0].as_ref().unwrap().id, bucket.depth)
            .is_eq();

        if !own_id_within_the_bucket {
            bucket.replace(&id, &addr);
            return;
        }

        self.split(id, addr, None);
    }

    fn split(&mut self, id: ID, addr: SocketAddrV4, bucket_idx: Option<usize>) {
        let bucket_idx = match bucket_idx {
            Some(idx) => idx,
            None => self.find_bucket_idx_by_id(&id),
        };

        let bucket = self.get_bucket_at(bucket_idx).unwrap();
        if bucket.depth == 160 {
            return;
        }

        let og_bucket_depth = bucket.depth;
        let (left, right) = bucket.split();

        debug!("split bucket_idx = {:?} bucket = {:?}", bucket_idx, bucket);
        debug!("split into {:?} {:?}", left, right);

        let (left_idx, right_idx) = self.push_children(bucket_idx, left, right);
        let cur_idx = id.left_or_right_by_depth(og_bucket_depth, left_idx, right_idx);

        let bucket = self.get_bucket_at_mut(cur_idx).unwrap();
        if bucket.try_insert(&id, &addr).is_ok() {
            return;
        }

        self.split(id, addr, Some(cur_idx))
    }

    fn get_bucket_at(&self, i: usize) -> Option<&Bucket> {
        match &self.data[i] {
            TreeNode::Parent(_, _) => None,
            TreeNode::Leaf(bucket) => Some(bucket),
        }
    }

    fn get_bucket_at_mut(&mut self, i: usize) -> Option<&mut Bucket> {
        match &mut self.data[i] {
            TreeNode::Parent(_, _) => None,
            TreeNode::Leaf(bucket) => Some(bucket),
        }
    }

    fn push_children(&mut self, parent: usize, left: Bucket, right: Bucket) -> (usize, usize) {
        self.data.push(TreeNode::Leaf(left));
        self.data.push(TreeNode::Leaf(right));

        let left_idx = self.data.len() - 2;
        let right_idx = self.data.len() - 1;

        self.data[parent] = TreeNode::Parent(left_idx, right_idx);

        (left_idx, right_idx)
    }
}

impl Bucket {
    fn new(depth: usize) -> Self {
        Self {
            depth,
            data: [(); K_NODE_PER_BUCKET].map(|_| Option::<Node>::default()),
        }
    }

    fn split(&self) -> (Self, Self) {
        let mut right = Bucket::new(self.depth + 1);
        let mut left = Bucket::new(self.depth + 1);

        let mut cur_left_idx = 0;
        let mut cur_right_idx = 0;

        for node in self.data.iter().flatten() {
            if node.id.get_bit(self.depth) {
                right.data[cur_right_idx] = Some(node.to_owned());
                cur_right_idx += 1;
            } else {
                left.data[cur_left_idx] = Some(node.to_owned());
                cur_left_idx += 1;
            }
        }

        (left, right)
    }

    fn contains(&self, id: &ID) -> bool {
        self.get_node(id).is_some()
    }

    fn get_node(&self, id: &ID) -> Option<&Node> {
        self.data
            .iter()
            .find(|node| Node::is_some_with_id(node, id).is_some())?
            .as_ref()
    }

    fn get_node_mut(&mut self, id: &ID) -> Option<&mut Node> {
        self.data
            .iter_mut()
            .find(|node| Node::is_some_with_id(node, id).is_some())?
            .as_mut()
    }

    fn replace(&mut self, id: &ID, addr: &SocketAddrV4) {
        let replaceable_idx = match self.find_replaceable_node_idx() {
            Ok(idx) => idx,
            Err(_) => thread_rng().gen_range(0..K_NODE_PER_BUCKET),
        };

        self.data[replaceable_idx] = Some(Node {
            id: id.to_owned(),
            addr: addr.to_owned(),
            last_seen: Some(Instant::now()),
        });
    }

    fn try_insert(&mut self, id: &ID, addr: &SocketAddrV4) -> Result<()> {
        *self.find_replaceable_node()? = Some(Node {
            id: id.to_owned(),
            addr: addr.to_owned(),
            last_seen: Some(Instant::now()),
        });

        Ok(())
    }

    fn find_replaceable_node(&mut self) -> Result<&mut Option<Node>> {
        Ok(&mut self.data[self.find_replaceable_node_idx()?])
    }

    fn find_replaceable_node_idx(&self) -> Result<usize> {
        if let Some(vacant_node) = self.data.iter().position(|x| x.is_none()) {
            return Ok(vacant_node);
        }

        if let Some(bad_node) = self
            .data
            .iter()
            .position(|x| Node::is_some_and_good(x).is_none())
        {
            return Ok(bad_node);
        }

        Err(anyhow!("bucket is full"))
    }

    fn iter_over_goods(&self) -> impl Iterator<Item = &Node> {
        self.data.iter().filter_map(Node::is_some_and_good)
    }

    fn _count_bads(&self) -> usize {
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

    fn _count_empty(&self) -> usize {
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

    fn eq_by_id(&self, id: &ID) -> bool {
        self.id == *id
    }

    fn cmp_by_distance_to_id(&self, other: &Self, id: &ID) -> Ordering {
        let distance_to_self = &self.id - id;
        let distance_to_other = &other.id - id;

        distance_to_self.cmp(&distance_to_other)
    }

    fn is_some_and_good(node_opt: &Option<Self>) -> Option<&Self> {
        let node = node_opt.as_ref()?;
        let last_seen_instant = node.last_seen.as_ref()?;

        if last_seen_instant.elapsed() < Duration::from_secs(15 * 60) {
            Some(node)
        } else {
            None
        }
    }

    fn is_some_with_id(node: &Option<Node>, id: &ID) -> Option<()> {
        node.as_ref()?.eq_by_id(id).then_some(())
    }
}

impl Default for RoutingTable {
    fn default() -> Self {
        let mut data = Vec::with_capacity(320);
        data.push(TreeNode::Leaf(Bucket::new(0)));

        Self {
            own_id: ID::new(rand::random()),
            data,
        }
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

struct RTIter<'a> {
    data: &'a RoutingTable,
    idx: usize,
    depth: usize,
}

impl<'a> Iterator for RTIter<'a> {
    type Item = ID;

    fn next(&mut self) -> Option<Self::Item> {
        let mut rv = self.data.own_id.to_owned();
        loop {
            if self.depth > 160 {
                return None;
            }

            match &self.data.data[self.idx] {
                TreeNode::Parent(left_child_idx, right_child_idx) => {
                    self.idx = self.data.own_id.left_or_right_by_depth(
                        self.depth,
                        *left_child_idx,
                        *right_child_idx,
                    );

                    rv.flip_bit(self.depth);
                }
                TreeNode::Leaf(_) => {
                    if self.depth == 160 {
                        rv = self.data.own_id.to_owned();
                    } else {
                        self.depth = usize::MAX;
                        let mut rv = self.data.own_id.to_owned();
                        rv.randomize_after_bit(self.depth);
                        return Some(rv);
                    }
                }
            }

            rv.randomize_after_bit(self.depth);

            let bucket = &self.data.get_bucket(&rv);

            if bucket.iter_over_goods().count() < K_NODE_PER_BUCKET {
                break;
            }

            self.depth += 1;
            rv = self.data.own_id.to_owned();
        }

        Some(rv)
    }
}

#[cfg(test)]
mod tests {
    use super::{RoutingTable, TreeNode, K_NODE_PER_BUCKET};
    use crate::{
        data_structures::ID,
        dht::krpc_message::Nodes,
        transcoding::{socketaddr_from_compact_bytes, COMPACT_SOCKADDR_LEN},
    };
    use std::net::SocketAddrV4;
    use tracing::debug;
    use tracing_test::traced_test;

    fn random_node() -> (ID, SocketAddrV4) {
        let id = ID::new(rand::random());

        let raw_addr: [u8; COMPACT_SOCKADDR_LEN] = rand::random();
        let addr = socketaddr_from_compact_bytes(&raw_addr).unwrap();

        (id, addr)
    }

    fn check_node_counts_for_leaf_at_idx(
        rt: &RoutingTable,
        idx: usize,
        g: usize,
        b: usize,
        e: usize,
    ) {
        let tree_node = &rt.data[idx];

        let TreeNode::Leaf(bucket) = tree_node else {
            panic!("expected Leaf, fount Parent");
        };

        assert_eq!(bucket.iter_over_goods().count(), g);
        assert_eq!(bucket._count_bads(), b);
        assert_eq!(bucket._count_empty(), e);
    }

    #[traced_test]
    #[test]
    fn basic_setup() {
        let mut rt = RoutingTable::default();

        let (id, addr) = random_node();

        rt.insert_good(id.to_owned(), addr);

        assert!(rt.contains_node(&id))
    }

    #[tokio::test]
    async fn cache() {
        let mut default_rt = RoutingTable::default();

        for _ in 0..128 {
            let (id, addr) = random_node();
            default_rt.touch_node(id, addr);
        }

        default_rt.store().await;

        let loaded_rt = RoutingTable::new().await;

        for (default, loaded) in default_rt.data.iter().zip(loaded_rt.data.iter()) {
            assert_eq!(default, loaded);
        }
    }

    #[test]
    fn no_duplicates() {
        let mut rt = RoutingTable::default();

        let (id, addr) = random_node();

        rt.insert_good(id.to_owned(), addr.to_owned());
        rt.insert_good(id.to_owned(), addr.to_owned());
        rt.insert_good(id, addr.to_owned());

        check_node_counts_for_leaf_at_idx(&rt, 0, 1, 0, 7);
    }

    #[test]
    fn split() {
        let mut rt = RoutingTable::default();
        let (mut id, addr) = random_node();

        for i in 0..K_NODE_PER_BUCKET {
            rt.insert_good(id.to_owned(), addr.to_owned());
            id.flip_bit(0);
            id.flip_bit(i + 1);
        }

        rt.insert_good(id.to_owned(), addr.to_owned());

        let TreeNode::Parent(left_idx, right_idx) = &rt.data[0] else {
            panic!("expected Parent, fount Leaf");
        };

        let (large_bucket_idx, small_bucket_idx) =
            id.left_or_right_by_depth(0, (*left_idx, *right_idx), (*right_idx, *left_idx));

        check_node_counts_for_leaf_at_idx(&rt, large_bucket_idx, 5, 0, 3);
        check_node_counts_for_leaf_at_idx(&rt, small_bucket_idx, 4, 0, 4);
    }

    #[test]
    fn when_first_split_is_not_enough() {
        let mut rt = RoutingTable::default();
        let (mut id, addr) = random_node();

        for i in 0..K_NODE_PER_BUCKET {
            rt.insert_good(id.to_owned(), addr.to_owned());
            id.flip_bit(1);
            id.flip_bit(i + 2);
        }

        rt.insert_good(id.to_owned(), addr.to_owned());

        let TreeNode::Parent(left_idx, right_idx) = &rt.data[0] else {
            panic!("expected Parent, fount Leaf");
        };

        let (new_parent_idx, empty_bucket_idx) =
            id.left_or_right_by_depth(0, (*left_idx, *right_idx), (*right_idx, *left_idx));

        check_node_counts_for_leaf_at_idx(&rt, empty_bucket_idx, 0, 0, 8);

        let TreeNode::Parent(left_idx, right_idx) = &rt.data[new_parent_idx] else {
            panic!("expected Parent, fount Leaf");
        };

        let (large_bucket_idx, small_bucket_idx) =
            id.left_or_right_by_depth(1, (*left_idx, *right_idx), (*right_idx, *left_idx));

        check_node_counts_for_leaf_at_idx(&rt, large_bucket_idx, 5, 0, 3);
        check_node_counts_for_leaf_at_idx(&rt, small_bucket_idx, 4, 0, 4);

        assert_eq!(rt.find_bucket_idx_by_id(&id), large_bucket_idx);
    }

    #[test]
    fn find_exact_node() {
        let mut rt = RoutingTable::default();
        let (mut id, addr) = random_node();

        let og_id = id.to_owned();

        for i in 0..20 {
            rt.insert_good(id.to_owned(), addr.to_owned());
            id.flip_bit(i % 3);
            id.flip_bit(i + 3);
        }

        rt.insert_good(og_id.to_owned(), addr.to_owned());

        let Some(nodes) = rt.find_node(&og_id) else {
            panic!("expected Some(Nodes), fount None");
        };

        let Nodes::Exact(exact) = nodes else {
            panic!("expected Nodes::Exact, fount Nodes::Closest");
        };

        assert_eq!(exact.id, og_id);
    }

    #[test]
    fn find_closest_nodes() {
        let mut rt = RoutingTable::default();
        let (target_id, _) = random_node();

        for _ in 0..260 {
            let (id, addr) = random_node();
            rt.insert_good(id, addr);
        }

        let Some(nodes) = rt.find_node(&target_id) else {
            panic!("expected Some(Nodes), fount None");
        };

        let Nodes::Closest(closest) = nodes else {
            panic!("expected Nodes::Closest, fount Nodes::Exact");
        };

        assert!(&target_id - &closest[0].id < &target_id - &closest[1].id);
    }

    macro_rules! zero_id_with_first {
        ($first:expr) => {
            ID::new([
                $first, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ])
        };
    }

    #[traced_test]
    #[test]
    fn iterator() {
        let mut rt = RoutingTable::default();
        rt.own_id = zero_id_with_first!(0b1111_1111);

        let (_, addr) = random_node();

        let left_ids = [
            zero_id_with_first!(0b0000_0000),
            zero_id_with_first!(0b0001_0000),
            zero_id_with_first!(0b0010_0000),
            zero_id_with_first!(0b0011_0000),
            zero_id_with_first!(0b0100_0000),
            zero_id_with_first!(0b0101_0000),
            zero_id_with_first!(0b0110_0000),
        ];

        for id in left_ids {
            rt.insert_good(id.to_owned(), addr.to_owned());
        }

        let right_ids = [
            zero_id_with_first!(0b1000_0000),
            zero_id_with_first!(0b1001_0000),
            zero_id_with_first!(0b1010_0000),
            zero_id_with_first!(0b1011_0000),
            zero_id_with_first!(0b1100_0000),
            zero_id_with_first!(0b1101_0000),
            zero_id_with_first!(0b1110_0000),
        ];

        for id in right_ids {
            rt.insert_good(id.to_owned(), addr.to_owned());
        }

        {
            assert_eq!(2, rt.iter_over_ids_within_fillable_buckets().count());
            let mut iter = rt.iter_over_ids_within_fillable_buckets();
            let left = iter.next().unwrap();
            let right = iter.next().unwrap();
            assert!(!left.get_bit(0));
            assert!(right.get_bit(0));
        }

        rt.insert_good(zero_id_with_first!(0b0111_0000), addr.to_owned());

        {
            assert_eq!(1, rt.iter_over_ids_within_fillable_buckets().count());
            let mut iter = rt.iter_over_ids_within_fillable_buckets();
            let right = iter.next().unwrap();
            assert!(right.get_bit(0));
        }

        rt.insert_good(zero_id_with_first!(0b1111_0000), addr.to_owned());

        {
            assert_eq!(1, rt.iter_over_ids_within_fillable_buckets().count());
            let mut iter = rt.iter_over_ids_within_fillable_buckets();
            let right = iter.next().unwrap();
            assert!(right.get_bit(0));
        }

        rt.insert_good(zero_id_with_first!(0b1000_0001), addr.to_owned());

        {
            debug!(?rt);
            assert_eq!(2, rt.iter_over_ids_within_fillable_buckets().count());
            let mut iter = rt.iter_over_ids_within_fillable_buckets();

            let right_left = iter.next().unwrap();
            assert!(right_left.get_bit(0));
            assert!(!right_left.get_bit(1));

            let right_right = iter.next().unwrap();
            assert!(right_right.get_bit(0));
            assert!(right_right.get_bit(1));
        }
    }
}
