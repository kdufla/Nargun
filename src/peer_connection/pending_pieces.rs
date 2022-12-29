use anyhow::{anyhow, Result};
use bytes::Bytes;
use rand::Rng;
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::oneshot;

use crate::constants::BLOCK_SIZE;
use crate::peer_message::Piece;
use crate::unsigned_ceil_div;

// TODO this is shit
#[derive(Clone)]
pub struct PendingPieces {
    data: Arc<
        StdMutex<(
            usize,
            Vec<u32>,
            HashMap<u32, (Chunks, oneshot::Sender<u64>)>,
        )>,
    >,
}

impl PendingPieces {
    pub fn new() -> Self {
        Self {
            data: Arc::new(StdMutex::new((0, Vec::new(), HashMap::new()))),
        }
    }

    pub fn add_piece(
        &mut self,
        idx: u32,
        piece_length: u32,
        finish_callback: oneshot::Sender<u64>,
    ) {
        let chunk_count = unsigned_ceil_div!(piece_length, BLOCK_SIZE);
        let chunks = Chunks::new(chunk_count as usize);

        let mut data = self.data.lock().unwrap();
        data.1.push(idx);
        data.2.insert(idx, (chunks, finish_callback));
    }

    pub fn get_random_unbeguns(&self, n: usize) -> Option<Vec<(u32, u32)>> {
        let mut data = self.data.lock().unwrap();

        for _ in 0..data.1.len() {
            let idx = data.0 % data.1.len();

            data.0 = data.0.wrapping_add(1);

            let p_idx = data.1[idx];

            if let Some(chunks) = data.2.get(&p_idx) {
                if let Some(c_idxs) = chunks.0.get_unbegun(n) {
                    return Some(c_idxs.iter().map(|c_idx| (p_idx, *c_idx)).collect());
                }
            }
        }

        None
    }

    pub fn remove(&mut self, idx: u32) -> Option<oneshot::Sender<u64>> {
        let mut data = self.data.lock().unwrap();

        let mut idx_in_vec = None;

        for (i, elem) in data.1.iter().enumerate() {
            if *elem == idx {
                idx_in_vec = Some(i);
                break;
            }
        }

        match idx_in_vec {
            Some(i) => {
                data.1.remove(i);
            }
            None => return None,
        }

        match data.2.remove(&idx) {
            Some((_, tx)) => Some(tx),
            None => None,
        }
    }

    pub fn count_pending(&self) -> u32 {
        self.data
            .lock()
            .unwrap()
            .2
            .iter()
            .map(|kv| kv.1 .0.count_pending())
            .sum()
    }

    pub fn chunk_requested(&self, piece_idx: u32, chunk_idx: u32) -> Result<()> {
        match self.data.lock().unwrap().2.get(&piece_idx) {
            Some(chunks) => {
                chunks.0.set_pending(chunk_idx as usize);
                Ok(())
            }
            None => Err(anyhow!("piece {} is not being downloaded", piece_idx)),
        }
    }

    pub fn store_chunk_get_remaining(&self, piece: &mut Piece) -> Result<u32> {
        let pieces = &self.data.lock().unwrap().2;

        if pieces.contains_key(&piece.index) && piece.begin % BLOCK_SIZE == 0 {
            let chunks = pieces.get(&piece.index).unwrap();
            let block_idx = (piece.begin / BLOCK_SIZE) as usize;

            println!(
                "chunks.0.len()={}, block_idx={}, chunks.0.is_pending(block_idx)={}",
                chunks.0.len(),
                block_idx,
                chunks.0.is_pending(block_idx)
            );

            if chunks.0.len() > block_idx && chunks.0.is_pending(block_idx) {
                chunks.0.set_downloaded(block_idx, piece.block.as_bytes());

                return Ok(chunks.0.remaining());
            }
        }

        Err(anyhow!(
            "chunk not saved: index={:?}, begin={:?}",
            piece.index,
            piece.begin
        ))
    }

    pub fn cancel_pending(&self) {
        for kv in self.data.lock().unwrap().2.iter() {
            kv.1 .0.cancel_pending();
        }
    }
}

#[derive(Clone, PartialEq)]
enum ChunkDownStatus {
    Unbegun,
    Pending,
    Downloaded(Bytes),
}

struct Chunks {
    data: Arc<StdMutex<Vec<ChunkDownStatus>>>,
}

impl Chunks {
    fn new(n: usize) -> Self {
        Self {
            data: Arc::new(std::sync::Mutex::new(vec![ChunkDownStatus::Unbegun; n])),
        }
    }

    fn len(&self) -> usize {
        self.data.lock().unwrap().len()
    }

    fn get_unbegun(&self, n: usize) -> Option<Vec<u32>> {
        if n == 0 {
            return None;
        }

        let data = self.data.lock().unwrap();

        if data
            .iter()
            .filter(|chunk| ChunkDownStatus::Unbegun == **chunk)
            .fold(0, |c, _| c + 1)
            == 0
        {
            None
        } else {
            let mut rng = rand::thread_rng();
            let rand_mid_idx = rng.gen_range(0..data.len());
            let (left, right) = data.split_at(rand_mid_idx);

            let mut count_unbegun = 0;

            let mut unbegun_idxs = Vec::with_capacity(n);

            for (i, chunk) in right.iter().enumerate() {
                if let ChunkDownStatus::Unbegun = chunk {
                    unbegun_idxs.push((rand_mid_idx + i) as u32);
                    count_unbegun += 1;

                    if count_unbegun == n {
                        break;
                    }
                }
            }

            if count_unbegun != n {
                for (i, chunk) in left.iter().enumerate() {
                    if let ChunkDownStatus::Unbegun = chunk {
                        unbegun_idxs.push(i as u32);
                        count_unbegun += 1;

                        if count_unbegun == n {
                            break;
                        }
                    }
                }
            }

            if count_unbegun == 0 {
                None
            } else {
                Some(unbegun_idxs)
            }
        }
    }

    fn count_pending(&self) -> u32 {
        self.data
            .lock()
            .unwrap()
            .iter()
            .filter(|chunk| ChunkDownStatus::Pending == **chunk)
            .fold(0, |c, _| c + 1)
    }

    fn remaining(&self) -> u32 {
        let chunks = self.data.lock().unwrap();

        chunks
            .iter()
            .filter(|chunk| ChunkDownStatus::Unbegun == **chunk)
            .fold(0, |c, _| c + 1)
            + chunks
                .iter()
                .filter(|chunk| ChunkDownStatus::Pending == **chunk)
                .fold(0, |c, _| c + 1)
    }

    fn cancel_pending(&self) {
        let mut chunks = self.data.lock().unwrap();

        for chunk_idx in 0..chunks.len() {
            if chunks[chunk_idx] == ChunkDownStatus::Pending {
                chunks[chunk_idx] = ChunkDownStatus::Unbegun;
            }
        }
    }

    // fn set_unbegun(&self, idx: usize) {
    //     self.data.lock().unwrap()[idx] = ChunkDownStatus::Unbegun;
    // }

    fn set_pending(&self, idx: usize) {
        self.data.lock().unwrap()[idx] = ChunkDownStatus::Pending;
    }

    fn is_pending(&self, idx: usize) -> bool {
        self.data.lock().unwrap()[idx] == ChunkDownStatus::Pending
    }

    fn set_downloaded(&self, idx: usize, data: Bytes) {
        self.data.lock().unwrap()[idx] = ChunkDownStatus::Downloaded(data);
    }
}
