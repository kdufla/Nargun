pub mod connection;
pub mod krpc_message;
pub mod routing_table;

use self::{
    connection::{CommandType, Connection, QueryCommand, RespCommand, MTU},
    krpc_message::{Nodes, Peer, ValuesOrNodes},
    routing_table::RoutingTable,
};
use crate::{constants::T26IX, dht::connection::ConCommand, peer::Peers, util::id::ID};
use anyhow::{anyhow, Result};
use bytes::Bytes;
use rand::{seq::IteratorRandom, thread_rng};
use std::{
    collections::{HashMap, HashSet},
    net::SocketAddrV4,
    time::Duration,
};
use tokio::{select, sync::mpsc, time::sleep};
use tracing::{debug, error};

const MAX_NODE_FETCH_SLEEP: u64 = 60;
const MAX_PEER_FETCH_SLEEP: u64 = 180;

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
    let own_node_id = ID(rand::random());

    let mut routing_table = RoutingTable::new(own_node_id.to_owned());

    let (dht_tx, mut dht_rx) = mpsc::channel(64);
    let (conn_tx, conn_rx) = mpsc::channel(64);

    let udp_connection =
        Connection::new(own_node_id.to_owned(), conn_tx, conn_rx, dht_tx.to_owned());

    let known_peers: HashSet<SocketAddrV4> = peers.peer_addresses().into_iter().collect();
    let mut peer_map = HashMap::from([(info_hash.to_owned(), known_peers)]);

    let dht_tx_clone = dht_tx.clone();
    tokio::spawn(async move {
        periodically_fetch_nodes(dht_tx_clone).await;
    });

    let dht_tx_clone = dht_tx.clone();
    tokio::spawn(async move {
        periodically_fetch_peers(dht_tx_clone, info_hash).await;
    });

    loop {
        match select! {
            addr = new_peer_with_dht.recv() => try_ping_node(&udp_connection, addr).await,
            command = dht_rx.recv() => process_incoming_command(command, &mut routing_table, &udp_connection, &peers, &mut peer_map, &own_node_id).await,
        } {
            Ok(_) => debug!(
                "command handled\nrouting_table = {:?}\npeer_map = {:?}",
                routing_table, peer_map
            ),
            Err(e) => error!("dht command failed. e = {:?}", e),
        }
    }
}

async fn process_incoming_command(
    command: Option<DhtCommand>,
    routing_table: &mut RoutingTable,
    connection: &Connection,
    peers: &Peers,
    peer_map: &mut HashMap<ID, HashSet<SocketAddrV4>>,
    own_id: &ID,
) -> Result<()> {
    let Some(command)  = command else{
        return Err(anyhow!("DhtCommand is None"));
    };

    debug!("incoming command = {:?}", command);

    match command {
        DhtCommand::Touch(id, addr) => routing_table.touch_node(id, addr),
        DhtCommand::NewNodes(nodes) => {
            for node in nodes.iter() {
                try_ping_node(connection, Some(node.addr)).await?
            }
        }
        DhtCommand::NewPeers(peer_list, info_hash) => {
            let addr_list = peer_list.iter().map(|p| p.addr().to_owned()).collect();

            if info_hash == *own_id {
                // TODO this is dumb.. peer is just a wrapper, insert should accept it.
                peers.insert_list(&addr_list);
            }

            peer_map
                .entry(info_hash)
                .and_modify(|stored_peers| {
                    for peer in addr_list.iter() {
                        stored_peers.insert(peer.to_owned());
                    }
                })
                .or_insert(addr_list.into_iter().collect());
        }
        DhtCommand::FindNode(target, tid, from) => {
            try_find_node(connection, routing_table, &target, from, tid).await?
        }
        DhtCommand::GetPeers(info_hash, tid, from) => {
            let vorn = match peer_map.get(&info_hash) {
                Some(peer_list) => {
                    let mut rng = thread_rng();
                    let sample = peer_list
                        .iter()
                        .choose_multiple(&mut rng, MTU / T26IX)
                        .iter()
                        .map(|addr| Peer::new(*addr.clone()))
                        .collect();

                    ValuesOrNodes::Values { values: sample }
                }
                None => {
                    let Some(nodes) = routing_table.find_node(&info_hash) else {
                        return Err(anyhow!("can't find nodes"));
                    };

                    ValuesOrNodes::Nodes { nodes }
                }
            };

            let (command, _) = ConCommand::new(
                connection::CommandType::Resp(RespCommand::GetPeers { v_or_n: vorn }, tid),
                from,
            );

            connection.send(command).await?
        }
        DhtCommand::FetchNodes => fetch_nodes(routing_table, connection).await,
        DhtCommand::FetchPeers(info_hash) => {
            fetch_peers(routing_table, connection, info_hash).await
        }
    }

    Ok(())
}

async fn try_find_node(
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

    Ok(connection.send(command).await?)
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

async fn fetch_nodes(routing_table: &mut RoutingTable, connection: &Connection) {
    for id in routing_table.iter_over_ids_within_fillable_buckets() {
        debug!("fetch_nodes for {:?}", id);
        let node = match routing_table.find_node(&id) {
            Some(nodes) => match nodes {
                Nodes::Exact(exact_node) => exact_node,
                Nodes::Closest(closest_nodes) => match closest_nodes.into_iter().next() {
                    Some(node) => node,
                    None => continue,
                },
            },
            None => continue,
        };

        let (command, _) = ConCommand::new(
            CommandType::Query(QueryCommand::FindNode { target: id }),
            node.addr,
        );

        let _ = connection.send(command).await;
    }
}

async fn fetch_peers(routing_table: &RoutingTable, connection: &Connection, info_hash: ID) {
    let mut close_nodes = [None, None, None];

    match routing_table.find_node(&info_hash) {
        Some(nodes) => match nodes {
            Nodes::Exact(exact_node) => {
                close_nodes[0] = Some(exact_node);
            }
            Nodes::Closest(closest_nodes) => {
                for (i, close_node) in closest_nodes
                    .into_iter()
                    .take(close_nodes.len())
                    .enumerate()
                {
                    close_nodes[i] = Some(close_node);
                }
            }
        },
        None => return,
    };

    for node in close_nodes.into_iter().filter_map(std::convert::identity) {
        let (command, _) = ConCommand::new(
            CommandType::Query(QueryCommand::GetPeers {
                info_hash: info_hash.to_owned(),
            }),
            node.addr,
        );

        let _ = connection.send(command).await;
    }
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
