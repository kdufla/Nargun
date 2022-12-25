pub mod connection;
pub mod krpc_message;
pub mod routing_table;

use self::{
    connection::{Connection, RespCommand, MTU},
    krpc_message::{Nodes, Peer, ValuesOrNodes},
    routing_table::RoutingTable,
};
use crate::{constants::T26IX, dht::connection::Command as ConCommand, peer::Peers, util::id::ID};
use anyhow::{anyhow, Result};
use rand::{seq::IteratorRandom, thread_rng};
use std::{collections::HashMap, net::SocketAddrV4};
use tokio::{select, sync::mpsc};

pub enum DhtCommand {
    Touch(ID, SocketAddrV4),
    NewNodes(Nodes),
    NewPeers(Vec<Peer>, ID),
    FindNode(ID, String, SocketAddrV4),
    GetPeers(ID, String, SocketAddrV4),
}

pub async fn dht(peers: Peers, info_hash: ID, mut new_peer_with_dht: mpsc::Receiver<SocketAddrV4>) {
    let own_node_id = ID(rand::random());

    let (dht_tx, mut dht_rx) = mpsc::channel(64);
    let (conn_tx, conn_rx) = mpsc::channel(64);

    let mut routing_table = RoutingTable::new(own_node_id.to_owned());

    let udp_connection = Connection::new(own_node_id.to_owned(), conn_tx, conn_rx, dht_tx);

    let mut peer_map = HashMap::new();
    let _ = peer_map.insert(info_hash.to_owned(), peers.peer_addresses());

    loop {
        let x = select! {
            addr = new_peer_with_dht.recv() => try_ping_node(&udp_connection, addr).await,
            command = dht_rx.recv() => process_incoming_command(command, &mut routing_table, &udp_connection, &peers, &mut peer_map, &own_node_id).await,
        };
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
        return Err(anyhow!("connection closed"));
    };

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

            match peer_map.get_mut(&info_hash) {
                Some(peers) => peers.append(&mut addr_list),
                None => {
                    let _ = peer_map.insert(info_hash, addr_list);
                }
            }
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
                    let Some(nodes) = routing_table.find_remote_node(&info_hash) else {
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
    }

    Ok(())
}

async fn try_find_node(
    connection: &Connection,
    routing_table: &mut RoutingTable,
    target: &ID,
    from: SocketAddrV4,
    tid: String,
) -> Result<()> {
    let Some(nodes) = routing_table.find_remote_node(&target) else {
        return Err(anyhow!("can't find nodes"));
    };

    let (command, _) = ConCommand::new(
        connection::CommandType::Resp(RespCommand::FindNode { nodes }, tid),
        from,
    );

    Ok(connection.conn_command_tx.send(command).await?)
}

async fn try_ping_node(connection: &Connection, addr: Option<SocketAddrV4>) -> Result<()> {
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
