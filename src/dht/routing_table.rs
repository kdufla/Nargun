use super::krpc_message::Nodes;
use crate::constants::{ID_LEN, T26IX};
use crate::util::functions::socketaddr_from_compact_bytes;
use crate::util::id::ID;
use anyhow::{anyhow, bail, Result};
use std::net::SocketAddrV4;
use std::time::Duration;
use tokio::time::Instant;

pub struct RoutingTable {
    own_id: ID,
    data: Vec<TreeNode>,
}

enum TreeNode {
    Parent(usize, usize),
    Leaf(Bucket),
}

#[derive(Debug, Clone)]
struct Bucket {
    depth: usize,
    data: [Option<RemoteNode>; 8],
}

#[derive(Debug, Eq, Clone)]
pub struct RemoteNode {
    pub id: ID,
    pub addr: SocketAddrV4,
    able_to_receive: bool,
    last_seen: Option<Instant>,
}

impl RemoteNode {
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
}

impl PartialEq for RemoteNode {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl RoutingTable {
    pub fn new(own_id: ID) -> Self {
        Self {
            own_id,
            data: Vec::with_capacity(320),
        }
    }

    pub fn find_remote_node(&self, id: &ID) -> Option<Nodes> {
        let bucket = self.get_bucket_ref(id);

        if let Some(exact_node) = bucket.get_node(id) {
            return Some(Nodes::Exact(exact_node.clone()));
        }

        self.get_closest_nodes(id, bucket)
    }

    fn get_closest_nodes(&self, id: &ID, bucket: &Bucket) -> Option<Nodes> {
        let mut closest = Vec::with_capacity(8);
        let mut prev_depth = 512;
        let mut bucket = bucket;
        let mut owned_id = id.to_owned();

        loop {
            for n in bucket.iter_over_goods() {
                closest.push(n.clone());
            }

            if closest.len() >= 8 || prev_depth == bucket.depth {
                break;
            }

            prev_depth = bucket.depth;
            owned_id.flip_bit(bucket.depth);
            bucket = self.get_bucket_ref(&owned_id);
        }

        if closest.len() >= 1 {
            closest.sort_by(|a, b| {
                let distance_to_a = &a.id - id;
                let distance_to_b = &b.id - id;

                distance_to_a.cmp(&distance_to_b)
            });

            Some(Nodes::Closest(closest))
        } else {
            None
        }
    }

    fn contains_remote_node(&self, id: &ID) -> bool {
        self.get_bucket_ref(id).contains(id)
    }

    pub fn touch_node(&mut self, id: ID, addr: SocketAddrV4) {
        let bucket = self.get_bucket_mut_ref(&id);

        if let Some(node) = bucket.get_node_mut(&id) {
            node.last_seen = Some(Instant::now())
        } else {
            self.insert_good(id.to_owned(), addr)
        }
    }

    fn get_bucket_mut_ref(&mut self, id: &ID) -> &mut Bucket {
        let bucket_idx = self.find_bucket_idx_by_id(id);

        let TreeNode::Leaf(bucket) = &mut self.data[bucket_idx] else {
            panic!("this should alway be a leaf");
        };

        bucket
    }

    fn get_bucket_ref(&self, id: &ID) -> &Bucket {
        let bucket_idx = self.find_bucket_idx_by_id(id);

        let TreeNode::Leaf(bucket) = &self.data[bucket_idx] else {
            panic!("this should alway be a leaf");
        };

        bucket
    }

    fn find_bucket_idx_by_id(&self, id: &ID) -> usize {
        let mut data_iter_idx = 0;

        for depth in 0..160 {
            match &self.data[data_iter_idx] {
                TreeNode::Parent(left_child_idx, right_child_idx) => {
                    // this assumes, that when a bucket is split, 1 is stored as a right child and vice versa
                    data_iter_idx = if id.get_bit(depth) {
                        *right_child_idx
                    } else {
                        *left_child_idx
                    }
                }
                TreeNode::Leaf(_) => return data_iter_idx,
            }
        }

        panic!("find_bucket_idx_by_id: could not find (impossible)");
    }

    // hey, future me! don't forget to increase len
    pub fn insert_good(&mut self, id: ID, addr: SocketAddrV4) {
        let own_id = self.own_id.to_owned();
        let bucket = self.get_bucket_mut_ref(&id);

        if bucket.iter_over_goods().count() == 8 {
            let any_id_from_bucket = &bucket.data[0].as_ref().unwrap().id;
            let own_id_within_bucket = any_id_from_bucket
                .cmp_first_n_bits(&own_id, bucket.depth)
                .is_eq();
            if own_id_within_bucket {
                self.split_and_insert(id, addr, own_id, None);
            }
        } else {
            let _ = bucket.try_insert(&id, &addr, InsertMode::EmptyAndBad);
        }
    }

