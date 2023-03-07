use super::file_cache::FileCache;
use super::Fs;
use crate::peers::connection::connection_manager::BLOCK_SIZE;
use crate::peers::connection::pending_pieces::FinishedPiece;
use crate::transcoding::metainfo::Info;
use anyhow::{bail, Result};
use async_trait::async_trait;
use std::io::SeekFrom;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

pub struct SingleFileRw {
    torrent_name: String,
    files: FileCache,
    info: Info,
}

impl SingleFileRw {
    pub fn new(base_dir: &str, info: Info) -> Self {
        Self {
            torrent_name: format!("{}/{}", base_dir, &info.name),
            files: FileCache::new(),
            info,
        }
    }
}

#[async_trait]
impl Fs for SingleFileRw {
    async fn read_block(
        &mut self,
        index: usize,
        begin: usize,
        length: usize,
        buf: &mut [u8],
    ) -> Result<()> {
        let piece_start = index * self.info.piece_length as usize;
        let block_start = piece_start + begin;

        let file = self.files.get_read(&self.torrent_name).await?;

        file.seek(std::io::SeekFrom::Start(block_start as u64))
            .await?;

        let data_buf = &mut buf[..length];

        file.read_exact(data_buf).await?;

        Ok(())
    }

    async fn write_piece(&mut self, piece: FinishedPiece) -> Result<()> {
        match piece.idx {
            idx if idx < self.info.pieces.len() - 1 => {
                self.write_middle_piece_single_file(piece).await?
            }
            idx if idx == self.info.pieces.len() - 1 => {
                self.write_last_piece_single_file(piece).await?
            }
            _ => {
                bail!("piece {} is out of bounds", piece.idx)
            }
        }

        Ok(())
    }
}

impl SingleFileRw {
    async fn write_middle_piece_single_file(&mut self, piece: FinishedPiece) -> Result<()> {
        let start = piece.idx * self.info.piece_length as usize;

        let file = self.files.get_write(&self.torrent_name).await?;
        file.seek(SeekFrom::Start(start as u64)).await?;

        for block in piece.data {
            file.write_all(&block).await?;
        }

        Ok(())
    }

    async fn write_last_piece_single_file(&mut self, piece: FinishedPiece) -> Result<()> {
        let start = piece.idx * self.info.piece_length as usize;

        let file = self.files.get_write(&self.torrent_name).await?;
        file.seek(SeekFrom::Start(start as u64)).await?;

        let mut remaining_bytes = (self.info.length() % self.info.piece_length) as usize;
        let mut block_iter = piece.data.iter();

        while remaining_bytes > 0 {
            let bytes_to_write = std::cmp::min(remaining_bytes, BLOCK_SIZE);

            let Some(block) = block_iter.next() else {
                bail!(
                    "last piece (idx={}) has less data ({} blocks) than expected ({} bytes)",
                    piece.idx,
                    piece.data.len(),
                    self.info.length() % self.info.piece_length
                );
            };

            file.write_all(&block[..bytes_to_write]).await?;

            remaining_bytes -= bytes_to_write;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::SingleFileRw;
    use crate::fs::Fs;
    use crate::peers::connection::connection_manager::BLOCK_SIZE;
    use crate::peers::connection::pending_pieces::FinishedPiece;
    use crate::transcoding::metainfo::Torrent;
    use rand::seq::IteratorRandom;
    use tokio::fs::{remove_dir_all, DirBuilder};
    use tracing_test::traced_test;

    const PATH: &str = "single_test";

    #[traced_test]
    #[tokio::test]
    async fn write_then_read_single() {
        DirBuilder::new()
            .recursive(true)
            .create(PATH)
            .await
            .unwrap();

        let (data, torrent) = Torrent::mock_single();

        let fs = SingleFileRw::new(PATH, torrent.info.clone());

        write_then_read(data, &torrent, fs).await;

        remove_dir_all(PATH).await.unwrap();
    }

    async fn write_then_read<'a>(
        data: Vec<Vec<Vec<u8>>>,
        torrent: &'a Torrent,
        mut fs: SingleFileRw,
    ) {
        write(&data, &mut fs).await;
        read(&data, torrent, &mut fs).await;
    }

    async fn write<'a>(data: &[Vec<Vec<u8>>], fs: &mut SingleFileRw) {
        for (idx, data) in data
            .iter()
            .cloned()
            .enumerate()
            .choose_multiple(&mut rand::thread_rng(), data.len())
        {
            fs.write_piece(FinishedPiece { idx, data }).await.unwrap();
        }
    }

    async fn read<'a>(data: &[Vec<Vec<u8>>], torrent: &'a Torrent, fs: &mut SingleFileRw) {
        let mut buf = Box::new([0u8; BLOCK_SIZE]);

        read_full_pieces(data, fs, buf.as_mut_slice()).await;

        read_full_blocks_in_last_piece(data, torrent, fs, buf.as_mut_slice()).await;

        read_partial_block_in_last_piece(data, torrent, fs, buf.as_mut_slice()).await;
    }

    async fn read_full_pieces<'a>(data: &[Vec<Vec<u8>>], fs: &mut SingleFileRw, buf: &mut [u8]) {
        for (piece_idx, piece) in data[..data.len() - 1].iter().enumerate() {
            for (block_idx, block) in piece.iter().enumerate() {
                fs.read_block(piece_idx, block_idx * BLOCK_SIZE, BLOCK_SIZE, buf)
                    .await
                    .unwrap();

                assert_eq!(buf, block.as_slice());
            }
        }
    }

    async fn read_full_blocks_in_last_piece<'a>(
        data: &[Vec<Vec<u8>>],
        torrent: &'a Torrent,
        fs: &mut SingleFileRw,
        buf: &mut [u8],
    ) {
        let data_in_last_piece = (torrent.info.length() % torrent.info.piece_length) as usize;
        let last_block_with_data = data_in_last_piece / BLOCK_SIZE;

        let last_piece_idx = data.len() - 1;
        let last_piece = data.last().unwrap();

        for (block_idx, block) in last_piece[..last_block_with_data].iter().enumerate() {
            fs.read_block(last_piece_idx, block_idx * BLOCK_SIZE, BLOCK_SIZE, buf)
                .await
                .unwrap();

            assert_eq!(buf, block.as_slice());
        }
    }

    async fn read_partial_block_in_last_piece<'a>(
        data: &[Vec<Vec<u8>>],
        torrent: &'a Torrent,
        fs: &mut SingleFileRw,
        buf: &mut [u8],
    ) {
        let data_in_last_piece = (torrent.info.length() % torrent.info.piece_length) as usize;
        let last_block_with_data = data_in_last_piece / BLOCK_SIZE;
        let data_in_last_block = data_in_last_piece % BLOCK_SIZE;

        let last_piece_idx = data.len() - 1;
        let last_piece = data.last().unwrap();

        buf[data_in_last_block..]
            .iter_mut()
            .for_each(|byte| *byte = 0);

        fs.read_block(
            last_piece_idx,
            last_block_with_data * BLOCK_SIZE,
            data_in_last_block,
            buf,
        )
        .await
        .unwrap();

        let last_block = last_piece.get(last_block_with_data).unwrap();

        assert_eq!(buf, last_block.as_slice());
    }
}
