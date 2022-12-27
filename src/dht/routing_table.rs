use super::krpc_message::Nodes;
use crate::constants::{ID_LEN, T26IX};
use crate::util::functions::socketaddr_from_compact_bytes;
use crate::util::id::ID;
use anyhow::{anyhow, bail, Result};
use std::cmp::Ordering;
use std::net::SocketAddrV4;
use std::time::Duration;
use tokio::time::Instant;

pub struct RoutingTable {
    own_id: ID,
    data: Vec<TreeNode>,
}

enum TreeNode {
    // when a bucket is split, 1 is stored as a right child and vice versa
    Parent(usize, usize),
    Leaf(Bucket),
}

#[derive(Debug, Clone)]
struct Bucket {
    depth: usize,
    data: [Option<Node>; 8],
}

#[derive(Debug, Eq, Clone)]
pub struct Node {
    pub id: ID,
    pub addr: SocketAddrV4,
    able_to_receive: bool,
    last_seen: Option<Instant>,
}

impl RoutingTable {
    pub fn new(own_id: ID) -> Self {
        let mut data = Vec::with_capacity(320);

        data.push(TreeNode::Leaf(Bucket::new(0)));

        Self { own_id, data }
    }

    pub fn find_node(&self, id: &ID) -> Option<Nodes> {
        let bucket = self.get_bucket(id);

        if let Some(exact_node) = bucket.get_node(id) {
            return Some(Nodes::Exact(exact_node.clone()));
        }

        self.get_closest_nodes(id, bucket)
    }

    fn get_closest_nodes<'a>(&'a self, id: &ID, mut bucket: &'a Bucket) -> Option<Nodes> {
        let mut closest = Vec::with_capacity(16);
        let mut prev_depth = usize::MAX;
        let mut sibling_id = id.to_owned();

        while closest.len() < 8 && prev_depth != bucket.depth {
            prev_depth = bucket.depth;

            closest.extend(bucket.iter_over_goods().cloned());

            // next closest nodes are descendants of sibling TreeNode
            sibling_id.flip_bit(bucket.depth);
            bucket = self.get_bucket(&sibling_id);
        }

        if closest.len() >= 1 {
            closest.sort_by(|a, b| a.cmp_by_distance_to_id(b, id));

            Some(Nodes::Closest(closest))
        } else {
            None
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

    fn contains_node(&self, id: &ID) -> bool {
        self.get_bucket(id).contains(id)
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

        let own_id = self.own_id.to_owned();
        let bucket = self.get_bucket_mut(&id);

        if bucket.try_insert(&id, &addr).is_ok() {
            return;
        }

        let own_id_within_the_bucket = own_id
            .cmp_first_n_bits(&bucket.data[0].as_ref().unwrap().id, bucket.depth)
            .is_eq();

        if !own_id_within_the_bucket {
            return;
        }

        self.split(id, addr, own_id, None);
    }

    fn split(&mut self, id: ID, addr: SocketAddrV4, own_id: ID, bucket_idx: Option<usize>) {
        let bucket_idx = match bucket_idx {
            Some(idx) => idx,
            None => self.find_bucket_idx_by_id(&id),
        };

        let bucket = self.get_bucket_at(bucket_idx).unwrap();
        let og_bucket_depth = bucket.depth;
        let (left, right) = bucket.split();

        let (left_idx, right_idx) = self.push_children(bucket_idx, left, right);
        let cur_idx = id.left_or_right_by_depth(og_bucket_depth, left_idx, right_idx);

        let bucket = self.get_bucket_at_mut(cur_idx).unwrap();
        if bucket.try_insert(&id, &addr).is_ok() || bucket.depth == 160 {
            return;
        }

        self.split(id, addr, own_id, Some(cur_idx))
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
            data: [(); 8].map(|_| Option::<Node>::default()),
        }
    }

