pub mod connection;
pub mod krpc_message;
pub mod routing_table;

use self::{
    connection::{CommandType, Connection, QueryCommand, RespCommand, MTU},
    krpc_message::{Nodes, ValuesOrNodes},
    routing_table::RoutingTable,
};
use crate::{
    constants::COMPACT_PEER_LEN,
    dht::connection::ConCommand,
    peer::{Peer, Peers},
    util::id::ID,
};
use anyhow::{anyhow, Result};
use bytes::Bytes;
use rand::{seq::IteratorRandom, thread_rng};
use std::{
    collections::{hash_map, HashMap, HashSet},
    net::SocketAddrV4,
    time::Duration,
};
use tokio::{
    select,
    sync::mpsc::{self, Receiver},
    time::sleep,
};
use tracing::{debug, error, warn};

type PeerMap = HashMap<ID, HashSet<Peer>>;

const MAX_NODE_FETCH_SLEEP: u64 = 60;
const MAX_PEER_FETCH_SLEEP: u64 = 180;
const FETCH_PEER_PARALLEL: usize = 3;

#[derive(Debug)]
pub enum DhtCommand {
    Touch(ID, SocketAddrV4),
    NewNodes(Nodes),
    NewPeers(Vec<Peer>, ID),
    FindNode(ID, Bytes, SocketAddrV4),
    GetPeers(ID, Bytes, SocketAddrV4),
    FetchNodes,
    FetchPeers(ID),
}

pub async fn dht(peers: Peers, info_hash: ID, mut new_peer_with_dht: mpsc::Receiver<SocketAddrV4>) {
    let (mut routing_table, udp_connection, mut dht_command_rx, mut peer_map) =
        setup(&peers, info_hash);

    loop {
        match select! {
            addr = new_peer_with_dht.recv() => try_ping_node(&udp_connection, addr).await,
            command = dht_command_rx.recv() => process_incoming_command(command, &mut routing_table, &udp_connection, &peers, &mut peer_map).await,
        } {
            Ok(_) => debug!(
                "command handled\nrouting_table = {:?}\npeer_map = {:?}",
                routing_table, peer_map
            ),
            Err(e) => error!("dht command failed. e = {:?}", e),
        }
    }
}

fn setup(
    peers: &Peers,
    info_hash: ID,
) -> (RoutingTable, Connection, Receiver<DhtCommand>, PeerMap) {
    let own_node_id = ID(rand::random());

    let routing_table = RoutingTable::new(own_node_id.to_owned());

    let (dht_tx, dht_rx) = mpsc::channel(64);
    let (conn_tx, conn_rx) = mpsc::channel(64);

    let udp_connection =
        Connection::new(own_node_id.to_owned(), conn_tx, conn_rx, dht_tx.to_owned());

    let known_peers: HashSet<Peer> = peers.peer_addresses().into_iter().collect();
    let peer_map = HashMap::from([(info_hash.to_owned(), known_peers)]);

    let dht_tx_clone = dht_tx.clone();
    tokio::spawn(async move {
        periodically_fetch_nodes(dht_tx_clone).await;
    });

    let dht_tx_clone = dht_tx.clone();
    tokio::spawn(async move {
        periodically_fetch_peers(dht_tx_clone, info_hash).await;
    });

    (routing_table, udp_connection, dht_rx, peer_map)
}

async fn process_incoming_command(
    command: Option<DhtCommand>,
    routing_table: &mut RoutingTable,
    connection: &Connection,
    torrent_peers: &Peers,
    dht_peer_map: &mut PeerMap,
) -> Result<()> {
    let Some(command)  = command else{
        return Err(anyhow!("DhtCommand is None"));
    };

    debug!("incoming command = {:?}", command);

    match command {
        DhtCommand::Touch(id, addr) => routing_table.touch_node(id, addr),
        DhtCommand::NewNodes(nodes) => ping_unknown_nodes(nodes, connection, routing_table),
        DhtCommand::NewPeers(new_peers, info_hash) => store_new_peers(
            new_peers,
            info_hash,
            routing_table,
            torrent_peers,
            dht_peer_map,
        ),
        DhtCommand::FindNode(target, tid, from) => {
            find_node(connection, routing_table, &target, from, tid)?
        }
        DhtCommand::GetPeers(info_hash, tid, from) => get_peers(
            connection,
            routing_table,
            dht_peer_map,
            info_hash,
            from,
            tid,
        )?,
        DhtCommand::FetchNodes => fetch_nodes(routing_table, connection),
        DhtCommand::FetchPeers(info_hash) => {
            fetch_peers(routing_table, connection, info_hash).await
        }
    }

    Ok(())
}

