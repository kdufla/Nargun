use super::krpc_message::{Arguments, Message, Nodes, Peer, Response, ValuesOrNodes};
use super::DhtCommand;
use crate::util::id::ID;
use bytes::Bytes;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::net::SocketAddrV4;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::net::UdpSocket;
use tokio::select;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tracing::{debug, error, info, trace, warn};

pub const MTU: usize = 1300;

#[derive(Clone, Debug)]
pub struct Connection {
    own_id: ID,
    pub conn_command_tx: mpsc::Sender<ConCommand>,
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

    async fn manage_udp(
        self,
        conn_command_rx: mpsc::Receiver<ConCommand>,
        dht_command_tx: mpsc::Sender<DhtCommand>,
    ) {
        let sock = UdpSocket::bind("0.0.0.0:46491").await.unwrap();
        let _local_addr = sock.local_addr().unwrap().port(); // TODO PORT message needs this

        info!(?sock);

        let secret = ID(rand::random());

        let sock_send = Arc::new(sock);
        let sock_recv = sock_send.clone();

        let pending = PendingRequests::new(); //TODO

        select! {
            _ = self.manage_sender(conn_command_rx, sock_send, secret.clone(), pending.clone()) => {},
            _ = self.manage_receiver(sock_recv, dht_command_tx, secret, pending.clone()) => {},
        }

        debug!("unreachable! select should not exit");
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

            let token = secret.hash_as_bytes(&command.target.ip().octets());

            let message = match command.command_type {
                CommandType::Query(query) => {
                    let (tid, message) = self.build_query_from_command(query);
                    pending.insert(tid, Self::get_type(&message));
                    message
                }
                CommandType::Resp(resp, tid) => self.build_resp_from_command(resp, tid, token),
            };

            debug!("send {:?}", message);

            let message = match message.to_bytes() {
                Ok(message) => message,
                Err(e) => {
                    error!(?e);
                    continue;
                }
            };

            if let Err(e) = sock.send_to(&message, command.target).await {
                warn!(?e);
            };
        }
    }

    fn get_type(message: &Message) -> RequestType {
        let Message::Query{arguments, ..} = message else{
            panic!("this should be a query");
        };

        match arguments {
            Arguments::Ping { .. } => RequestType::Ping,
            Arguments::FindNode { target, .. } => RequestType::FindNode(target.to_owned()),
            Arguments::GetPeers { info_hash, .. } => RequestType::GetPeers(info_hash.to_owned()),
            Arguments::AnnouncePeer { .. } => RequestType::AnnouncePeer,
        }
    }

    fn build_query_from_command(&self, query: QueryCommand) -> (Bytes, Message) {
        match query {
            QueryCommand::Ping => Message::ping_query(&self.own_id),
            QueryCommand::FindNode { target } => Message::find_nodes_query(&self.own_id, target),
            QueryCommand::GetPeers { info_hash } => {
                Message::get_peers_query(&self.own_id, info_hash)
            }
            QueryCommand::AnnouncePeer {
                info_hash,
                port,
                token,
            } => Message::announce_peer_query(&self.own_id, info_hash, port, token),
        }
    }

    fn build_resp_from_command(&self, resp: RespCommand, tid: Bytes, token: Bytes) -> Message {
        match resp {
            RespCommand::Ping => Message::ping_resp(&self.own_id, tid),
            RespCommand::FindNode { nodes } => Message::find_nodes_resp(&self.own_id, nodes, tid),
            RespCommand::GetPeers { v_or_n } => {
                Message::get_peers_resp(&self.own_id, token, v_or_n, tid)
            }
            RespCommand::AnnouncePeer => Message::announce_peer_resp(&self.own_id, tid),
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

        while let Ok((bytes_read, from_addr)) = sock.recv_from(&mut buf).await {
            trace!("rec raw {:?}", buf);

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

            self.handle_message(message, from_addr, &dht_command_tx, &pending, &secret)
                .await;
        }
    }

    async fn handle_message(
        &self,
        message: Message,
        from: SocketAddr,
        dht_command_tx: &mpsc::Sender<DhtCommand>,
        pending: &PendingRequests,
        secret: &ID,
    ) {
        let  SocketAddr::V4(from) = from else {
            info!("IPv6 request dropped");
            return;
        };

        match message {
            Message::Query {
                transaction_id,
                arguments,
                ..
            } => {
                Self::handle_query(
                    transaction_id.into_bytes(),
                    arguments,
                    from,
                    &self.conn_command_tx,
                    dht_command_tx,
                    secret,
                )
                .await
            }
            Message::Response {
                transaction_id,
                response,
                ..
            } => {
                Self::handle_resp(
                    transaction_id.into_bytes(),
                    response,
                    from,
                    dht_command_tx,
                    pending,
                )
                .await
            }
            Message::Error { .. } => todo!(),
        }
    }

    async fn handle_query(
        transaction_id: Bytes,
        arguments: Arguments,
        from: SocketAddrV4,
        conn_command_tx: &mpsc::Sender<ConCommand>,
        dht_command_tx: &mpsc::Sender<DhtCommand>,
        secret: &ID,
    ) {
        match arguments {
            Arguments::Ping { .. } => {
                let (command, _) =
                    ConCommand::new(CommandType::Resp(RespCommand::Ping, transaction_id), from);
                let _ = conn_command_tx.send(command).await;
            }
            Arguments::FindNode { target, .. } => {
                let _ = dht_command_tx
                    .send(DhtCommand::FindNode(target, transaction_id, from))
                    .await;
            }
            Arguments::GetPeers { info_hash, .. } => {
                let _ = dht_command_tx
                    .send(DhtCommand::GetPeers(info_hash, transaction_id, from))
                    .await;
            }
            Arguments::AnnouncePeer {
                info_hash,
                port,
                token,
                ..
            } => {
                let correct_token = secret.hash_as_bytes(&from.ip().octets());

                if correct_token == token.into_bytes() {
                    let mut peer_addr = from.clone();
                    peer_addr.set_port(port);
                    let peer = Peer::new(peer_addr);
                    let _ = dht_command_tx
                        .send(DhtCommand::NewPeers(vec![peer], info_hash.to_owned()))
                        .await;

                    let (command, _) = ConCommand::new(
                        CommandType::Resp(RespCommand::AnnouncePeer, transaction_id),
                        from,
                    );
                    let _ = conn_command_tx.send(command).await;
                }
            }
        }
    }

    async fn handle_resp(
        transaction_id: Bytes,
        response: Response,
        from: SocketAddrV4,
        dht_command_tx: &mpsc::Sender<DhtCommand>,
        pending: &PendingRequests,
    ) {
        let Some(pending_request) = pending.get(&transaction_id) else {
            warn!("resp from an unknown node");
            return;
        };

        match response {
            Response::Ping { id } => {
                if !matches!(pending_request, RequestType::Ping) {
                    return;
                }

                let _ = dht_command_tx.send(DhtCommand::Touch(id, from)).await;
            }
            Response::FindNode { id, nodes } => {
                if !matches!(pending_request, RequestType::FindNode(_)) {
                    return;
                };

                let _ = dht_command_tx.send(DhtCommand::Touch(id, from)).await;
                let _ = dht_command_tx.send(DhtCommand::NewNodes(nodes)).await;
            }
            Response::GetPeers {
                id,
                token,
                values_or_nodes,
            } => {
                // TODO token
                let RequestType::GetPeers(info_hash) = pending_request else {
                    return;
                };

                let _ = dht_command_tx.send(DhtCommand::Touch(id, from)).await;

                match values_or_nodes {
                    ValuesOrNodes::Values { values } => {
                        let _ = dht_command_tx
                            .send(DhtCommand::NewPeers(values, info_hash.to_owned()))
                            .await;
                    }
                    ValuesOrNodes::Nodes { nodes } => {
                        let _ = dht_command_tx.send(DhtCommand::NewNodes(nodes)).await;
                    }
                }
            }
            Response::AnnouncePeer { id } => {
                if !matches!(pending_request, RequestType::AnnouncePeer) {
                    return;
                }

                let _ = dht_command_tx.send(DhtCommand::Touch(id, from)).await;
            }
        }
    }
}

