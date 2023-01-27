pub mod command;
pub mod pending;
pub use self::command::{CommandType, ConCommand, QueryCommand, RespCommand};
pub use self::pending::{PendingRequests, RequestType};
use super::dht::DhtCommand;
use super::krpc_message::{Arguments, Message, Nodes, Response, ValuesOrNodes};
use super::routing_table::Node;
use crate::client::Peer;
use crate::data_structures::ID;
use crate::gateway_device::LOCAL_PORT_UDP;
use bytes::Bytes;
use std::net::SocketAddr;
use std::net::SocketAddrV4;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::select;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::SendError;
use tracing::{debug, error, info, warn};

pub const MTU: usize = 1300;

#[derive(Clone, Debug)]
pub struct Connection {
    own_id: ID,
    conn_command_tx: mpsc::Sender<ConCommand>,
}

impl Connection {
    pub fn new(
        own_id: ID,
        conn_command_tx: mpsc::Sender<ConCommand>,
        conn_command_rx: mpsc::Receiver<ConCommand>,
        dht_command_tx: mpsc::Sender<DhtCommand>,
    ) -> Self {
        let rv = Self {
            own_id,
            conn_command_tx,
        };
        let connection = rv.clone();

        tokio::spawn(async move {
            connection.manage_udp(conn_command_rx, dht_command_tx).await;
        });

        rv
    }

    pub async fn send(&self, command: ConCommand) -> Result<(), SendError<ConCommand>> {
        let res = self.conn_command_tx.send(command).await;
        if let Err(e) = &res {
            debug!("connection can't accept command. e = {:?}", e);
        }
        res
    }

    async fn manage_udp(
        self,
        conn_command_rx: mpsc::Receiver<ConCommand>,
        dht_command_tx: mpsc::Sender<DhtCommand>,
    ) {
        let sock = UdpSocket::bind(format!("0.0.0.0:{}", LOCAL_PORT_UDP))
            .await
            .unwrap();

        let secret = ID::new(rand::random());

        let sock_send = Arc::new(sock);
        let sock_recv = sock_send.clone();

        let pending = PendingRequests::new();

        select! {
            _ = self.manage_sender(conn_command_rx, sock_send, secret.clone(), pending.clone()) => {},
            _ = self.manage_receiver(sock_recv, dht_command_tx, secret, pending.clone()) => {},
        }

        error!("unreachable! select should not exit");
    }

    async fn manage_sender(
        &self,
        mut conn_command_rx: mpsc::Receiver<ConCommand>,
        sock: Arc<UdpSocket>,
        secret: ID,
        mut pending: PendingRequests,
    ) {
        loop {
            let Some(command) = conn_command_rx.recv().await else {break};
            debug!(?command);

            let target = command.target.to_owned();

            let message: Message = command.into_message(&self.own_id, &secret);

            if let Message::Query { transaction_id, .. } = &message {
                pending.insert(transaction_id.as_bytes(), (&message).into());
            }

            debug!("send {:?}", message);

            let message = match message.to_bytes() {
                Ok(message) => message,
                Err(e) => {
                    error!(?e);
                    continue;
                }
            };

            let cloned_sock = sock.clone();
            tokio::spawn(async move {
                if let Err(e) = cloned_sock.send_to(&message, target).await {
                    warn!(?e);
                };
            });
        }
    }

    async fn manage_receiver(
        &self,
        sock: Arc<UdpSocket>,
        dht_command_tx: mpsc::Sender<DhtCommand>,
        secret: ID,
        pending: PendingRequests,
    ) {
        let mut buf = [0u8; MTU];

        let handler = ReceivedMessageHandler::new(
            self.conn_command_tx.to_owned(),
            dht_command_tx,
            secret,
            pending,
        );

        while let Ok((_, from_addr)) = sock.recv_from(&mut buf).await {
            let message = match Message::from_bytes(&buf) {
                Ok(message) => {
                    debug!("rec {:?}", message);
                    message
                }
                Err(e) => {
                    error!("can't parse e = {:?}, buf = {:?}", e, buf);
                    continue;
                }
            };

            handler.handle_message(message, from_addr);
        }
    }
}

struct ReceivedMessageHandler {
    conn_command_tx: mpsc::Sender<ConCommand>,
    dht_command_tx: mpsc::Sender<DhtCommand>,
    secret: ID,
    pending: PendingRequests,
}