    fn split(&self) -> (Self, Self) {
        let mut right = Bucket::new(self.depth + 1);
        let mut left = Bucket::new(self.depth + 1);

        let mut cur_left_idx = 0;
        let mut cur_right_idx = 0;

        for node in &self.data {
            if let Some(node) = node {
                if node.id.get_bit(self.depth) {
                    right.data[cur_right_idx] = Some(node.to_owned());
                    cur_right_idx += 1;
                } else {
                    left.data[cur_left_idx] = Some(node.to_owned());
                    cur_left_idx += 1;
                }
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
            .find(|node| Node::is_some_with_id(node, &id).is_some())?
            .as_ref()
    }

    fn get_node_mut(&mut self, id: &ID) -> Option<&mut Node> {
        self.data
            .iter_mut()
            .find(|node| Node::is_some_with_id(node, &id).is_some())?
            .as_mut()
    }

    fn try_insert(&mut self, id: &ID, addr: &SocketAddrV4) -> Result<()> {
        let empty_elem = self.data.iter_mut().find(|x| x.is_none());

        let replaceable_elem = if empty_elem.is_none() {
            self.data
                .iter_mut()
                .find(|x| Node::is_some_and_good(x).is_none())
        } else {
            empty_elem
        };

        let Some(elem_to_replace)= replaceable_elem else {
            return Err(anyhow!("bucket is full"));
        };

        *elem_to_replace = Some(Node {
            id: id.to_owned(),
            addr: addr.to_owned(),
            able_to_receive: true,
            last_seen: Some(Instant::now()),
        });

        Ok(())
    }

    fn iter_over_goods(&self) -> impl Iterator<Item = &Node> {
        self.data.iter().filter_map(Node::is_some_and_good)
    }

    fn _count_bads(&self) -> usize {
        let exists_and_is_bad = |node_opt: &Option<Node>| {
            let node = node_opt.as_ref()?;
            let last_seen_instant = node.last_seen.as_ref()?;
            let status_expired = last_seen_instant.elapsed() > Duration::from_secs(15 * 60);
            (!node.able_to_receive || status_expired).then_some(())
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
    pub fn from_compact_bytes(buff: &[u8]) -> Result<Self> {
        if buff.len() == T26IX {
            let (id, addr) = buff.split_at(ID_LEN);

            Ok(Self {
                id: ID(id.try_into()?),
                addr: socketaddr_from_compact_bytes(addr)?,
                able_to_receive: false,
                last_seen: None,
            })
        } else {
            bail!(
                "Node::from_compact buff size is {}, expected {}",
                buff.len(),
                T26IX
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

        if node.able_to_receive && (last_seen_instant.elapsed() < Duration::from_secs(15 * 60)) {
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

#[cfg(test)]
mod routing_table_tests {
    use std::net::SocketAddrV4;

    use crate::{
        constants::SIX,
        dht::krpc_message::Nodes,
        util::{functions::socketaddr_from_compact_bytes, id::ID},
    };

    use super::{RoutingTable, TreeNode};

    fn random_node() -> (ID, SocketAddrV4) {
        let id = ID(rand::random());

        let raw_addr: [u8; SIX] = rand::random();
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

    #[test]
    fn basic_setup() {
        let mut rt = RoutingTable::new(ID(rand::random()));

        let (id, addr) = random_node();

        rt.insert_good(id.to_owned(), addr);

        assert!(rt.contains_node(&id))
    }

    #[test]
    fn no_duplicates() {
        let mut rt = RoutingTable::new(ID(rand::random()));

        let (id, addr) = random_node();

        rt.insert_good(id.to_owned(), addr.to_owned());
        rt.insert_good(id.to_owned(), addr.to_owned());
        rt.insert_good(id.to_owned(), addr.to_owned());

        check_node_counts_for_leaf_at_idx(&rt, 0, 1, 0, 7);
    }

    #[test]
    fn split() {
        let mut rt = RoutingTable::new(ID(rand::random()));
        let (mut id, addr) = random_node();

        for i in 0..8 {
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
        let mut rt = RoutingTable::new(ID(rand::random()));
        let (mut id, addr) = random_node();

        for i in 0..8 {
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
        let mut rt = RoutingTable::new(ID(rand::random()));
        let (mut id, addr) = random_node();

        let og_id = id.to_owned();

        for i in 0..20 {
            rt.insert_good(id.to_owned(), addr.to_owned());
            id.flip_bit(i % 3);
            id.flip_bit(i + 3);
        }

        rt.insert_good(id.to_owned(), addr.to_owned());

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
        let mut rt = RoutingTable::new(ID(rand::random()));
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
}