fn ping_unknown_nodes(nodes: Nodes, connection: &Connection, routing_table: &RoutingTable) {
    let unknown_node_addresses: Vec<SocketAddrV4> = nodes
        .iter()
        .filter_map(|node| routing_table.contains_node(&node.id).then_some(node.addr))
        .collect();

    let owned_connection = connection.to_owned();
    tokio::spawn(async move {
        for node_addr in unknown_node_addresses {
            if let Err(e) = try_ping_node(&owned_connection, Some(node_addr)).await {
                warn!(?e);
            }
        }
    });
}

fn store_new_peers(
    new_peers: Vec<Peer>,
    info_hash: ID,
    routing_table: &RoutingTable,
    torrent_peers: &Peers,
    dht_peer_map: &mut PeerMap,
) {
    if info_hash == *routing_table.own_id() {
        torrent_peers.insert_list(&new_peers);
    }

    match dht_peer_map.entry(info_hash) {
        hash_map::Entry::Occupied(mut entry) => {
            let stored_peers = entry.get_mut();
            for peer in new_peers.iter() {
                stored_peers.insert(peer.to_owned());
            }
        }
        hash_map::Entry::Vacant(entry) => {
            entry.insert(new_peers.into_iter().collect());
        }
    }
}

fn find_node(
    connection: &Connection,
    routing_table: &mut RoutingTable,
    target: &ID,
    from: SocketAddrV4,
    tid: Bytes,
) -> Result<()> {
    let Some(nodes) = routing_table.find_node(&target) else {
        return Err(anyhow!("can't find nodes"));
    };

    let (command, _) = ConCommand::new(
        connection::CommandType::Resp(RespCommand::FindNode { nodes }, tid),
        from,
    );

    let owned_connection = connection.to_owned();
    tokio::spawn(async move {
        if let Err(e) = owned_connection.send(command).await {
            warn!(?e);
        }
    });

    Ok(())
}

fn get_peers(
    connection: &Connection,
    routing_table: &mut RoutingTable,
    dht_peer_map: &mut PeerMap,
    info_hash: ID,
    from: SocketAddrV4,
    tid: Bytes,
) -> Result<()> {
    let values_or_nodes = match dht_peer_map.get(&info_hash) {
        Some(stored_peers) => {
            let mut rng = thread_rng();

            ValuesOrNodes::Values {
                values: stored_peers
                    .iter()
                    .choose_multiple(&mut rng, MTU / COMPACT_PEER_LEN)
                    .iter()
                    .map(|p| (*p).clone())
                    .collect(),
            }
        }
        None => {
            let Some(nodes) = routing_table.find_node(&info_hash) else {
                return Err(anyhow!("can't find nodes"));
            };

            ValuesOrNodes::Nodes { nodes }
        }
    };

    let (command, _) = ConCommand::new(
        connection::CommandType::Resp(RespCommand::GetPeers { values_or_nodes }, tid),
        from,
    );

    let owned_connection = connection.to_owned();
    tokio::spawn(async move {
        if let Err(e) = owned_connection.send(command).await {
            warn!(?e);
        }
    });

    Ok(())
}

async fn try_ping_node(connection: &Connection, addr: Option<SocketAddrV4>) -> Result<()> {
    debug!("try ping {:?}", addr);
    match addr {
        Some(addr) => Ok(ping_node(connection, addr).await),
        None => Err(anyhow!("missing addr. hint: connection might be closed")),
    }
}

