use super::{
    connection_handle::UnchokeWaiter,
    connection_manager::{ConMessageType, ConnectionMessage, BLOCK_SIZE},
    message::Piece,
    pending_pieces::{AddBlockRes, PendingPieces},
};
use crate::{
    data_structures::ID,
    peers::{connection::message::Message, peer::Peer},
    shutdown,
    transcoding::metainfo::Torrent,
};
use anyhow::{bail, Result};
use std::{collections::HashMap, time::Duration};
use tokio::{select, sync::mpsc, time::sleep};
use tracing::warn;

const ACTIVE_BLOCKS_PER_PEER: usize = 16;

#[derive(Debug)]
pub enum PeerManagerCommand {
    StartPiece((ID, usize)),
    StopPiece(usize),
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum BlockStatus {
    Queued,
    Requested,
}

#[derive(Debug, Clone, Copy)]
pub struct BlockAddress {
    pub piece_idx: usize,
    pub block_idx: usize,
    status: BlockStatus,
}

impl BlockAddress {
    pub fn new(piece_idx: usize, block_idx: usize) -> Self {
        Self {
            piece_idx,
            block_idx,
            status: BlockStatus::Queued,
        }
    }
}

pub fn spawn_piece_downloader(
    peer: Peer,
    send_tx: mpsc::Sender<Message>,
    finished_piece_tx: mpsc::Sender<ConnectionMessage>,
    downloaded_block_rx: mpsc::Receiver<Piece>,
    new_piece_rx: mpsc::Receiver<PeerManagerCommand>,
    pending_pieces: PendingPieces,
    unchoke_waiter: UnchokeWaiter,
    shutdown_rx: shutdown::Receiver,
) {
    tokio::spawn(async move {
        manage_piece_downloader(
            peer,
            send_tx,
            finished_piece_tx,
            downloaded_block_rx,
            new_piece_rx,
            pending_pieces,
            unchoke_waiter,
            shutdown_rx,
        )
        .await;
    });
}

async fn manage_piece_downloader(
    peer: Peer,
    send_tx: mpsc::Sender<Message>,
    finished_piece_tx: mpsc::Sender<ConnectionMessage>,
    mut downloaded_block_rx: mpsc::Receiver<Piece>,
    mut new_piece_rx: mpsc::Receiver<PeerManagerCommand>,
    mut pending_pieces: PendingPieces,
    mut unchoke_waiter: UnchokeWaiter,
    mut shutdown_rx: shutdown::Receiver,
) {
    let mut current_pieces = HashMap::new();
    let mut current_blocks = Vec::with_capacity(ACTIVE_BLOCKS_PER_PEER);

    let mut you_dont_need_to_lock_that_mutex_every_time_counter = 0;

    loop {
        select! {
            _ = shutdown_rx.recv() => { return },
            block = downloaded_block_rx.recv() => {
                foo(peer, block.unwrap(), &mut pending_pieces, &mut current_blocks, &current_pieces, &finished_piece_tx).await;
            },
            command = new_piece_rx.recv() => {
                match command{
                    Some(command) => match command{
                        PeerManagerCommand::StartPiece((piece_hash, piece_idx)) => {current_pieces.insert(piece_idx, piece_hash);},
                        PeerManagerCommand::StopPiece(piece_idx) => {current_pieces.remove(&piece_idx);},
                    },
                    None => todo!(),
                }
            },
        };

        if you_dont_need_to_lock_that_mutex_every_time_counter % (ACTIVE_BLOCKS_PER_PEER / 3) == 0 {
            pending_pieces.take_not_good_blocks(
                current_pieces.keys().into_iter().cloned().collect(),
                &mut current_blocks,
            );
        }

        for queued_block in current_blocks
            .iter_mut()
            .filter(|block| block.status == BlockStatus::Queued)
        {
            if let Err(e) = send_tx.send(Message::Request((*queued_block).into())).await {
                warn!(?e);
            }

            sleep(Duration::from_secs(10)).await;

            queued_block.status = BlockStatus::Requested;
        }

        you_dont_need_to_lock_that_mutex_every_time_counter += 1;
    }
}

async fn foo(
    peer: Peer,
    block: Piece,
    pending_pieces: &mut PendingPieces,
    current_blocks: &mut Vec<BlockAddress>,
    current_pieces: &HashMap<usize, ID>,
    finished_piece_tx: &mpsc::Sender<ConnectionMessage>,
) -> Result<()> {
    if block.begin as usize % BLOCK_SIZE > 0 {
        bail!("piece.begin as usize % BLOCK_SIZE > 0");
    }

    let piece_idx = block.index as usize;
    let Some(piece_hash) = current_pieces.get(&piece_idx) else {
            bail!("current_pieces.get(&piece_idx) == None");
        };

    let block_idx = block.begin as usize / BLOCK_SIZE;

    let Some(idx) = current_blocks.iter().position(|block_addr| block_addr.block_idx == block_idx ) else {
            bail!("current_blocks.iter().position(|block_addr| block_addr.block_idx == block_idx ) == None");
        };
    let mut removed_block_addr = current_blocks.remove(idx);

    match pending_pieces.add_block_data(piece_idx, block_idx, block.block.into_vec().into()) {
        AddBlockRes::Added => todo!(),
        AddBlockRes::Failed => {
            removed_block_addr.status = BlockStatus::Queued;
            current_blocks.push(removed_block_addr);
        }
        AddBlockRes::AddedLast(finished_piece) => {
            if Torrent::piece_hash(&finished_piece.data) != *piece_hash {
                pending_pieces.add_piece(piece_idx);
                bail!("Torrent::piece_hash(&finished_piece.data) != *piece_hash");
            }

            finished_piece_tx
                .send(ConnectionMessage {
                    peer,
                    message: ConMessageType::FinishedPiece(finished_piece),
                })
                .await;
        }
    }

    Ok(())
}
