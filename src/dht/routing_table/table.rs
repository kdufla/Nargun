use super::bucket::Bucket;
use super::{Node, K_NODE_PER_BUCKET};
use crate::data_structures::{ID, ID_BIT_COUNT};
use crate::dht::krpc_message::Nodes;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::net::SocketAddrV4;
use tokio::fs::{DirBuilder, File};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{debug, error};

const APP_DIR_NAME: &str = ".nargun";
#[cfg(not(test))]
const CACHED_DHT_FILE: &str = "dht";
#[cfg(test)]
const CACHED_DHT_FILE: &str = "dht.test";

#[derive(Debug, Deserialize, Serialize)]
pub struct RoutingTable {
    own_id: ID,
    data: Vec<Bucket>,
}

impl RoutingTable {
    pub async fn new() -> Self {
        match Self::try_loading_from_disk().await {
            Ok(cache) => {
                debug!(?cache);
                cache
            }
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

    pub fn iter_nodes(&self) -> impl Iterator<Item = &Node> {
        self.data.iter().flat_map(|bucket| bucket.iter_nodes())
    }

    pub fn find_node(&self, id: &ID) -> Option<Nodes> {
        let bucket = self.get_bucket(id);

        if let Some(exact_node) = bucket.get_node(id) {
            return Some(Nodes::Exact(exact_node.clone()));
        }

        self.get_closest_nodes(id)
    }

    pub fn iter_over_ids_within_fillable_buckets(&self) -> impl Iterator<Item = ID> + '_ {
        RandIdInFillableBucketIter {
            iter: Box::new(self.data[..self.data.len() - 1].iter()),
            last: self.data.last(),
            own_id: self.own_id,
        }
    }

    pub fn touch_node(&mut self, id: ID, addr: SocketAddrV4) {
        let bucket = self.get_bucket_mut(&id);

        if let Some(node) = bucket.get_node_mut(&id) {
            node.touch();
        } else {
            self.insert_new_good(id, addr);
        }
    }

    pub fn contains_node(&self, id: &ID) -> bool {
        self.get_bucket(id).contains(id)
    }

    fn get_closest_nodes(&self, id: &ID) -> Option<Nodes> {
        let mut nodes = Vec::new();

        let node_bucket_idx = self.find_bucket_idx_by_id(id);

        for bucket in self.data[node_bucket_idx..]
            .iter()
            .chain(self.data[..node_bucket_idx].iter().rev())
        {
            nodes.extend(bucket.iter_over_goods().cloned());

            if nodes.len() >= K_NODE_PER_BUCKET {
                break;
            }
        }

        if !nodes.is_empty() {
            nodes.sort_by(|a, b| a.cmp_by_distance_to_id(b, id));
            debug!("found closest for {:?}, closest: {:?}", id, nodes);
            Some(Nodes::Closest(nodes))
        } else {
            debug!("no closest nodes for {:?}", id);
            None
        }
    }

    fn get_bucket(&self, id: &ID) -> &Bucket {
        let bucket_idx = self.find_bucket_idx_by_id(id);

        &self.data[bucket_idx]
    }

    fn get_bucket_mut(&mut self, id: &ID) -> &mut Bucket {
        let bucket_idx = self.find_bucket_idx_by_id(id);

        &mut self.data[bucket_idx]
    }

    fn find_bucket_idx_by_id(&self, id: &ID) -> usize {
        let Some(first_diff_idx) = self.own_id.first_diff_bit_idx(id) else {
            // match means it's in own_id bucket
            return self.data.len() -1;
        };

        // diff occurs deeper than current table coverage
        if first_diff_idx >= self.data.len() {
            return self.data.len() - 1;
        }

        first_diff_idx
    }

    fn insert_new_good(&mut self, id: ID, addr: SocketAddrV4) {
        if self.try_insert(&id, &addr).is_ok() {
            return;
        }

        if !self.id_within_splittable_bucket(&id) {
            return;
        }

        if self.max_depth_reached() {
            return;
        }

        self.split();

        self.insert_new_good(id, addr);
    }

    fn max_depth_reached(&self) -> bool {
        self.data.len() == 161 // TODO const
    }

    fn try_insert(&mut self, id: &ID, addr: &SocketAddrV4) -> Result<()> {
        self.get_bucket_mut(id).try_insert(id, addr)
    }

    fn id_within_splittable_bucket(&self, id: &ID) -> bool {
        self.data
            .last()
            .unwrap()
            .id_within_bucket_range(&self.own_id, id)
    }

    fn split(&mut self) {
        let splittable_bucket = self.data.pop().unwrap();
        let parent_depth = splittable_bucket.depth();
        let (zero_at_depth, one_at_depth) = splittable_bucket.split();

        let (own_id_bucket, other_bucket) =
            self.own_id
                .bit_based_order(parent_depth, one_at_depth, zero_at_depth);

        self.data.push(other_bucket);
        self.data.push(own_id_bucket);
    }
}

// impl Iterator for RoutingTable {
//     type Item = &Bucket;

//     fn next(&mut self) -> Option<Self::Item> {
//         todo!()
//     }
// }

impl Default for RoutingTable {
    fn default() -> Self {
        let own_id = ID::new(rand::random());
        let mut data = Vec::with_capacity(ID_BIT_COUNT + 1);
        data.push(Bucket::new(0));

        Self { own_id, data }
    }
}

struct RandIdInFillableBucketIter<'a> {
    own_id: ID,
    last: Option<&'a Bucket>,
    iter: Box<dyn Iterator<Item = &'a Bucket> + 'a>,
}

