use super::channel_message::{FromCon, FromConQuery, FromConResp, ToCon, ToConQuery, ToConResp};
use super::connection_manager::{ConnectionManager, QueryId};
use super::error::Error;
use crate::data_structures::{SerializableBuf, ID};
use crate::dht::krpc_message::{Nodes, ValuesOrNodes};
use std::net::SocketAddrV4;
use tokio::sync::mpsc;

pub const DHT_MTU: usize = 1300;

#[derive(Debug)]
pub struct Connection {
    querying_connection: QueryingConnection,
    send_message: mpsc::Sender<ToCon>,
    query_listener: mpsc::Receiver<FromConQuery>,
}

#[derive(Debug)]
pub struct QueryingConnection {
    send_message: mpsc::Sender<ToCon>,
    response_receiver_channel_rx: mpsc::Receiver<FromConResp>,
    response_receiver_channel_tx: mpsc::Sender<FromConResp>,
}

impl QueryingConnection {
    pub async fn recv_resp(&mut self) -> FromConResp {
        // unwrap because one sender is always alive in Self i.e. even if connection dies, this will continue waiting
        // it's not a problem since this is just a limited cloneable part of connection
        self.response_receiver_channel_rx.recv().await.unwrap()
    }

    pub async fn send_ping(&self, target: SocketAddrV4) -> Result<(), Error> {
        self.send_query(ToConQuery::Ping, target).await
    }

    pub async fn send_find_node(&self, node_id: ID, target: SocketAddrV4) -> Result<(), Error> {
        self.send_query(ToConQuery::FindNode { target: node_id }, target)
            .await
    }

    pub async fn send_get_peers(&self, info_hash: ID, target: SocketAddrV4) -> Result<(), Error> {
        self.send_query(ToConQuery::GetPeers { info_hash }, target)
            .await
    }

    pub async fn send_announce_peer(
        &self,
        info_hash: ID,
        port: u16,
        token: SerializableBuf,
        target: SocketAddrV4,
    ) -> Result<(), Error> {
        self.send_query(
            ToConQuery::AnnouncePeer {
                info_hash,
                port,
                token,
            },
            target,
        )
        .await
    }

    async fn send_query(
        &self,
        query_variant: ToConQuery,
        target: SocketAddrV4,
    ) -> Result<(), Error> {
        let msg = ToCon::Query {
            resp_returner: self.response_receiver_channel_tx.clone(),
            target,
            variant: query_variant,
        };

        self.send_message
            .send(msg)
            .await
            .map_err(|e| Error::ConDropped {
                source: Some(Box::new(e)),
            })
    }
}

impl Clone for QueryingConnection {
    fn clone(&self) -> Self {
        let (response_receiver_channel_tx, response_receiver_channel_rx) = mpsc::channel(1 << 6);
        Self {
            send_message: self.send_message.clone(),
            response_receiver_channel_rx,
            response_receiver_channel_tx,
        }
    }
}

impl Connection {
    pub fn new(own_id: ID) -> Self {
        let (send_to_remote_command_tx, send_to_remote_command_rx) = mpsc::channel(1 << 10);
        let (response_receiver_channel_tx, response_receiver_channel_rx) = mpsc::channel(1 << 10);
        let (query_listener_tx, query_listener_rx) = mpsc::channel(1 << 8);

        let querying_connection = QueryingConnection {
            send_message: send_to_remote_command_tx.clone(),
            response_receiver_channel_rx,
            response_receiver_channel_tx,
        };

        ConnectionManager::start(own_id, send_to_remote_command_rx, query_listener_tx);

        Self {
            querying_connection,
            send_message: send_to_remote_command_tx,
            query_listener: query_listener_rx,
        }
    }

    pub fn get_querying_connection(&self) -> QueryingConnection {
        self.querying_connection.clone()
    }

    pub async fn recv(&mut self) -> Result<FromCon, Error> {
        let Self {
            querying_connection,
            query_listener,
            ..
        } = self;

        let msg = tokio::select! {
            resp = querying_connection.recv_resp() => FromCon::Resp(resp),
            query = query_listener.recv() => {
                FromCon::Query(query.ok_or(Error::ConDropped { source: None })?)
            }
        };

        Ok(msg)
    }

    pub async fn send_ping(&self, target: SocketAddrV4) -> Result<(), Error> {
        self.querying_connection.send_ping(target).await
    }

    pub async fn send_find_node(&self, node_id: ID, target: SocketAddrV4) -> Result<(), Error> {
        self.querying_connection
            .send_find_node(node_id, target)
            .await
    }

    pub async fn resp_to_find_node(&self, nodes: Nodes, query_id: QueryId) -> Result<(), Error> {
        self.send_resp(ToConResp::FindNode { nodes }, query_id)
            .await
    }

    pub async fn resp_to_get_peers(
        &self,
        values_or_nodes: ValuesOrNodes,
        query_id: QueryId,
    ) -> Result<(), Error> {
        self.send_resp(ToConResp::GetPeers { values_or_nodes }, query_id)
            .await
    }

    async fn send_resp(&self, resp_variant: ToConResp, query_id: QueryId) -> Result<(), Error> {
        let msg = ToCon::Resp {
            query_id,
            variant: resp_variant,
        };

        self.send_message
            .send(msg)
            .await
            .map_err(|e| Error::ConDropped {
                source: Some(Box::new(e)),
            })
    }
}
