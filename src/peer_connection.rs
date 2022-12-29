pub mod command;
pub mod handshake;
pub mod info;
pub mod pending_pieces;

use self::command::{Command, CommandType};
use self::handshake::initiate_handshake;
use self::info::PeerConnectionInfo;
use self::pending_pieces::PendingPieces;
use crate::constants::{BLOCK_SIZE, KEEP_ALIVE_INTERVAL_SECS, MAX_CONCURRENT_REQUESTS};
use crate::peer_message::{Message, Request};
use crate::unsigned_ceil_div;
use crate::util::{bitmap::Bitmap, id::ID};
use anyhow::Result;
use std::net::SocketAddr;
use tokio::io::{split, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::select;
use tokio::sync::mpsc;
use tokio::time::{interval, Duration};

#[derive(Clone)]
pub struct PeerConnection {
    peer_id: ID,
    socket_addr: SocketAddr,
    info_hash: ID,
    connection_info: PeerConnectionInfo,
    pieces: Bitmap,
    piece_length: u32,
    pub command_sender: mpsc::Sender<Command>,
}

impl PeerConnection {
    pub fn new(
        peer_id: ID,
        socket_addr: SocketAddr,
        info_hash: ID,
        connection_info: PeerConnectionInfo,
        piece_length: u32,
        pieces: Bitmap,
        command_sender: mpsc::Sender<Command>,
        command_receiver: mpsc::Receiver<Command>,
    ) -> Self {
        let connection = Self {
            peer_id,
            socket_addr,
            info_hash,
            connection_info,
            pieces,
            piece_length,
            command_sender,
        };

        let conn = connection.clone();

        tokio::spawn(async move {
            conn.manage_tcp(command_receiver).await;
        });

        connection
    }

    pub fn is_active(&self) -> bool {
        self.connection_info.connection_is_active()
    }

    async fn manage_tcp(self, command_receiver: mpsc::Receiver<Command>) {
        match initiate_handshake(&self.socket_addr, &self.info_hash, &self.peer_id).await {
            Ok(mut stream) => {
                let (read_stream, write_stream) = split(&mut stream);

                let pending_pieces = PendingPieces::new();

                if let Err(e) = select! {
                    rv = self.manager_sender(write_stream, pending_pieces.clone(), command_receiver) => {rv},
                    rv = self.manager_receiver( read_stream, pending_pieces.clone()) => {rv},
                } {
                    println!("error in manage_tcp: {:?}", e);
                    self.connection_info.set_connection_is_active(false);
                    pending_pieces.cancel_pending();
                }
            }
            Err(_) => {
                self.connection_info.set_connection_is_active(false);
            }
        }
    }

    async fn manager_sender(
        &self,
        mut stream: WriteHalf<&mut TcpStream>,
        mut pending_pieces: PendingPieces,
        mut command_receiver: mpsc::Receiver<Command>,
    ) -> Result<()> {
        let mut interval = interval(Duration::from_secs(KEEP_ALIVE_INTERVAL_SECS));

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    self.send_keep_alive(&mut stream).await?;
                    // TODO handle error
                },
                command_option = command_receiver.recv() => {
                    if let Some(command) = command_option{
                        self.handle_command(command, &mut pending_pieces)?;
                        self.send_requests_if_possible(&pending_pieces, &mut stream).await?;
                    }
                    // TODO handle error
                }
            };
        }
    }

    async fn send_requests_if_possible(
        &self,
        pending_pieces: &PendingPieces,
        stream: &mut WriteHalf<&mut TcpStream>,
    ) -> Result<()> {
        if self.connection_info.peer_choking() {
            return Ok(());
        }

        let pending_chunks = pending_pieces.count_pending();
        let free_request_spots = if MAX_CONCURRENT_REQUESTS > pending_chunks {
            (MAX_CONCURRENT_REQUESTS - pending_chunks) as usize
        } else {
            0
        };

        if let Some(unbeguns) = pending_pieces.get_random_unbeguns(free_request_spots) {
            for (piece_idx, chunk_idx) in unbeguns {
                if let Ok(_) = self.send_request(stream, piece_idx, chunk_idx).await {
                    pending_pieces.chunk_requested(piece_idx, chunk_idx)?;
                }
            }
        }

        Ok(())
    }

    fn handle_command(&self, command: Command, pending_pieces: &mut PendingPieces) -> Result<()> {
        match command.command {
            CommandType::Request(piece_idx) => {
                pending_pieces.add_piece(piece_idx, self.piece_length, command.response_channel);
            }
            CommandType::Finished => {}
        };
        Ok(())
    }

    async fn manager_receiver(
        &self,
        mut stream: ReadHalf<&mut TcpStream>,
        mut pending_pieces: PendingPieces,
    ) -> Result<()> {
        loop {
            let message = Message::from_stream(&mut stream).await?;

            match message {
                Message::KeepAlive => (), // TODO impl timeout
                Message::Choke => {
                    self.connection_info.set_peer_choking(true);
                    pending_pieces.cancel_pending();
                }
                Message::Unchoke => {
                    self.connection_info.set_peer_choking(false);
                    let (finished_command, _) = Command::new(CommandType::Finished);
                    self.command_sender.send(finished_command).await?;
                    // TODO better way to trigger sender
                }
                Message::Interested => self.connection_info.set_peer_interested(true),
                Message::NotInterested => self.connection_info.set_peer_interested(false),
                Message::Have(piece_index) => self.pieces.set(piece_index as usize, true),
                Message::Bitfield(bitfield) => {
                    // TODO handle error
                    self.pieces.replace_data(&bitfield.into_bytes())?;
                }
                Message::Request(_request) => (), // TODO upload
                Message::Piece(mut piece) => {
                    if pending_pieces.store_chunk_get_remaining(&mut piece)? == 0 {
                        if let Some(tx) = pending_pieces.remove(piece.index) {
                            println!("save to disk!");
                            // TODO save on disk
                            if let Err(e) = tx.send(69) {
                                anyhow::bail!("piece finish callback not working: {}", e);
                            }
                        }
                    }

                    let (finished_command, _) = Command::new(CommandType::Finished);
                    self.command_sender.send(finished_command).await?;
                }
                Message::Port(_listen_port) => {}
            };
        }
    }

    pub async fn send_request(
        &self,
        stream: &mut WriteHalf<&mut TcpStream>,
        index: u32,
        chunk_idx: u32,
    ) -> Result<()> {
        let chunk_count = unsigned_ceil_div!(self.piece_length, BLOCK_SIZE);

        let request = Message::Request(Request {
            index,
            begin: chunk_idx * BLOCK_SIZE,
            length: if chunk_idx == chunk_count - 1 && self.piece_length % BLOCK_SIZE != 0 {
                self.piece_length % BLOCK_SIZE
            } else {
                BLOCK_SIZE
            },
        })
        .into_bytes()?;

        stream.write_all(request.as_ref()).await?;

        Ok(())
    }

    async fn send_keep_alive(&self, stream: &mut WriteHalf<&mut TcpStream>) -> Result<()> {
        stream
            .write_all(&Message::KeepAlive.into_bytes().unwrap())
            .await?;

        Ok(())
    }

    // async fn send_choke(&mut self) -> Result<()> {
    //     self.stream
    //         .write_all(&Message::Choke.into_bytes().unwrap())
    //         .await?;

    //     Ok(())
    // }

    // async fn send_unchoke(&mut self) -> Result<()> {
    //     self.stream
    //         .write_all(&Message::Unchoke.into_bytes().unwrap())
    //         .await?;

    //     Ok(())
    // }

    // async fn send_interested(&mut self) -> Result<()> {
    //     self.stream
    //         .write_all(&Message::Interested.into_bytes().unwrap())
    //         .await?;

    //     Ok(())
    // }

    // async fn send_not_interested(&mut self) -> Result<()> {
    //     self.stream
    //         .write_all(&Message::NotInterested.into_bytes().unwrap())
    //         .await?;

    //     Ok(())
    // }

    // async fn send_have(&mut self, piece_index: u32) -> Result<()> {
    //     self.stream
    //         .write_all(&Message::Have(piece_index).into_bytes().unwrap())
    //         .await?;

    //     Ok(())
    // }

    // async fn send_bitfield(&mut self, bitfield: Bytes) -> Result<()> {
    //     self.stream
    //         .write_all(
    //             &Message::Bitfield(SerializableBytes::new(bitfield))
    //                 .into_bytes()
    //                 .unwrap(),
    //         )
    //         .await?;

    //     Ok(())
    // }
}
