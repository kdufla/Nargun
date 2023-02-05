use super::connection::{
    AddBlockRes, ConMessageType, Connection, ConnectionMessage, Message, PendingPieces,
};
use crate::{
    client::{FinishedPiece, BLOCK_SIZE},
    data_structures::ID,
    transcoding::metainfo::Torrent,
};
use anyhow::{bail, Result};
use std::collections::HashMap;
use tokio::{select, sync::mpsc};

// TODO unwraps, no-text bails... just read/modify all of it

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

pub async fn manage_peer(
    mut new_piece_rx: mpsc::Receiver<PeerManagerCommand>,
    mut download_manager_rx: mpsc::Receiver<ConnectionMessage>,
    mut pending_pieces: PendingPieces,
    finished_piece_tx: mpsc::Sender<FinishedPiece>,
    connection: Connection,
) {
    let mut current_pieces = HashMap::new();
    let mut current_blocks = Vec::with_capacity(ACTIVE_BLOCKS_PER_PEER);

    let mut you_dont_need_to_lock_that_mutex_every_time_counter = 0;

    loop {
        select! {
            message = download_manager_rx.recv() => {
                foo(&mut pending_pieces, &mut current_blocks, &current_pieces, &finished_piece_tx, message.unwrap()).await;
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
            connection
                .send(Message::Request((*queued_block).into()))
                .await;

            queued_block.status = BlockStatus::Requested;
        }

        you_dont_need_to_lock_that_mutex_every_time_counter += 1;
    }
}

async fn foo(
    pending_pieces: &mut PendingPieces,
    current_blocks: &mut Vec<BlockAddress>,
    current_pieces: &HashMap<usize, ID>,
    finished_piece_tx: &mpsc::Sender<FinishedPiece>,
    message: ConnectionMessage,
) -> Result<()> {
    if let ConMessageType::Piece(piece) = message.message {
        if piece.begin as usize % BLOCK_SIZE > 0 {
            bail!("");
        }

        let piece_idx = piece.index as usize;
        let Some(piece_hash) = current_pieces.get(&piece_idx) else {
            bail!("");
        };

        let block_idx = piece.begin as usize / BLOCK_SIZE;

        let Some(idx) = current_blocks.iter().position(|block_addr| block_addr.block_idx == block_idx ) else {
            bail!("");
        };
        let mut removed_block_addr = current_blocks.remove(idx);

        match pending_pieces.add_block_data(piece_idx, block_idx, piece.block.as_bytes().into()) {
            AddBlockRes::Added => todo!(),
            AddBlockRes::Failed => {
                removed_block_addr.status = BlockStatus::Queued;
                current_blocks.push(removed_block_addr);
            }
            AddBlockRes::AddedLast(finished_piece) => {
                if Torrent::piece_hash(&finished_piece.data) != *piece_hash {
                    pending_pieces.add_piece(piece_idx);
                    bail!("");
                }

                finished_piece_tx.send(finished_piece).await;
            }
        };
    }

    Ok(())
}
