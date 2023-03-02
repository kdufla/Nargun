pub mod command;
pub mod error;
pub mod pending;

use self::command::{CommandType, ConCommand, QueryCommand, RespCommand};
use self::error::Error;
use self::pending::PendingRequests;
use super::krpc_message::{Arguments, Message, Nodes, Response, ValuesOrNodes};
use crate::client::Peer;
use crate::data_structures::{NoSizeBytes, ID};
use crate::gateway_device::LOCAL_PORT_UDP;
use bytes::Bytes;
use std::borrow::Borrow;
use std::net::SocketAddr;
use std::net::SocketAddrV4;
use tokio::net::UdpSocket;
use tokio::select;
use tokio::sync::mpsc;
use tracing::{debug, error, info, instrument, warn};

pub const MTU: usize = 1300;

#[derive(Debug)]
pub struct Connection {
    querying_connection: QueryingConnection,
    send_message: mpsc::Sender<ConCommand>,
    query_listener: mpsc::Receiver<Query>,
}

#[derive(Debug)]
pub struct QueryingConnection {
    send_message: mpsc::Sender<ConCommand>,
    response_receiver_channel_rx: mpsc::Receiver<Resp>,
    response_receiver_channel_tx: mpsc::Sender<Resp>,
}

#[derive(Debug)]
pub struct QueryId {
    target: SocketAddrV4,
    transaction_id: Bytes,
}

#[derive(Debug)]
pub enum ConMsg {
    Query(Query),
    Resp(Resp),
}

#[derive(Debug)]
pub enum Query {
    FindNode { target: ID, query_id: QueryId },
    GetPeers { info_hash: ID, query_id: QueryId },
    NewPeer { new_peer: Peer, info_hash: ID },
}

#[derive(Debug)]
pub enum Resp {
    Touch {
        id: ID,
        from: SocketAddrV4,
    },
    NewNodes {
        nodes: Nodes,
        from: SocketAddrV4,
        id: ID,
    },
    GetPeersResp {
        from: SocketAddrV4,
        token: NoSizeBytes,
        v_or_n: ValuesOrNodes,
    },
}

impl QueryingConnection {
    pub async fn recv_resp(&mut self) -> Resp {
        // unwrap because one sender is always alive in Self i.e. even if connection dies, this will continue waiting
        // it's not a problem since this is just a limited cloneable part of connection
        self.response_receiver_channel_rx.recv().await.unwrap()
    }

    pub async fn send_ping(&self, target: SocketAddrV4) -> Result<(), Error> {
        self.send_query(QueryCommand::Ping, target).await
    }

    pub async fn send_find_node(&self, node_id: ID, target: SocketAddrV4) -> Result<(), Error> {
        self.send_query(QueryCommand::FindNode { target: node_id }, target)
            .await
    }

    pub async fn send_get_peers(&self, info_hash: ID, target: SocketAddrV4) -> Result<(), Error> {
        self.send_query(QueryCommand::GetPeers { info_hash }, target)
            .await
    }

