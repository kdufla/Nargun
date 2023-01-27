use super::BLOCK_SIZE as SUPER_BLOCK_SIZE;
use crate::data_structures::ID;
use crate::unsigned_ceil_div;
use anyhow::{bail, Result};
use rand::seq::SliceRandom;
use rand::thread_rng;
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;

const MAX_PIECES_DOWNLOADING: usize = 10;
const MAINLINE_TCP_REQUEST_TIMEOUT_SECS: u64 = 7;
const BLOCK_SIZE: usize = SUPER_BLOCK_SIZE as usize;

#[derive(Debug)]
pub struct BlockAddress {
    pub piece_idx: usize,
    pub block_idx: usize,
}

#[derive(Clone)]
pub struct PendingPieces {
    piece_length: usize,
    blocks_per_piece: usize,
    finish_callback: mpsc::Sender<FinishedPiece>,
    data: Arc<StdMutex<HashMap<usize, Piece>>>,
}

#[derive(Debug)]
pub struct FinishedPiece {
    pub idx: usize,
    pub data: Vec<Vec<u8>>,
}

struct Piece {
    remaining: usize,
    pending: usize,
    downloaded: usize,
    data: Vec<Block>,
}

#[derive(Clone)]
struct PendingBlockDesc {
    target: ID,
    time_sent: Instant,
}

#[derive(Clone)]
enum Block {
    Unbegun,
    Pending(PendingBlockDesc),
    Downloaded(Vec<u8>),
}

enum Add {
    Added,
    AddedLast(Piece),
}

impl PendingPieces {
    pub fn new(piece_length: usize, finish_callback: mpsc::Sender<FinishedPiece>) -> Self {
        Self {
            piece_length,
            blocks_per_piece: unsigned_ceil_div!(piece_length, BLOCK_SIZE as usize),
            finish_callback,
            data: Arc::new(StdMutex::new(HashMap::with_capacity(
                MAX_PIECES_DOWNLOADING,
            ))),
        }
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
        from: &ID,
    ) -> Result<()> {
        self.check_block_bounds(block_idx, &block_data)?;

        if let Add::AddedLast(finished_piece) =
            self.blocking_add_for_valid_block(piece_idx, block_idx, block_data, from)?
        {
            let tx = self.finish_callback.clone();
            tokio::spawn(async move {
                let _ = tx
                    .send(FinishedPiece {
                        data: finished_piece.data.into_iter().map(|b| b.into()).collect(),
                        idx: piece_idx,
                    })
                    .await;
            });
        }

        Ok(())
    }

    pub fn cancel_pending(&mut self, from: &ID) {
        let mut data = self.data.lock().unwrap();

        for (_, piece) in data.iter_mut() {
            for block in piece.data.iter_mut() {
                if block.is_expected_from(from) {
                    *block = Block::Unbegun;
                }
            }
        }
    }

    pub fn take_not_good_blocks(&self, pieces: &[usize], n: usize) -> Vec<BlockAddress> {
        let mut pieces = pieces.to_owned();
        let mut rng = thread_rng();
        pieces.shuffle(&mut rng);

        let data = self.data.lock().unwrap();

        let iter_over_unbegun =
            Self::filtered_iter_over_blocks(&pieces, &data, |block| block.is(&Block::Unbegun));

        let iter_over_expired =
            Self::filtered_iter_over_blocks(&pieces, &data, |block| block.is_expired_pending());

        iter_over_unbegun.chain(iter_over_expired).take(n).collect()
    }

    // for each piece form the list gets a list of blocks that passes provided closure
    // just read it..  it looks way more complicated than it actually is
    fn filtered_iter_over_blocks<'a>(
        piece_indexes: &'a [usize],
        data: &'a HashMap<usize, Piece>,
        filter_closure: fn(&'a Block) -> bool,
    ) -> impl Iterator<Item = BlockAddress> + 'a {
        piece_indexes
            .iter()
            .filter_map(move |&piece_idx| {
                data.get(&piece_idx).map(|piece| {
                    piece
                        .data
                        .iter()
                        .enumerate()
                        .filter_map(move |(block_idx, block)| {
                            filter_closure(block).then_some(BlockAddress {
                                piece_idx,
                                block_idx,
                            })
                        })
                })
            })
            .flatten()
    }

    fn check_block_bounds(&self, block_idx: usize, data: &Vec<u8>) -> Result<()> {
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

        if block_size > data.len() {
            bail!(
                "block is too small. expected {}, got {}",
                block_size,
                data.len()
            );
        } else if block_size < data.len() {
            bail!(
                "block is too large. expected {}, got {}",
                block_size,
                data.len()
            );
        }

        Ok(())
    }

    fn blocking_add_for_valid_block(
        &mut self,
        piece_idx: usize,
        block_idx: usize,
        block_data: Vec<u8>,
        from: &ID,
    ) -> Result<Add> {
        let mut data = self.data.lock().unwrap();

        let Some(piece) = data.get_mut(&piece_idx) else {
                bail!("piece {} not pending",piece_idx);
            };

        let block = &mut piece.data[block_idx];

        if !block.is_expected_from(from) {
            bail!("unknown source");
        }

        *block = Block::Downloaded(block_data);
        piece.downloaded += 1;
        piece.pending -= 1;

        Ok(if self.blocks_per_piece == piece.downloaded {
            let finished_piece = data.remove(&piece_idx).unwrap();
            Add::AddedLast(finished_piece)
        } else {
            Add::Added
        })
    }
}

impl Piece {
    fn new(size: usize) -> Self {
        let piece_count = unsigned_ceil_div!(size, BLOCK_SIZE as usize);
        let data = vec![Block::default(); piece_count];

        Self {
            remaining: piece_count,
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

    fn is_not_expired(&self) -> bool {
        let Self::Pending(description) = self else {
            return false;
        };

        description.time_sent.elapsed() < Duration::from_secs(MAINLINE_TCP_REQUEST_TIMEOUT_SECS)
    }

    fn is_expected_from(&self, target: &ID) -> bool {
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

struct SingleCycleWrappingIter<'a, T> {
    iter: Box<dyn Iterator<Item = &'a T> + 'a>,
}

impl<'a, T> SingleCycleWrappingIter<'a, T> {
    fn new(data: &'a [T], start: usize) -> Self {
        Self {
            iter: Box::new(data[start..].iter().chain(data[..start].iter())),
        }
    }
}

impl<'a, T> Iterator for SingleCycleWrappingIter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next()
    }
}
