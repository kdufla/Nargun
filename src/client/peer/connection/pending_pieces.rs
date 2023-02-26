use super::BLOCK_SIZE;
use crate::client::peer::BlockAddress;
use crate::client::Peer;

use crate::unsigned_ceil_div;
use anyhow::{anyhow, bail, Result};
use rand::seq::SliceRandom;
use rand::thread_rng;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use tokio::time::Instant;
use tracing::error;

const MAX_PIECES_DOWNLOADING: usize = 10;
const MAINLINE_TCP_REQUEST_TIMEOUT_SECS: u64 = 7;

#[derive(Clone)]
pub struct PendingPieces {
    piece_length: usize,
    blocks_per_piece: usize,
    copy_owner: Option<Peer>,
    data: Arc<StdMutex<HashMap<usize, Piece>>>,
}

#[derive(Debug)]
pub struct FinishedPiece {
    pub idx: usize,
    pub data: Vec<Vec<u8>>,
}

struct Piece {
    pending: usize,
    downloaded: usize,
    data: Vec<Block>,
}

#[derive(Clone)]
struct PendingBlockDesc {
    target: Peer,
    time_sent: Instant,
}

#[derive(Clone)]
enum Block {
    Unbegun,
    Pending(PendingBlockDesc),
    Downloaded(Vec<u8>),
}

pub enum AddBlockRes {
    Failed,
    Added,
    AddedLast(FinishedPiece),
}

impl PendingPieces {
    pub fn new(piece_length: usize) -> Self {
        Self {
            piece_length,
            blocks_per_piece: unsigned_ceil_div!(piece_length, BLOCK_SIZE),
            copy_owner: None,
            data: Arc::new(StdMutex::new(HashMap::with_capacity(
                MAX_PIECES_DOWNLOADING,
            ))),
        }
    }

    pub fn set_owner_peer(&mut self, owner: Peer) {
        self.copy_owner = Some(owner);
    }

    pub fn add_piece(&mut self, idx: usize) -> Result<()> {
        let mut data = self.data.lock().unwrap();

        if data.contains_key(&idx) {
            return Ok(());
        }

        if data.len() >= MAX_PIECES_DOWNLOADING {
            bail!("Pending is full");
        }

        data.insert(idx, Piece::new(self.piece_length));

        Ok(())
    }

    pub fn add_block_data(
        &mut self,
        piece_idx: usize,
        block_idx: usize,
        block_data: Vec<u8>,
    ) -> AddBlockRes {
        if let Err(e) = self.check_block_bounds(block_idx, &block_data) {
            error!(?e);
            return AddBlockRes::Failed;
        };

        let mut data = self.data.lock().unwrap();

        let Some(piece) = data.get_mut(&piece_idx) else {
            error!("trying to store a block of unknown piece {}", piece_idx);
            return AddBlockRes::Failed;
        };

        let block = &mut piece.data[block_idx];

        *block = Block::Downloaded(block_data);
        piece.downloaded += 1;
        piece.pending -= 1;

        if self.blocks_per_piece == piece.downloaded {
            let finished_piece = data.remove(&piece_idx).unwrap();
            AddBlockRes::AddedLast(FinishedPiece {
                data: finished_piece.data.into_iter().map(|b| b.into()).collect(),
                idx: piece_idx,
            })
        } else {
            AddBlockRes::Added
        }
    }

    // TODO this should be used, you probably forgot it. remove this # when you find its place.
    #[allow(dead_code)]
    pub fn cancel_pending(&mut self, from: &Peer) {
        let mut data = self.data.lock().unwrap();

        for (_, piece) in data.iter_mut() {
            for block in piece.data.iter_mut() {
                if block.is_expected_from(from) {
                    *block = Block::Unbegun;
                }
            }
        }
    }

    pub fn take_not_good_blocks(&self, mut pieces: Vec<usize>, block_buf: &mut Vec<BlockAddress>) {
        let mut rng = thread_rng();
        pieces.shuffle(&mut rng);

        let filters = [
            |block: &Block| block.is(&Block::Unbegun),
            |block: &Block| block.is_expired_pending(),
        ];

        let mut data = self.data.lock().unwrap();

        let now = Instant::now();

        for filter in filters {
            for (block, block_address) in
                Self::filtered_iter_over_blocks(&pieces, &mut data, filter)
            {
                *block = Block::Pending(PendingBlockDesc {
                    target: self.copy_owner.unwrap(),
                    time_sent: now,
                });

                block_buf.push(block_address);
            }

            if block_buf.capacity() == block_buf.len() {
                break;
            }
        }
    }

    fn filtered_iter_over_blocks<'a>(
        piece_indexes: &'a [usize],
        data: &'a mut HashMap<usize, Piece>,
        filter_closure: fn(&Block) -> bool,
    ) -> impl Iterator<Item = (&'a mut Block, BlockAddress)> + 'a {
        data.iter_mut()
            .filter_map(move |(piece_idx, piece)| {
                piece_indexes.contains(piece_idx).then_some(
                    piece
                        .data
                        .iter_mut()
                        .enumerate()
                        .filter_map(move |(block_idx, block)| {
                            filter_closure(block)
                                .then_some((block, BlockAddress::new(*piece_idx, block_idx)))
                        }),
                )
            })
            .flatten()
    }

    fn check_block_bounds(&self, block_idx: usize, data: &[u8]) -> Result<()> {
        if block_idx >= self.blocks_per_piece {
            bail!(
                "block index ({}) out of bounds ({})",
                block_idx,
                self.blocks_per_piece
            );
        }

        let last_block_in_piece = block_idx + 1 == self.blocks_per_piece;

        let block_size = if last_block_in_piece {
            self.piece_length % BLOCK_SIZE
        } else {
            BLOCK_SIZE
        };

        match block_size.cmp(&data.len()) {
            Ordering::Greater => Err(anyhow!(
                "block is too small. expected {}, got {}",
                block_size,
                data.len()
            )),
            Ordering::Less => Err(anyhow!(
                "block is too large. expected {}, got {}",
                block_size,
                data.len()
            )),
            Ordering::Equal => Ok(()),
        }
    }
}

impl Piece {
    fn new(size: usize) -> Self {
        let piece_count = unsigned_ceil_div!(size, BLOCK_SIZE);
        let data = vec![Block::default(); piece_count];

        Self {
            pending: 0,
            downloaded: 0,
            data,
        }
    }
}

impl Default for Block {
    fn default() -> Self {
        Self::Unbegun
    }
}

impl Block {
    fn is(&self, v: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(v)
    }

    fn is_expired_pending(&self) -> bool {
        let Self::Pending(description) = self else {
            return false;
        };

        description.time_sent.elapsed() > Duration::from_secs(MAINLINE_TCP_REQUEST_TIMEOUT_SECS)
    }

    fn is_expected_from(&self, target: &Peer) -> bool {
        let Self::Pending(description) = self else {
            return false;
        };

        &description.target == target
    }
}

impl From<Block> for Vec<u8> {
    fn from(block: Block) -> Self {
        match block {
            Block::Downloaded(data) => data,
            _ => panic!("block must be Block::Downloaded"),
        }
    }
}