    fn split_and_insert(
        &mut self,
        id: ID,
        addr: SocketAddrV4,
        own_id: ID,
        bucket_idx: Option<usize>,
    ) {
        let bucket_idx = match bucket_idx {
            Some(idx) => idx,
            None => self.find_bucket_idx_by_id(&id),
        };

        let bucket = self.bucket_at(bucket_idx).unwrap();
        let new_depth = bucket.depth + 1;

        let (mut left, mut right) = Self::distribute_data_in_new_buckets(&bucket.data, new_depth);

        let (own_bucket, own_idx) = if own_id.get_bit(new_depth) {
            (&mut right, self.data.len() + 1)
        } else {
            (&mut left, self.data.len())
        };

        let split_freed_up_space = own_bucket
            .try_insert(&id, &addr, InsertMode::EmptyAndBad)
            .is_ok();

        // don't change this order. variable own_idx assumes [... , left, right]
        self.data.push(TreeNode::Leaf(left));
        self.data.push(TreeNode::Leaf(right));

        self.data[bucket_idx] = TreeNode::Parent(self.data.len() - 2, self.data.len() - 1);

        if !split_freed_up_space {
            self.split_and_insert(id, addr, own_id, Some(own_idx))
        }
    }

    fn bucket_at(&self, i: usize) -> Option<&Bucket> {
        match &self.data[i] {
            TreeNode::Parent(_, _) => None,
            TreeNode::Leaf(bucket) => Some(bucket),
        }
    }

    fn distribute_data_in_new_buckets(
        data: &[Option<RemoteNode>; 8],
        depth: usize,
    ) -> (Bucket, Bucket) {
        let mut right = Bucket {
            depth: depth,
            data: [(); 8].map(|_| Option::<RemoteNode>::default()),
        };
        let mut left = right.clone();

        let mut cur_left_idx = 0;
        let mut cur_right_idx = 0;

        for node in data {
            if let Some(node) = node {
                if node.id.get_bit(depth) {
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
}

enum InsertMode {
    Empty,
    EmptyAndBad,
}

impl Bucket {
    fn sort_by_distance(&mut self, id: &ID) {
        let cmp = |a: &Option<RemoteNode>, b: &Option<RemoteNode>| {
            let dist_a = match a {
                Some(a) => &a.id - id,
                None => ID([0; ID_LEN]),
            };

            let dist_b = match b {
                Some(b) => &b.id - id,
                None => ID([0xFF; ID_LEN]),
            };

            dist_a.cmp(&dist_b)
        };
        self.data.sort_by(cmp);
    }

    fn node_exists_and_has_id(remote_node: &Option<RemoteNode>, id: &ID) -> Option<()> {
        remote_node.as_ref()?.eq_by_id(id).then_some(())
    }

    fn get_node_mut(&mut self, id: &ID) -> Option<&mut RemoteNode> {
        self.data
            .iter_mut()
            .find(|remote_node| Self::node_exists_and_has_id(remote_node, &id).is_some())?
            .as_mut()
    }

    fn get_node(&self, id: &ID) -> Option<&RemoteNode> {
        let result = self
            .data
            .iter()
            .find(|remote_node| Self::node_exists_and_has_id(remote_node, &id).is_some())?;

        result.as_ref()
    }

    fn contains(&self, id: &ID) -> bool {
        self.get_node(id).is_some()
    }

    fn count_empty(&self) -> usize {
        self.data
            .iter()
            .fold(0, |acc, x| if x.is_none() { acc + 1 } else { acc })
    }

    fn try_insert(&mut self, id: &ID, addr: &SocketAddrV4, insert_mode: InsertMode) -> Result<()> {
        // TODO consider simplifying is you don't need a bad-less insert mode
        let search_criteria = match insert_mode {
            InsertMode::Empty => |x: &&mut Option<RemoteNode>| x.is_none(),
            InsertMode::EmptyAndBad => {
                |x: &&mut Option<RemoteNode>| Self::node_is_some_and_good(x).is_none()
            }
        };

        let Some(empty_elem)= self.data.iter_mut().find(search_criteria) else {
            return Err(anyhow!("bucket is full"));
        };

        *empty_elem = Some(RemoteNode {
            id: id.to_owned(),
            addr: addr.to_owned(),
            able_to_receive: true,
            last_seen: Some(Instant::now()),
        });

        Ok(())
    }

    fn node_is_some_and_good(remote_node_opt: &Option<RemoteNode>) -> Option<&RemoteNode> {
        let remote_node = remote_node_opt.as_ref()?;
        let last_seen_instant = remote_node.last_seen.as_ref()?;

        if remote_node.able_to_receive
            && (last_seen_instant.elapsed() < Duration::from_secs(15 * 60))
        {
            Some(remote_node)
        } else {
            None
        }
    }

    fn iter_over_goods(&self) -> impl Iterator<Item = &RemoteNode> {
        self.data.iter().filter_map(Self::node_is_some_and_good)
    }

    fn mut_iter_over_bads(&mut self) -> impl Iterator<Item = &mut Option<RemoteNode>> {
        let exists_and_is_bad = |remote_node_opt: &&mut Option<RemoteNode>| {
            let remote_node = remote_node_opt.as_ref()?;
            let last_seen_instant = remote_node.last_seen.as_ref()?;
            let status_expired = last_seen_instant.elapsed() > Duration::from_secs(15 * 60);
            (!remote_node.able_to_receive || status_expired).then_some(())
        };

        self.data
            .iter_mut()
            .filter(move |remote_node_opt| exists_and_is_bad(remote_node_opt).is_some())
    }
}

impl RemoteNode {
    fn eq_by_id(&self, id: &ID) -> bool {
        self.id == *id
    }
}
