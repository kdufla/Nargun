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
use std::{collections::HashMap, net::SocketAddrV4, time::Duration};
use tokio::{select, sync::mpsc, time::sleep};
use tracing::{debug, error};

const MAX_FETCH_SLEEP: u64 = 60;

#[derive(Debug)]
pub enum DhtCommand {
    Touch(ID, SocketAddrV4),
    NewNodes(Nodes),
    NewPeers(Vec<Peer>, ID),
    FindNode(ID, Bytes, SocketAddrV4),
    GetPeers(ID, Bytes, SocketAddrV4),
    FetchNodes,
}

pub async fn dht(peers: Peers, info_hash: ID, mut new_peer_with_dht: mpsc::Receiver<SocketAddrV4>) {
    let own_node_id = ID(rand::random());

    let mut routing_table = RoutingTable::new(own_node_id.to_owned());

    let (dht_tx, mut dht_rx) = mpsc::channel(64);
    let (conn_tx, conn_rx) = mpsc::channel(64);

    let udp_connection =
        Connection::new(own_node_id.to_owned(), conn_tx, conn_rx, dht_tx.to_owned());

    let mut peer_map = HashMap::from([(info_hash.to_owned(), peers.peer_addresses())]);

    tokio::spawn(async move {
        periodically_fetch_nodes(dht_tx).await;
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
    peer_map: &mut HashMap<ID, Vec<SocketAddrV4>>,
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
            let mut addr_list = peer_list.iter().map(|p| p.addr().to_owned()).collect();

            if info_hash == *own_id {
                // TODO this is dumb.. peer is just a wrapper, insert should accept it.
                peers.insert_list(&addr_list);
            }

            peer_map
                .entry(info_hash)
                .and_modify(|stored_peers| stored_peers.append(&mut addr_list))
                .or_insert(addr_list);
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

            connection.conn_command_tx.send(command).await?
        }
        DhtCommand::FetchNodes => fetch_nodes(routing_table, connection).await,
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

    Ok(connection.conn_command_tx.send(command).await?)
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

    let _ = connection.conn_command_tx.send(command).await;
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

        connection.conn_command_tx.send(command).await;
    }
}

async fn periodically_fetch_nodes(dht_command_tx: mpsc::Sender<DhtCommand>) {
    let growth_formula = |x: u64| {
        let x = x as f64;
        let y = 10.0 + 3.4 * x.powf(0.65);
        y as u64
    };

    for i in 0..u64::MAX {
        match dht_command_tx.send(DhtCommand::FetchNodes).await {
            Ok(_) => debug!("\n\n\n\n\n\n\n\n\n\nfetch"),
            Err(e) => error!("periodical fetch failed {:?}", e),
        };

        let seconds_to_sleep = growth_formula(i);
        let seconds_to_sleep = if seconds_to_sleep < MAX_FETCH_SLEEP {
            seconds_to_sleep
        } else {
            MAX_FETCH_SLEEP
        };

        debug!("sleep for {} secs", seconds_to_sleep);
        sleep(Duration::from_secs(seconds_to_sleep)).await;
    }
}
