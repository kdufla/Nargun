use super::{
    connection::{
        error::Error as ConError, ConMsg, Connection, Query, QueryId, QueryingConnection, Resp, MTU,
    },
    krpc_message::{Nodes, ValuesOrNodes},
    routing_table::{Node, RoutingTable},
};
use crate::{
    client::{Peer, Peers, COMPACT_PEER_LEN},
    data_structures::{NoSizeBytes, ID},
    shutdown,
};
use anyhow::anyhow;
use rand::{seq::IteratorRandom, thread_rng};
use std::{
    cmp::Ordering,
    collections::{hash_map, HashMap, HashSet},
    net::SocketAddrV4,
    time::Duration,
};
use tokio::{select, sync::mpsc, time::sleep};
use tracing::{debug, error, instrument, warn};

type PeerMap = HashMap<ID, HashSet<Peer>>;

const MAX_NODE_FETCH_SLEEP: u64 = 60;
const MAX_SECS_INTERVAL_BETWEEN_PEER_FETCHES: u64 = 120;

// TODO!!! The worst naming yet. like ConMsg is channel message from connection to task and ConCommand the other way.

#[instrument(skip_all)]
pub async fn dht(
    info_hash: ID,
    tcp_port: u16,
    mut routing_table: RoutingTable,
    mut peers: Peers,
    mut peer_with_dht: mpsc::Receiver<SocketAddrV4>,
    mut shutdown_rx: shutdown::Receiver,
    mut connection: Connection,
) -> anyhow::Result<()> {
    let known_peers: HashSet<Peer> = peers.peer_addresses().into_iter().collect();
    let mut peer_map = HashMap::from([(info_hash.to_owned(), known_peers)]);

    for addr in routing_table.iter_nodes().map(|node| node.addr) {
        debug!(?addr);
        connection.send_ping(addr).await?;
    }

    let mut spawn_peer_fetcher_when_at_least_one_node_is_good = true;

    loop {
        debug!("loop");
        select! {
            _ = shutdown_rx.recv() => {
                routing_table.store().await;
                return Ok(());
            }
            addr = peer_with_dht.recv() => {
                let addr = addr.ok_or(anyhow!("peer receiver failed"))?;
                connection.send_ping(addr).await?;
            },
            message = connection.recv() => {
                match message? {
                    ConMsg::Query(query) => process_query(query, &mut routing_table, &connection, &mut peers, &mut peer_map).await?,
                    ConMsg::Resp(resp) => process_resp(resp, &connection, &mut routing_table).await?,
                }
            },
        }

        if spawn_peer_fetcher_when_at_least_one_node_is_good && routing_table.count_good_nodes() > 0
        {
            let Some(nodes) = routing_table.get_closest_nodes(&info_hash) else {
                warn!("I just checked few lines ago... can't say anything useful, you just have to start debugging");
                continue;
            };

            let querying_connection = connection.get_querying_connection();
            let peers_cl = peers.clone();

            tokio::spawn(async move {
                if let Err(e) = periodically_fetch_peers(
                    info_hash,
                    tcp_port,
                    nodes,
                    querying_connection,
                    peers_cl,
                )
                .await
                {
                    error!(?e);
                }
            });

            spawn_peer_fetcher_when_at_least_one_node_is_good = false;
        }
    }
}

async fn process_query(
    query: Query,
    routing_table: &mut RoutingTable,
    connection: &Connection,
    torrent_peers: &mut Peers,
    dht_peer_map: &mut PeerMap,
) -> Result<(), ConError> {
    match query {
        Query::FindNode { target, query_id } => {
            process_find_node_query(connection, routing_table, target, query_id).await?
        }
        Query::GetPeers {
            info_hash,
            query_id,
        } => {
            process_get_peers_query(
                connection,
                routing_table,
                torrent_peers,
                dht_peer_map,
                info_hash,
                query_id,
            )
            .await?
        }
        Query::NewPeer {
            new_peer,
            info_hash,
        } => store_new_peer(new_peer, info_hash, torrent_peers, dht_peer_map),
    }

    Ok(())
}

async fn process_find_node_query(
    connection: &Connection,
    routing_table: &mut RoutingTable,
    target: ID,
    query_id: QueryId,
) -> Result<(), ConError> {
    if let Some(nodes) = routing_table.find_node(&target) {
        connection.resp_to_find_node(nodes, query_id).await?;
    };

    Ok(())
}