    pub async fn send_announce_peer(
        &self,
        info_hash: ID,
        port: u16,
        token: Bytes,
        target: SocketAddrV4,
    ) -> Result<(), Error> {
        self.send_query(
            QueryCommand::AnnouncePeer {
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
        command_type: QueryCommand,
        target: SocketAddrV4,
    ) -> Result<(), Error> {
        let command = ConCommand {
            command_type: CommandType::Query {
                command: command_type,
                resp_returner: self.response_receiver_channel_tx.clone(),
            },
            target,
        };

        self.send_message
            .send(command)
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

    pub async fn recv(&mut self) -> Result<ConMsg, Error> {
        let Self {
            querying_connection,
            query_listener,
            ..
        } = self;

        let msg = tokio::select! {
            resp = querying_connection.recv_resp() => ConMsg::Resp(resp),
            query = query_listener.recv() => {
                ConMsg::Query(query.ok_or(Error::ConDropped { source: None })?)
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
        self.send_resp(RespCommand::FindNode { nodes }, query_id)
            .await
    }

    pub async fn resp_to_get_peers(
        &self,
        values_or_nodes: ValuesOrNodes,
        query_id: QueryId,
    ) -> Result<(), Error> {
        self.send_resp(RespCommand::GetPeers { values_or_nodes }, query_id)
            .await
    }

    async fn send_resp(&self, command_type: RespCommand, query_id: QueryId) -> Result<(), Error> {
        let command = ConCommand {
            command_type: CommandType::Resp {
                command: command_type,
                tid: query_id.transaction_id,
            },
            target: query_id.target,
        };

        self.send_message
            .send(command)
            .await
            .map_err(|e| Error::ConDropped {
                source: Some(Box::new(e)),
            })
    }
}

struct ConnectionManager {
    own_id: ID,
    sock: UdpSocket,
    send_to_remote: mpsc::Receiver<ConCommand>,
    query_listener: mpsc::Sender<Query>,
    pending: PendingRequests,
    secret: ID,
}

impl ConnectionManager {
    fn start(
        own_id: ID,
        send_to_remote: mpsc::Receiver<ConCommand>,
        query_listener: mpsc::Sender<Query>,
    ) {
        tokio::spawn(async move {
            let sock = UdpSocket::bind(format!("0.0.0.0:{LOCAL_PORT_UDP}"))
                .await
                .unwrap();

            let manager = Self {
                own_id,
                sock,
                send_to_remote,
                query_listener,
                pending: PendingRequests::new(),
                secret: ID::new(rand::random()),
            };

            manager.manage_udp().await;
        });
    }

    #[instrument(skip_all)]
    async fn manage_udp(mut self) -> Option<()> {
        let mut buf = vec![0u8; MTU];

        loop {
            select! {
                res = self.sock.recv_from(&mut buf) => {
                    let (_ , from) = res.ok()?;
                    self.handle_received_message(from, &buf).await;
                },
                send_command = self.send_to_remote.recv() => {
                    self.send(send_command?).await;
                },
            }
        }
    }

    async fn send(&mut self, command: ConCommand) {
        let ConCommand {
            command_type,
            target,
        } = command;

        debug!(?command_type);

        let message = match command_type {
            CommandType::Query {
                command,
                resp_returner,
            } => {
                let message = command.into_message(&self.own_id);
                self.pending
                    .insert(message.tid().as_bytes(), (&message).into(), resp_returner);

                message
            }
            CommandType::Resp { command, tid } => {
                let token = self.secret.hash_as_bytes(&target.ip().octets());
                command.into_message(&self.own_id, tid, token)
            }
        };

        let message = match message.into_bytes() {
            Ok(message) => message,
            Err(e) => {
                error!(?e);
                return;
            }
        };

        if let Err(e) = self.sock.send_to(&message, target).await {
            warn!(?e);
        };
    }

    async fn handle_received_message(&mut self, from: SocketAddr, buf: &[u8]) {
        let  SocketAddr::V4(from) = from else {
            info!("IPv6 request dropped");
            return;
        };

        let message = match Message::from_bytes(&buf) {
            Ok(message) => message,
            Err(e) => {
                error!("can't parse e = {:?}, buf = {:?}", e, buf);
                return;
            }
        };

        debug!(?message);

        match message {
            Message::Query {
                transaction_id,
                arguments,
                ..
            } => {
                self.handle_query(transaction_id.into_bytes(), arguments, from)
                    .await
            }
            Message::Response {
                transaction_id,
                response,
                ..
            } => {
                self.handle_resp(transaction_id.into_bytes(), response, from)
                    .await
            }
            Message::Error { .. } => (),
        }
    }

    async fn handle_query(
        &mut self,
        transaction_id: Bytes,
        arguments: Arguments,
        from: SocketAddrV4,
    ) {
        match arguments {
            Arguments::Ping { .. } => {
                self.send(ConCommand::new(
                    CommandType::Resp {
                        command: RespCommand::Ping,
                        tid: transaction_id,
                    },
                    from,
                ))
                .await;
            }
            Arguments::FindNode { target, .. } => {
                self.query_listener
                    .send(Query::FindNode {
                        target,
                        query_id: QueryId {
                            target: from,
                            transaction_id,
                        },
                    })
                    .await
                    .unwrap();
            }
            Arguments::GetPeers { info_hash, .. } => {
                self.query_listener
                    .send(Query::GetPeers {
                        info_hash,
                        query_id: QueryId {
                            target: from,
                            transaction_id,
                        },
                    })
                    .await
                    .unwrap();
            }
            Arguments::AnnouncePeer {
                info_hash,
                port,
                token,
                ..
            } => {
                if self.token_is_valid(token.into_bytes(), &from) {
                    let mut peer_addr = from;
                    peer_addr.set_port(port);

                    self.query_listener
                        .send(Query::NewPeer {
                            new_peer: Peer::new(peer_addr),
                            info_hash,
                        })
                        .await
                        .unwrap();

                    self.send(ConCommand::new(
                        CommandType::Resp {
                            command: RespCommand::AnnouncePeer,
                            tid: transaction_id,
                        },
                        from,
                    ))
                    .await;
                }
            }
        }
    }

    fn token_is_valid(&self, token: Bytes, source_addr: &SocketAddrV4) -> bool {
        token == self.secret.hash_as_bytes(&source_addr.ip().octets())
    }

    async fn handle_resp(&mut self, transaction_id: Bytes, response: Response, from: SocketAddrV4) {
        let Some(querying_task_channel) =
            self.pending.get(&transaction_id, response.borrow().into())
        else {
            warn!("resp not pending");
            return;
        };

        match response {
            Response::Ping { id } => {
                querying_task_channel
                    .send(Resp::Touch { id, from })
                    .await
                    .unwrap();
            }
            Response::FindNode { nodes, id } => {
                querying_task_channel
                    .send(Resp::NewNodes { nodes, from, id })
                    .await
                    .unwrap();
            }
            Response::GetPeers {
                values_or_nodes,
                token,
                ..
            } => {
                querying_task_channel
                    .send(Resp::GetPeersResp {
                        from,
                        token,
                        v_or_n: values_or_nodes,
                    })
                    .await
                    .unwrap();
            }
            Response::AnnouncePeer { id } => {
                querying_task_channel
                    .send(Resp::Touch { id, from })
                    .await
                    .unwrap();
            }
        }
    }
}