impl ReceivedMessageHandler {
    fn new(
        conn_command_tx: mpsc::Sender<ConCommand>,
        dht_command_tx: mpsc::Sender<DhtCommand>,
        secret: ID,
        pending: PendingRequests,
    ) -> Self {
        Self {
            conn_command_tx,
            dht_command_tx,
            secret,
            pending,
        }
    }

    fn handle_message(&self, message: Message, from: SocketAddr) {
        let  SocketAddr::V4(from) = from else {
            info!("IPv6 request dropped");
            return;
        };

        match message {
            Message::Query {
                transaction_id,
                arguments,
                ..
            } => self.handle_query(transaction_id.into_bytes(), arguments, from),
            Message::Response {
                transaction_id,
                response,
                ..
            } => self.handle_resp(transaction_id.into_bytes(), response, from),
            Message::Error { .. } => todo!(),
        }
    }

    fn handle_query(&self, transaction_id: Bytes, arguments: Arguments, from: SocketAddrV4) {
        match arguments {
            Arguments::Ping { .. } => {
                self.conn_send(ConCommand::new(
                    CommandType::Resp(RespCommand::Ping, transaction_id),
                    from,
                ));
            }
            Arguments::FindNode { target, .. } => {
                self.dht_send(DhtCommand::FindNode(target, transaction_id, from));
            }
            Arguments::GetPeers { info_hash, .. } => {
                self.dht_send(DhtCommand::GetPeers(info_hash, transaction_id, from));
            }
            Arguments::AnnouncePeer {
                id,
                info_hash,
                port,
                token,
            } => {
                if self.token_is_valid(token.into_bytes(), &from) {
                    self.store_node(from.to_owned(), port, info_hash, id);
                    self.conn_send(ConCommand::new(
                        CommandType::Resp(RespCommand::AnnouncePeer, transaction_id),
                        from,
                    ))
                }
            }
        }
    }

    fn token_is_valid(&self, token: Bytes, source_addr: &SocketAddrV4) -> bool {
        token == self.secret.hash_as_bytes(&source_addr.ip().octets())
    }

    fn store_node(&self, from: SocketAddrV4, peer_port: u16, info_hash: ID, id: ID) {
        let mut peer_addr = from.clone();
        peer_addr.set_port(peer_port);
        let peer = Peer::new(peer_addr);

        self.dht_send(DhtCommand::NewPeers(vec![peer], info_hash.to_owned()));

        self.dht_send(DhtCommand::NewNodes(Nodes::Exact(Node::new_good(id, from))));
    }

    fn handle_resp(&self, transaction_id: Bytes, response: Response, from: SocketAddrV4) {
        let Some(pending_request) = self.pending.get(&transaction_id) else {
            warn!("resp from an unknown node");
            return;
        };

        let response_type: RequestType = (&response).into();
        if std::mem::discriminant(&pending_request) != std::mem::discriminant(&response_type) {
            warn!("resp on pending req has different type");
            return;
        }

        match response {
            Response::Ping { id } => {
                self.dht_send(DhtCommand::Touch(id, from));
            }
            Response::FindNode { id, nodes } => {
                self.dht_send(DhtCommand::Touch(id, from));
                self.dht_send(DhtCommand::NewNodes(nodes));
            }
            Response::GetPeers {
                id,
                values_or_nodes,
                ..
            } => {
                // TODO token
                let RequestType::GetPeers(info_hash) = pending_request else {
                    return;
                };

                self.dht_send(DhtCommand::Touch(id, from));

                match values_or_nodes {
                    ValuesOrNodes::Values { values } => {
                        self.dht_send(DhtCommand::NewPeers(values, info_hash.to_owned()));
                    }
                    ValuesOrNodes::Nodes { nodes } => {
                        self.dht_send(DhtCommand::NewNodes(nodes));
                    }
                }
            }
            Response::AnnouncePeer { id } => {
                self.dht_send(DhtCommand::Touch(id, from));
            }
        }
    }

    fn conn_send(&self, command: ConCommand) {
        let cloned_tx = self.conn_command_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = cloned_tx.send(command).await {
                warn!(?e);
            };
        });
    }

    fn dht_send(&self, command: DhtCommand) {
        let cloned_tx = self.dht_command_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = cloned_tx.send(command).await {
                warn!(?e);
            };
        });
    }
}