impl<'a> Iterator for RandIdInFillableBucketIter<'a> {
    type Item = ID;

    fn next(&mut self) -> Option<Self::Item> {
        for bucket in self.iter.by_ref() {
            if bucket.is_full() {
                continue;
            }

            let mut own_id = self.own_id;
            own_id.flip_bit(bucket.depth() - 1);
            own_id.randomize_after_bit(bucket.depth() - 1);

            return Some(own_id);
        }

        let last = std::mem::replace(&mut self.last, None)?;

        if last.depth() == ID_BIT_COUNT && last.is_full() {
            None
        } else {
            Some(self.own_id)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{RoutingTable, K_NODE_PER_BUCKET};
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

    fn check_node_counts_at_idx(
        rt: &RoutingTable,
        idx: usize,
        good: usize,
        bad: usize,
        empty: usize,
    ) {
        let bucket = &rt.data[idx];

        assert_eq!(bucket.iter_over_goods().count(), good);
        assert_eq!(bucket._count_bads(), bad);
        assert_eq!(bucket._count_empty(), empty);
    }

    #[test]
    fn basic_setup() {
        let mut rt = RoutingTable::default();

        let (id, addr) = random_node();

        rt.insert_new_good(id, addr);

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

        rt.insert_new_good(id, addr);
        rt.insert_new_good(id, addr);
        rt.insert_new_good(id, addr);

        check_node_counts_at_idx(&rt, 0, 1, 0, 7);
    }

    #[test]
    fn split() {
        let mut rt = RoutingTable::default();
        let (_, addr) = random_node();
        let mut id = rt.own_id;

        for i in 0..K_NODE_PER_BUCKET {
            rt.insert_new_good(id, addr);
            id.flip_bit(0);
            id.flip_bit(i + 1);
        }

        rt.insert_new_good(id, addr);

        check_node_counts_at_idx(&rt, 0, 4, 0, 4);
        check_node_counts_at_idx(&rt, 1, 5, 0, 3);
    }

    #[test]
    fn multi_split_insert() {
        let mut rt = RoutingTable::default();
        let (_, addr) = random_node();
        let mut id = rt.own_id;

        // everyone has same first bit
        for i in 0..K_NODE_PER_BUCKET {
            rt.insert_new_good(id, addr);
            id.flip_bit(1);
            id.flip_bit(i + 2);
        }

        rt.insert_new_good(id, addr);

        debug!(?rt);

        check_node_counts_at_idx(&rt, 0, 0, 0, 8);
        check_node_counts_at_idx(&rt, 1, 4, 0, 4);
        check_node_counts_at_idx(&rt, 2, 5, 0, 3);

        assert_eq!(rt.find_bucket_idx_by_id(&id), 2);
    }

    #[test]
    fn find_exact_node() {
        let mut rt = RoutingTable::default();
        let (mut id, addr) = random_node();

        let og_id = id;

        for i in 0..20 {
            rt.insert_new_good(id, addr);
            id.flip_bit(i % 3);
            id.flip_bit(i + 3);
        }

        rt.insert_new_good(og_id, addr);

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
            rt.insert_new_good(id, addr);
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
        let mut rt = RoutingTable {
            own_id: zero_id_with_first!(0b1111_1111),
            ..Default::default()
        };

        let (_, addr) = random_node();

        let zeros = [
            zero_id_with_first!(0b0000_0000),
            zero_id_with_first!(0b0001_0000),
            zero_id_with_first!(0b0010_0000),
            zero_id_with_first!(0b0011_0000),
            zero_id_with_first!(0b0100_0000),
            zero_id_with_first!(0b0101_0000),
            zero_id_with_first!(0b0110_0000),
        ];

        for id in zeros {
            rt.insert_new_good(id, addr);
        }

        let ones = [
            zero_id_with_first!(0b1000_0000),
            zero_id_with_first!(0b1001_0000),
            zero_id_with_first!(0b1010_0000),
            zero_id_with_first!(0b1011_0000),
            zero_id_with_first!(0b1100_0000),
            zero_id_with_first!(0b1101_0000),
            zero_id_with_first!(0b1110_0000),
        ];

        for id in ones {
            rt.insert_new_good(id, addr);
        }

        for id in rt.iter_over_ids_within_fillable_buckets() {
            debug!(?id);
        }

        {
            assert_eq!(2, rt.iter_over_ids_within_fillable_buckets().count());

            let mut iter = rt.iter_over_ids_within_fillable_buckets();

            let starts_with_zero = iter.next().unwrap();
            assert!(!starts_with_zero.get_bit(0));

            let starts_with_one = iter.next().unwrap();
            assert!(starts_with_one.get_bit(0));
        }

        {
            rt.insert_new_good(zero_id_with_first!(0b0111_0000), addr);
            assert_eq!(1, rt.iter_over_ids_within_fillable_buckets().count());
            let mut iter = rt.iter_over_ids_within_fillable_buckets();
            let starts_with_one = iter.next().unwrap();
            assert!(starts_with_one.get_bit(0));
        }

        {
            // this fills own_id bucket but its still splittable
            rt.insert_new_good(zero_id_with_first!(0b1111_0000), addr);
            assert_eq!(1, rt.iter_over_ids_within_fillable_buckets().count());
            let mut iter = rt.iter_over_ids_within_fillable_buckets();
            let starts_with_one = iter.next().unwrap();
            assert!(starts_with_one.get_bit(0));
        }

        {
            // after split
            rt.insert_new_good(zero_id_with_first!(0b1100_0001), addr);
            assert_eq!(2, rt.iter_over_ids_within_fillable_buckets().count());

            let mut iter = rt.iter_over_ids_within_fillable_buckets();

            let starts_with_10 = iter.next().unwrap();
            assert!(starts_with_10.get_bit(0));
            assert!(!starts_with_10.get_bit(1));

            let starts_with_11 = iter.next().unwrap();
            assert!(starts_with_11.get_bit(0));
            assert!(starts_with_11.get_bit(1));
        }
    }
}
