use super::{
    connection::{ConError, Connection, FromCon, FromConQuery, FromConResp, QueryId, DHT_MTU},
    krpc_message::{Nodes, ValuesOrNodes},
    peer_fetcher::spawn_fetcher,
    routing_table::RoutingTable,
};
use crate::{
    capped_growing_interval::CappedGrowingInterval,
    client::{Peer, Peers, COMPACT_PEER_LEN},
    data_structures::ID,
    shutdown,
};
use anyhow::anyhow;
use rand::{seq::IteratorRandom, thread_rng};
use std::{
    collections::{HashMap, HashSet},
    net::SocketAddrV4,
};
use tokio::{select, sync::mpsc};
use tracing::{instrument, warn};

type PeerMap = HashMap<ID, HashSet<Peer>>;

const MAX_SECS_INTERVAL_BETWEEN_NODE_REFRESH: f64 = 200.0;
const INITIAL_SECS_INTERVAL: f64 = 3.0;
const INTERVAL_GROWTH: fn(f64) -> f64 = |x: f64| INITIAL_SECS_INTERVAL + 0.04 * x.powf(3.0);

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
        connection.send_ping(addr).await?;
    }

    let mut refresh_table_interval =
        CappedGrowingInterval::new(MAX_SECS_INTERVAL_BETWEEN_NODE_REFRESH, INTERVAL_GROWTH);

    let mut spawn_peer_fetcher_when_at_least_one_node_is_good = true;

    loop {
        select! {
            _ = shutdown_rx.recv() => {
                routing_table.store().await;
                return Ok(());
            }
            _ = refresh_table_interval.tick() => { refresh_table(&routing_table, &connection).await? }
            addr = peer_with_dht.recv() => {
                let addr = addr.ok_or(anyhow!("peer receiver failed"))?;
                connection.send_ping(addr).await?;
            },
            message = connection.recv() => {
                match message? {
                    FromCon::Query(query) => process_query(query, &mut routing_table, &connection, &mut peers, &mut peer_map).await?,
                    FromCon::Resp(resp) => process_resp(resp, &connection, &mut routing_table).await?,
                }
            },
        }

        if spawn_peer_fetcher_when_at_least_one_node_is_good
            && routing_table.count_good_nodes() > 0
            && spawn_fetcher(
                info_hash,
                tcp_port,
                peers.clone(),
                &routing_table,
                &connection,
            )
            .is_ok()
        {
            spawn_peer_fetcher_when_at_least_one_node_is_good = false;
        }
    }
}

async fn refresh_table(
    routing_table: &RoutingTable,
    connection: &Connection,
) -> Result<(), ConError> {
    for rand_id in routing_table.iter_over_ids_within_fillable_buckets() {
        if let Some(closest_node) = routing_table.get_closest_node(&rand_id) {
            connection
                .send_find_node(rand_id, closest_node.addr)
                .await?;
        }
    }
    Ok(())
}

async fn process_query(
    query: FromConQuery,
    routing_table: &mut RoutingTable,
    connection: &Connection,
    torrent_peers: &mut Peers,
    dht_peer_map: &mut PeerMap,
) -> Result<(), ConError> {
    match query {
        FromConQuery::FindNode { target, query_id } => {
            process_find_node_query(target, connection, routing_table, query_id).await?
        }
        FromConQuery::GetPeers {
            info_hash,
            query_id,
        } => {
            process_get_peers_query(
                info_hash,
                connection,
                routing_table,
                torrent_peers,
                dht_peer_map,
                query_id,
            )
            .await?
        }
        FromConQuery::NewPeer {
            new_peer,
            info_hash,
        } => store_new_peer(new_peer, info_hash, torrent_peers, dht_peer_map),
    }

    Ok(())
}

async fn process_find_node_query(
    target: ID,
    connection: &Connection,
    routing_table: &mut RoutingTable,
    query_id: QueryId,
) -> Result<(), ConError> {
    if let Some(nodes) = routing_table.find_node(&target) {
        connection.resp_to_find_node(nodes, query_id).await?;
    };

    Ok(())
}

async fn process_get_peers_query(
    info_hash: ID,
    connection: &Connection,
    routing_table: &mut RoutingTable,
    torrent_peers: &mut Peers,
    dht_peer_map: &mut PeerMap,
    query_id: QueryId,
) -> Result<(), ConError> {
    let values_or_nodes = match find_peers(info_hash, torrent_peers, dht_peer_map) {
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

fn find_peers(info_hash: ID, torrent_peers: &Peers, dht_peer_map: &PeerMap) -> Option<Vec<Peer>> {
    if torrent_peers.serve(&info_hash) {
        torrent_peers.get_good_peers(DHT_MTU / COMPACT_PEER_LEN)
    } else {
        let mut rng = thread_rng();
        dht_peer_map.get(&info_hash).map(|peers| {
            peers
                .iter()
                .choose_multiple(&mut rng, DHT_MTU / COMPACT_PEER_LEN)
                .into_iter()
                .cloned()
                .collect()
        })
    }
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
    resp: FromConResp,
    connection: &Connection,
    routing_table: &mut RoutingTable,
) -> Result<(), ConError> {
    match resp {
        FromConResp::Touch { id, from } => routing_table.touch_node(id, from),
        FromConResp::NewNodes { nodes, from, id } => {
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
