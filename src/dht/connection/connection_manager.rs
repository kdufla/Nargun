use super::channel_message::{FromConQuery, FromConResp, ToCon, ToConResp};
use super::pending::PendingRequests;
use crate::client::Peer;
use crate::data_structures::ID;
use crate::dht::krpc_message::Arguments;
use crate::dht::krpc_message::Message;
use crate::dht::krpc_message::Response;
use crate::gateway_device::LOCAL_PORT_UDP;
use bytes::Bytes;
use std::borrow::Borrow;
use std::net::SocketAddr;
use std::net::SocketAddrV4;
use tokio::net::UdpSocket;
use tokio::select;
use tokio::sync::mpsc;
use tracing::{error, info, instrument, warn};

pub const MTU: usize = 1300;

pub struct ConnectionManager {
    own_id: ID,
    sock: UdpSocket,
    send_to_remote: mpsc::Receiver<ToCon>,
    query_listener: mpsc::Sender<FromConQuery>,
    pending: PendingRequests,
    secret: ID,
}

#[derive(Debug)]
pub struct QueryId {
    target: SocketAddrV4,
    transaction_id: Bytes,
}

impl ConnectionManager {
    pub fn start(
        own_id: ID,
        send_to_remote: mpsc::Receiver<ToCon>,
        query_listener: mpsc::Sender<FromConQuery>,
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
        let mut buf = vec![0u8; 2 * MTU];

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

    async fn send(&mut self, command: ToCon) {
        let (message, target) = match command {
            ToCon::Query {
                resp_returner,
                target,
                variant,
            } => {
                let message = variant.into_message(&self.own_id);
                self.pending
                    .insert(message.tid().as_bytes(), (&message).into(), resp_returner);

                (message, target)
            }
            ToCon::Resp { query_id, variant } => {
                let token = self.secret.hash_as_bytes(&query_id.target.ip().octets());
                let message = variant.into_message(&self.own_id, query_id.transaction_id, token);

                (message, query_id.target)
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

        let message = match Message::from_bytes(buf) {
            Ok(message) => message,
            Err(e) => {
                error!("can't parse e = {:?}, buf = {:?}", e, buf);
                return;
            }
        };

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
                let pong = ToCon::Resp {
                    query_id: QueryId {
                        target: from,
                        transaction_id,
                    },
                    variant: ToConResp::Ping,
                };

                self.send(pong).await;
            }
            Arguments::FindNode { target, .. } => {
                self.query_listener
                    .send(FromConQuery::FindNode {
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
                    .send(FromConQuery::GetPeers {
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
                        .send(FromConQuery::NewPeer {
                            new_peer: Peer::new(peer_addr),
                            info_hash,
                        })
                        .await
                        .unwrap();

                    let resp = ToCon::Resp {
                        query_id: QueryId {
                            target: from,
                            transaction_id,
                        },
                        variant: ToConResp::AnnouncePeer,
                    };

                    self.send(resp).await;
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
                    .send(FromConResp::Touch { id, from })
                    .await
                    .unwrap();
            }
            Response::FindNode { nodes, id } => {
                querying_task_channel
                    .send(FromConResp::NewNodes { nodes, from, id })
                    .await
                    .unwrap();
            }
            Response::GetPeers {
                values_or_nodes,
                token,
                ..
            } => {
                querying_task_channel
                    .send(FromConResp::GetPeers {
                        from,
                        token,
                        v_or_n: values_or_nodes,
                    })
                    .await
                    .unwrap();
            }
            Response::AnnouncePeer { id } => {
                querying_task_channel
                    .send(FromConResp::Touch { id, from })
                    .await
                    .unwrap();
            }
        }
    }
}
