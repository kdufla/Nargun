// Note: term 'node' is ambiguous because nodes (dht users stored in routing table) are stored in nodes (of a tree data-structure)
// this code uses remote-node to describe former and tree-node for latter

use std::{net::SocketAddr, time::Duration};

use crate::util::id::ID;
use anyhow::{anyhow, Result};
use std::mem;
use tokio::time::Instant;

pub struct RoutingTable {
    data: Vec<TreeNode>,
}

enum TreeNode {
    Parent(usize, usize),
    Leaf(Bucket),
}

struct Bucket {
    min_id: ID,
    max_id: ID,
    data: [Option<RemoteNode>; 8],
}

struct RemoteNode {
    id: ID,
    addr: SocketAddr,
    last_seen: Option<Instant>,
}

impl RoutingTable {
    pub fn new() -> Self {
        Self {
            data: Vec::with_capacity(320),
        }
    }

    fn find_bucket_by_id(&self, id: &ID) -> Result<usize> {
        let mut data_iter_idx = 0;

        for depth in 0..160 {
            match &self.data[data_iter_idx] {
                TreeNode::Parent(left_child_idx, right_child_idx) => {
                    data_iter_idx = if id.get_bit(depth) {
                        *right_child_idx
                    } else {
                        *left_child_idx
                    }
                }
                TreeNode::Leaf(bucket) => {
                    return if bucket.id_within_range(id) {
                        Ok(data_iter_idx)
                    } else {
                        Err(anyhow!("find_bucket_by_id: found a bucket (at idx={}) but id is not in this range. this should be impossible.", data_iter_idx))
                    }
                }
            }
        }

        Err(anyhow!("find_bucket_by_id: could not find it"))
    }

    pub fn insert(&mut self, remote_node_id: ID) {
        let bucket_idx = self.find_bucket_by_id(&remote_node_id).unwrap(); // TODO error handling, you don't have to change this, just come up with a reason to justify.
    }
}

impl Bucket {
    fn id_within_range(&self, id: &ID) -> bool {
        self.min_id <= *id && self.max_id >= *id
    }

    fn all_good(&self) -> bool {
        self.data
            .iter()
            .all(|remote_node_opt| match remote_node_opt {
                Some(remote_node) => match remote_node.last_seen {
                    Some(last_seen_instant) => {
                        if last_seen_instant.elapsed() < Duration::from_secs(15 * 60) {
                            true
                        } else {
                            false
                        }
                    }
                    None => false,
                },
                None => false,
            })
    }
}

impl RemoteNode {
    async fn ping(&self) {}
}
