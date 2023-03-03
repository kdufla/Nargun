use super::{
    connection::{ConError, Connection, FromConResp, QueryingConnection},
    krpc_message::{Nodes, ValuesOrNodes},
    routing_table::{Node, RoutingTable},
};
use crate::{
    capped_growing_interval::CappedGrowingInterval,
    client::Peers,
    data_structures::{NoSizeBytes, ID},
};
use anyhow::bail;
use std::{
    cmp::Ordering,
    collections::{hash_map::Entry::Vacant, HashMap},
    net::SocketAddrV4,
    time::Duration,
};
use tokio::time::sleep;
use tracing::{error, warn};

const MAX_SECS_INTERVAL_BETWEEN_PEER_FETCHES: f64 = 120.0;
const INITIAL_SECS_INTERVAL: f64 = 7.0;
const INTERVAL_GROWTH: fn(f64) -> f64 = |x: f64| INITIAL_SECS_INTERVAL + 0.01 * x.powf(3.7);
const ANNOUNCE_TO_N_TOP_NODES: usize = 1 << 4;
const ACTIVE_CLOSE_NODES_CAP_LOW: usize = 1 << 7;
const ACTIVE_CLOSE_NODES_CAP_HIGH: usize = 1 << 9;

type NodeMap = HashMap<SocketAddrV4, NodeCloseToInfoHash>;

pub fn spawn_fetcher(
    info_hash: ID,
    tcp_port: u16,
    peers: Peers,
    routing_table: &RoutingTable,
    connection: &Connection,
) -> anyhow::Result<()> {
    let Some(nodes) = routing_table.get_closest_nodes(&info_hash) else {
        bail!("trying to spawn fetcher without nodes routing table");
    };

    let querying_connection = connection.get_querying_connection();

    tokio::spawn(async move {
        if let Err(e) =
            periodically_fetch_peers(info_hash, tcp_port, nodes, querying_connection, peers).await
        {
            error!(?e);
        }
    });

    Ok(())
}

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
        CappedGrowingInterval::new(MAX_SECS_INTERVAL_BETWEEN_PEER_FETCHES, INTERVAL_GROWTH);

    loop {
        interval.tick().await;
        fetch_peers(&info_hash, &mut nodes, &mut connection, &mut peers).await?;
        announce_peer(port, info_hash, &nodes, &mut connection).await?;
    }
}

async fn fetch_peers(
    info_hash: &ID,
    active_nodes: &mut NodeMap,
    connection: &mut QueryingConnection,
    peers: &mut Peers,
) -> Result<(), ConError> {
    for addr in active_nodes.keys() {
        connection.send_get_peers(*info_hash, *addr).await?;
    }

    loop {
        let resp = tokio::select! {
            _ = sleep(Duration::from_secs(3)) => { break },
            resp = connection.recv_resp() => { resp },
        };

        let FromConResp::GetPeers { from, token, v_or_n: values_or_nodes } = resp else {
            warn!("expected response to get_peers message, got {:?}", resp);
            continue;
        };

        let Some(active_node) = active_nodes.get_mut(&from) else {
            continue; // response is by someone who already got marked as inactive or too far to care
        };

        match values_or_nodes {
            ValuesOrNodes::Values { values } => {
                peers.extend(values.into_iter());

                *active_node = NodeCloseToInfoHash::HasPeers {
                    id: active_node.id(),
                    token,
                };
            }
            ValuesOrNodes::Nodes { nodes } => {
                *active_node = NodeCloseToInfoHash::Peerless {
                    id: active_node.id(),
                    token,
                };

                get_peers_from_new_nodes(*info_hash, nodes, active_nodes, connection).await?;
            }
        }

        if active_nodes.len() >= ACTIVE_CLOSE_NODES_CAP_HIGH {
            forget_far_nodes(active_nodes, info_hash);

            // don't overuse this. it uses global lock for O(n) operation
            if peers.enough_peers() {
                break;
            }
        }
    }

    Ok(())
}

async fn announce_peer(
    port: u16,
    info_hash: ID,
    nodes: &NodeMap,
    connection: &mut QueryingConnection,
) -> Result<(), ConError> {
    let sorted_nodes = get_sorted_nodes(nodes, &info_hash);

    let sorted_known_nodes = sorted_nodes
        .into_iter()
        .flat_map(|(addr, node)| node.token().map(|token| (addr, token)))
        .take(ANNOUNCE_TO_N_TOP_NODES);

    for (addr, token) in sorted_known_nodes {
        connection
            .send_announce_peer(info_hash, port, token.as_bytes(), *addr)
            .await?
    }

    Ok(())
}

fn forget_far_nodes(nodes: &mut NodeMap, info_hash: &ID) {
    let sorted_nodes = get_sorted_nodes(nodes, info_hash);

    *nodes = sorted_nodes
        .into_iter()
        .take(ACTIVE_CLOSE_NODES_CAP_LOW)
        .map(|(addr, node)| (*addr, node.to_owned()))
        .collect();
}

fn get_sorted_nodes<'a>(
    nodes: &'a NodeMap,
    info_hash: &ID,
) -> Vec<(&'a SocketAddrV4, &'a NodeCloseToInfoHash)> {
    let mut nodes: Vec<(&SocketAddrV4, &NodeCloseToInfoHash)> = nodes.iter().collect();

    nodes.sort_by(|(_, a), (_, b)| {
        let discriminant_ordering = a.cmp_discriminant(b);
        if discriminant_ordering != Ordering::Equal {
            return discriminant_ordering;
        }

        (&a.id() - info_hash).cmp(&(&b.id() - info_hash))
    });

    nodes
}

async fn get_peers_from_new_nodes(
    info_hash: ID,
    new_nodes: Nodes,
    active_nodes: &mut NodeMap,
    connection: &mut QueryingConnection,
) -> Result<(), ConError> {
    for Node { addr, id, .. } in new_nodes.iter() {
        if let Vacant(vacant_entry) = active_nodes.entry(*addr) {
            connection.send_get_peers(info_hash, *addr).await?;
            vacant_entry.insert(NodeCloseToInfoHash::Unknown(*id));
        }
    }

    Ok(())
}

#[derive(Debug, Clone)]
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
                Ordering::Less
            }
            (NodeCloseToInfoHash::HasPeers { .. }, NodeCloseToInfoHash::Unknown(_)) => {
                Ordering::Less
            }
            (NodeCloseToInfoHash::Peerless { .. }, NodeCloseToInfoHash::Unknown(_)) => {
                Ordering::Less
            }
            (NodeCloseToInfoHash::Peerless { .. }, NodeCloseToInfoHash::HasPeers { .. }) => {
                Ordering::Greater
            }
            (NodeCloseToInfoHash::Unknown(_), NodeCloseToInfoHash::HasPeers { .. }) => {
                Ordering::Greater
            }
            (NodeCloseToInfoHash::Unknown(_), NodeCloseToInfoHash::Peerless { .. }) => {
                Ordering::Greater
            }
            _ => Ordering::Equal,
        }
    }
}