async fn process_get_peers_query(
    connection: &Connection,
    routing_table: &mut RoutingTable,
    torrent_peers: &mut Peers,
    dht_peer_map: &mut PeerMap,
    info_hash: ID,
    query_id: QueryId,
) -> Result<(), ConError> {
    let peers = if torrent_peers.serve(&info_hash) {
        torrent_peers.get_good_peers(MTU / COMPACT_PEER_LEN)
    } else {
        let mut rng = thread_rng();
        dht_peer_map.get(&info_hash).map(|peers| {
            peers
                .into_iter()
                .choose_multiple(&mut rng, MTU / COMPACT_PEER_LEN)
                .into_iter()
                .cloned()
                .collect()
        })
    };

    let values_or_nodes = match peers {
        Some(peers) => ValuesOrNodes::Values { values: peers },
        None => {
            let Some(nodes) = routing_table.find_node(&info_hash) else {
                return Ok(()); // TODO this is not Ok but it's not crash worthy. maybe I should use non-option rv? it won't ever be empty anyway.
            };

            ValuesOrNodes::Nodes { nodes }
        }
    };

    connection
        .resp_to_get_peers(values_or_nodes, query_id)
        .await
}

fn store_new_peer(
    new_peer: Peer,
    info_hash: ID,
    torrent_peers: &mut Peers,
    dht_peer_map: &mut PeerMap,
) {
    if torrent_peers.serve(&info_hash) {
        torrent_peers.insert(new_peer);
    } else {
        dht_peer_map
            .entry(info_hash.to_owned())
            .and_modify(|peers| {
                peers.insert(new_peer);
            })
            .or_insert(HashSet::from([new_peer]));
    }
}

async fn process_resp(
    resp: Resp,
    connection: &Connection,
    routing_table: &mut RoutingTable,
) -> Result<(), ConError> {
    match resp {
        Resp::Touch { id, from } => routing_table.touch_node(id, from),
        Resp::NewNodes { nodes, from, id } => {
            routing_table.touch_node(id, from);
            ping_unknown_nodes(nodes, connection, routing_table).await?
        }
        _ => warn!("received response to get_peers message, but it is never sent from here"),
    }

    Ok(())
}

async fn ping_unknown_nodes(
    nodes: Nodes,
    connection: &Connection,
    routing_table: &RoutingTable,
) -> Result<(), ConError> {
    let unknown_node_addresses: Vec<SocketAddrV4> = nodes
        .iter()
        .filter_map(|node| (!routing_table.contains_node(&node.id)).then_some(node.addr))
        .collect();

    for node_addr in unknown_node_addresses {
        connection.send_ping(node_addr).await?;
    }

    Ok(())
}

type NodeMap = HashMap<SocketAddrV4, NodeCloseToInfoHash>;

async fn periodically_fetch_peers(
    info_hash: ID,
    port: u16,
    nodes: Vec<Node>,
    mut connection: QueryingConnection,
    mut peers: Peers,
) -> Result<(), ConError> {
    let mut nodes: NodeMap = nodes
        .into_iter()
        .map(|node| (node.addr, NodeCloseToInfoHash::Unknown(node.id)))
        .collect();

    let mut interval =
        CappedGrowingInterval::new(MAX_SECS_INTERVAL_BETWEEN_PEER_FETCHES, |x: u64| {
            (10.0 + 0.01 * (x as f64).powf(3.7)) as u64
        });

    loop {
        interval.tick().await;
        fetch_peers(&info_hash, &mut nodes, &mut connection, &mut peers).await?;

        let mut x: Vec<(SocketAddrV4, NodeCloseToInfoHash)> = nodes.into_iter().collect();
        x.sort_by(|(_, a), (_, b)| {
            let discriminant_ordering = a.cmp_discriminant(b);
            if discriminant_ordering != Ordering::Equal {
                return discriminant_ordering;
            }

            (a.id() - info_hash).cmp(&(b.id() - info_hash))
        });

        nodes = x.into_iter().take(MAX_ACTIVE_CLOSE_NODES).collect();

        for (addr, node) in nodes
            .iter()
            .filter(|(_, node)| node.is_known())
            .take(ANNOUNCE_TO_N_TOP_NODES)
        {
            if let Some(token) = node.token() {
                connection
                    .send_announce_peer(info_hash, port, token.as_bytes(), *addr)
                    .await?
            }
        }

        // let x = nodes_with_peers
        //     .into_iter()
        //     .map(|(addr, _)| NodeCloseToInfoHash::HasPeers(addr))
        //     .chain(
        //         active_nodes
        //             .into_iter()
        //             .map(|(addr, _)| NodeCloseToInfoHash::Peerless(addr)),
        //     );

        // nodes.extend(x);
    }
}

const ANNOUNCE_TO_N_TOP_NODES: usize = 1 << 4;
const MAX_ACTIVE_CLOSE_NODES: usize = 1 << 7;