async fn ping_node(connection: &Connection, addr: SocketAddrV4) {
    let (command, _) = ConCommand::new(
        connection::CommandType::Query(connection::QueryCommand::Ping),
        addr,
    );

    let _ = connection.send(command).await;
}

fn fetch_nodes(routing_table: &mut RoutingTable, connection: &Connection) {
    let targets = get_non_full_bucket_infos(routing_table);

    let owned_connection = connection.to_owned();
    tokio::spawn(async move {
        for (id, addr) in targets {
            let (command, _) = ConCommand::new(
                CommandType::Query(QueryCommand::FindNode { target: id }),
                addr,
            );

            if let Err(e) = owned_connection.send(command).await {
                warn!(?e);
            }
        }
    });
}

fn get_non_full_bucket_infos(routing_table: &mut RoutingTable) -> Vec<(ID, SocketAddrV4)> {
    let mut targets = Vec::new();

    for id in routing_table.iter_over_ids_within_fillable_buckets() {
        debug!("fetch_nodes for {:?}", id);
        let node = match routing_table.find_node(&id) {
            Some(nodes) => match nodes.iter().next() {
                Some(node) => node.to_owned(),
                None => continue,
            },
            None => continue,
        };

        targets.push((id, node.addr));
    }

    targets
}

async fn fetch_peers(routing_table: &RoutingTable, connection: &Connection, info_hash: ID) {
    let Some(nodes) = routing_table.find_node(&info_hash) else {
        return;
    };

    let close_node_addresses: Vec<SocketAddrV4> = nodes
        .iter()
        .take(FETCH_PEER_PARALLEL)
        .map(|node| node.addr)
        .collect();

    let owned_connection = connection.to_owned();
    tokio::spawn(async move {
        for addr in close_node_addresses {
            let (command, _) = ConCommand::new(
                CommandType::Query(QueryCommand::GetPeers {
                    info_hash: info_hash.to_owned(),
                }),
                addr,
            );

            if let Err(e) = owned_connection.send(command).await {
                warn!(?e);
            }
        }
    });
}

async fn periodically_fetch_nodes(dht_command_tx: mpsc::Sender<DhtCommand>) {
    let growth_formula = |x: u64| (9.0 + 0.001 * (x as f64).powf(2.4)) as u64;

    for i in 0..u64::MAX {
        match dht_command_tx.send(DhtCommand::FetchNodes).await {
            Ok(_) => debug!("fetch nodes"),
            Err(e) => error!("periodical fetch nodes failed {:?}", e),
        };

        let seconds_to_sleep = growth_formula(i);
        let seconds_to_sleep = if seconds_to_sleep < MAX_NODE_FETCH_SLEEP {
            seconds_to_sleep
        } else {
            MAX_NODE_FETCH_SLEEP
        };

        debug!("fetch_nodes: sleep for {} secs", seconds_to_sleep);
        sleep(Duration::from_secs(seconds_to_sleep)).await;
    }
}

async fn periodically_fetch_peers(dht_command_tx: mpsc::Sender<DhtCommand>, info_hash: ID) {
    let growth_formula = |x: u64| (5.0 + 0.000001 * (x as f64).powf(5.6)) as u64;

    for i in 0..u64::MAX {
        match dht_command_tx
            .send(DhtCommand::FetchPeers(info_hash.to_owned()))
            .await
        {
            Ok(_) => debug!("fetch peers"),
            Err(e) => error!("periodical fetch peers failed {:?}", e),
        };

        let seconds_to_sleep = growth_formula(i);
        let seconds_to_sleep = if seconds_to_sleep < MAX_PEER_FETCH_SLEEP {
            seconds_to_sleep
        } else {
            MAX_PEER_FETCH_SLEEP
        };

        debug!("fetch_peers: sleep for {} secs", seconds_to_sleep);
        sleep(Duration::from_secs(seconds_to_sleep)).await;
    }
}