#[derive(Debug)]
pub enum CommandType {
    Query(QueryCommand),
    Resp(RespCommand, Bytes),
}

#[derive(Debug)]
pub enum QueryCommand {
    Ping,
    FindNode {
        target: ID,
    },
    GetPeers {
        info_hash: ID,
    },
    AnnouncePeer {
        info_hash: ID,
        port: u16,
        token: Bytes,
    },
}

#[derive(Debug)]
pub enum RespCommand {
    Ping,
    FindNode { nodes: Nodes },
    GetPeers { v_or_n: ValuesOrNodes },
    AnnouncePeer,
}

#[derive(Debug)]
pub struct ConCommand {
    pub command_type: CommandType,
    pub target: SocketAddrV4,
    pub response_channel: oneshot::Sender<u64>,
}

impl ConCommand {
    pub fn new(command: CommandType, target: SocketAddrV4) -> (Self, oneshot::Receiver<u64>) {
        let (tx, rx) = oneshot::channel();
        (
            Self {
                command_type: command,
                target,
                response_channel: tx,
            },
            rx,
        )
    }
}

#[derive(Clone, Debug)]
enum RequestType {
    Ping,
    FindNode(ID),
    GetPeers(ID),
    AnnouncePeer,
}

#[derive(Clone, Debug)]
struct PendingRequests(Arc<Mutex<HashMap<Bytes, RequestType>>>);

impl PendingRequests {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(HashMap::<Bytes, RequestType>::new())))
    }

    pub fn get(&self, k: &Bytes) -> Option<RequestType> {
        self.0.lock().unwrap().get(k).cloned()
    }

    pub fn insert(&mut self, k: Bytes, v: RequestType) -> Option<RequestType> {
        self.0.lock().unwrap().insert(k, v)
    }
}