#[derive(Debug)]
enum NodeCloseToInfoHash {
    Unknown(ID),
    HasPeers { id: ID, token: NoSizeBytes },
    Peerless { id: ID, token: NoSizeBytes },
}

impl Eq for NodeCloseToInfoHash {}

impl PartialEq for NodeCloseToInfoHash {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Unknown(l_id), Self::Unknown(r_id)) => l_id == r_id,
            (Self::HasPeers { id: l_id, .. }, Self::HasPeers { id: r_id, .. }) => l_id == r_id,
            (Self::Peerless { id: l_id, .. }, Self::Peerless { id: r_id, .. }) => l_id == r_id,
            _ => false,
        }
    }
}

impl NodeCloseToInfoHash {
    fn id(&self) -> ID {
        match self {
            NodeCloseToInfoHash::Unknown(id) => *id,
            NodeCloseToInfoHash::HasPeers { id, .. } => *id,
            NodeCloseToInfoHash::Peerless { id, .. } => *id,
        }
    }

    fn is_known(&self) -> bool {
        match self {
            NodeCloseToInfoHash::Unknown(_) => false,
            NodeCloseToInfoHash::HasPeers { .. } => true,
            NodeCloseToInfoHash::Peerless { .. } => true,
        }
    }

    fn token(&self) -> Option<&NoSizeBytes> {
        match self {
            NodeCloseToInfoHash::Unknown(_) => None,
            NodeCloseToInfoHash::HasPeers { token, .. } => Some(token),
            NodeCloseToInfoHash::Peerless { token, .. } => Some(token),
        }
    }

    fn cmp_discriminant(&self, other: &Self) -> Ordering {
        match (self, other) {
            (NodeCloseToInfoHash::HasPeers { .. }, NodeCloseToInfoHash::Peerless { .. }) => {
                Ordering::Greater
            }
            (NodeCloseToInfoHash::HasPeers { .. }, NodeCloseToInfoHash::Unknown(_)) => {
                Ordering::Greater
            }
            (NodeCloseToInfoHash::Peerless { .. }, NodeCloseToInfoHash::Unknown(_)) => {
                Ordering::Greater
            }
            (NodeCloseToInfoHash::Peerless { .. }, NodeCloseToInfoHash::HasPeers { .. }) => {
                Ordering::Less
            }
            (NodeCloseToInfoHash::Unknown(_), NodeCloseToInfoHash::HasPeers { .. }) => {
                Ordering::Less
            }
            (NodeCloseToInfoHash::Unknown(_), NodeCloseToInfoHash::Peerless { .. }) => {
                Ordering::Less
            }
            _ => Ordering::Equal,
        }
    }
}

async fn fetch_peers(
    info_hash: &ID,
    active_nodes: &mut NodeMap,
    connection: &mut QueryingConnection,
    peers: &mut Peers,
) -> Result<(), ConError> {
    // let mut active_nodes = NodeMap::new();

    for addr in active_nodes.keys() {
        connection.send_get_peers(*info_hash, *addr).await?;
    }

    loop {
        let resp = tokio::select! {
            resp = connection.recv_resp() => {resp},
            _ = sleep(Duration::from_secs(3)) => {break}
        };

        let Resp::GetPeersResp { from, token, v_or_n } = resp else {
            warn!("expected response to get_peers message, got {:?}", resp);
            continue;
        };

        let Some(v) = active_nodes.get_mut(&from) else {
            continue; // it's by someone who already got marked as inactive
        };

        match v_or_n {
            ValuesOrNodes::Values { values } => {
                peers.extend(values.into_iter());
                *v = NodeCloseToInfoHash::HasPeers { id: v.id(), token };
            }
            ValuesOrNodes::Nodes { nodes } => {
                *v = NodeCloseToInfoHash::Peerless { id: v.id(), token };

                for Node { addr, id, .. } in nodes.iter() {
                    if let hash_map::Entry::Vacant(entry) = active_nodes.entry(*addr) {
                        connection.send_get_peers(*info_hash, *addr).await?;
                        entry.insert(NodeCloseToInfoHash::Unknown(*id));
                    }
                }
            }
        }
        debug!("node map len: {}", active_nodes.len());
    }

    Ok(())
}

struct CappedGrowingInterval {
    growth_formula: fn(u64) -> u64,
    iter_count: u64,
    max: u64,
}

impl CappedGrowingInterval {
    fn new(max: u64, growth_formula: fn(u64) -> u64) -> Self {
        Self {
            growth_formula,
            iter_count: 0,
            max,
        }
    }

    async fn tick(&mut self) {
        let sleep_duration = Ord::min((self.growth_formula)(self.iter_count), self.max);
        sleep(Duration::from_secs(sleep_duration)).await;
        self.iter_count += 1;
        debug!("tick");
    }
}
