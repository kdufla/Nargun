use super::file_cache::FileCache;
use super::multi_file_iter::FilesInBounds;
use crate::peers::connection::connection_manager::BLOCK_SIZE;
use crate::peers::connection::pending_pieces::FinishedPiece;
use crate::transcoding::metainfo::{File as MetainfoFile, Info, Mode};
use anyhow::{bail, Result};
use std::collections::HashSet;
use std::io::SeekFrom;
use tokio::fs::{remove_dir_all, DirBuilder};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

#[allow(dead_code)]
pub struct FS<'a> {
    torrent_name: String,
    info: &'a Info,
    files: FileCache,
}

impl<'a> FS<'a> {
    pub async fn new(base_dir: String, info: &'a Info) -> FS<'a> {
        if let Mode::Multi { files } = &info.mode {
            build_dir_tree(&base_dir, &info.name, files).await.unwrap();
        }

        Self {
            torrent_name: format!("{}/{}", base_dir, info.name),
            info,
            files: FileCache::new(),
        }
    }

    #[allow(dead_code)]
    pub async fn read_block(&mut self, index: usize, begin: usize, length: usize, buf: &mut [u8]) {
        match &self.info.mode {
            Mode::Single { .. } => self.read_block_single_file(index, begin, length, buf).await,
            Mode::Multi { files } => {
                self.read_block_multi_file(files, index, begin, length, buf)
                    .await
            }
        }
        .unwrap();
    }

    #[allow(dead_code)]
    pub async fn write_piece(&mut self, piece: FinishedPiece) {
        match &self.info.mode {
            Mode::Single { .. } => self.write_piece_single_file(piece).await,
            Mode::Multi { files } => self.write_piece_multi_file(piece, files).await,
        }
        .unwrap();
    }

    async fn read_block_single_file(
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

    async fn read_block_multi_file(
        &mut self,
        files: &[MetainfoFile],
        index: usize,
        begin: usize,
        length: usize,
        buf: &mut [u8],
    ) -> Result<()> {
        let piece_start = index * self.info.piece_length as usize;
        let block_start = piece_start + begin;
        let block_end = block_start + length;

        for file_info in FilesInBounds::new(files, block_start, block_end) {
            let file_in_block =
                &mut buf[file_info.start_within_iter_bounds..file_info.end_within_iter_bounds];

            let path = format!("{}/{}", self.torrent_name, file_info.file.path.join("/"));

            let file = self.files.get_write(&path).await?;
            file.seek(std::io::SeekFrom::Start(
                file_info.file_bytes_before_iter as u64,
            ))
            .await?;

            file.read_exact(file_in_block).await?;
        }

        Ok(())
    }

    async fn write_piece_single_file(&mut self, piece: FinishedPiece) -> Result<()> {
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

    async fn write_middle_piece_single_file(&mut self, piece: FinishedPiece) -> Result<()> {
        let start = piece.idx * self.info.piece_length as usize;

        let file = self.files.get_write(&self.torrent_name).await?;
        file.seek(std::io::SeekFrom::Start(start as u64)).await?;

        for block in piece.data {
            file.write_all(&block).await?;
        }

        Ok(())
    }

    async fn write_last_piece_single_file(&mut self, piece: FinishedPiece) -> Result<()> {
        let start = piece.idx * self.info.piece_length as usize;

        let file = self.files.get_write(&self.torrent_name).await?;
        file.seek(std::io::SeekFrom::Start(start as u64)).await?;

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

    async fn write_piece_multi_file(
        &mut self,
        piece: FinishedPiece,
        files: &[MetainfoFile],
    ) -> Result<()> {
        let piece_start = piece.idx * self.info.piece_length as usize;
        let piece_end = (piece.idx + 1) * self.info.piece_length as usize;

        for file_info in FilesInBounds::new(files, piece_start, piece_end) {
            let path = format!("{}/{}", self.torrent_name, file_info.file.path.join("/"));

            self.write_file_chunk_from_multi_file_piece(
                &path,
                &piece.data,
                file_info.file_bytes_before_iter,
                file_info.start_within_iter_bounds,
                file_info.end_within_iter_bounds,
            )
            .await?;
        }

        Ok(())
    }

    async fn write_file_chunk_from_multi_file_piece(
        &mut self,
        path: &str,
        piece_data: &[Vec<u8>],
        start_within_file: usize,
        start_within_piece: usize,
        end_within_piece: usize,
    ) -> Result<()> {
        let file = self.files.get_write(path).await?;
        file.seek(SeekFrom::Start(start_within_file as u64)).await?;

        let length = end_within_piece - start_within_piece;
        let start_within_block = start_within_piece % BLOCK_SIZE;
        let end_within_block = if end_within_piece % BLOCK_SIZE == 0 {
            BLOCK_SIZE // 0 means full block because it ends at BLOCK_SIZE
        } else {
            end_within_piece % BLOCK_SIZE
        };

        let start_block = start_within_piece / BLOCK_SIZE;
        let end_block = ((end_within_piece - 1) / BLOCK_SIZE) + 1;

        let data = &piece_data[start_block..end_block];

        match data {
            [] => bail!("empty write"),
            [file_fits_in_block] => {
                file.write_all(
                    &file_fits_in_block[start_within_block..start_within_block + length],
                )
                .await?;
            }
            [first, full_blocks @ .., last] => {
                file.write_all(&first[start_within_block..]).await?;

                for block in full_blocks {
                    file.write_all(block).await?;
                }

                file.write_all(&last[..end_within_block]).await?;
            }
        }

        Ok(())
    }
}

pub async fn build_dir_tree(
    base_dir: &str,
    torrent_dir: &str,
    files: &[MetainfoFile],
) -> Result<()> {
    let working_dir = format!("{base_dir}/{torrent_dir}/");

    let dirs = dirs_by_asc_depth(&working_dir, files);

    create_dirs(&working_dir, &dirs).await
}

async fn create_dirs(working_dir: &str, dirs: &[String]) -> Result<()> {
    let mut builder = DirBuilder::new();

    builder.recursive(false).create(working_dir).await?;

    for dir in dirs.iter() {
        if let Err(e) = builder.recursive(false).create(dir).await {
            remove_dir_all(working_dir).await?;
            bail!(e);
        }
    }

    Ok(())
}

fn dirs_by_asc_depth(working_dir: &str, files: &[MetainfoFile]) -> Vec<String> {
    let mut dirs = HashSet::new();

    for file in files.iter().filter(|file| file.path.len() > 1) {
        let mut path = "".to_string();

        for dir_name in file.path[..file.path.len() - 1].iter() {
            path.push_str(dir_name);
            path.push('/');
            dirs.insert(path.to_owned());
        }
    }

    let mut dirs: Vec<String> = dirs
        .into_iter()
        .map(|dir| format!("{working_dir}{dir}"))
        .collect();

    dirs.sort_by_cached_key(|s| s.chars().filter(|ch| *ch == '/').count());

    dirs
}

#[cfg(test)]
mod tests {
    use super::FS;
    use crate::peers::connection::connection_manager::BLOCK_SIZE;
    use crate::peers::connection::pending_pieces::FinishedPiece;
    use crate::transcoding::metainfo::Torrent;
    use rand::seq::IteratorRandom;
    use tokio::fs::{remove_dir_all, DirBuilder};
    use tracing_test::traced_test;

    const SINGLE_PATH: &str = "single_test";
    const MULTI_PATH: &str = "multi_test";

    #[tokio::test]
    async fn write_then_read_single() {
        DirBuilder::new()
            .recursive(true)
            .create(SINGLE_PATH)
            .await
            .unwrap();

        let (data, torrent) = Torrent::mock_single();

        let fs = FS::new(SINGLE_PATH.to_string(), &torrent.info).await;

        write_then_read(data, &torrent, fs).await;

        remove_dir_all(SINGLE_PATH).await.unwrap();
    }

    #[tokio::test]
    #[traced_test]
    async fn write_then_read_multi() {
        DirBuilder::new()
            .recursive(true)
            .create(MULTI_PATH)
            .await
            .unwrap();

        let (data, torrent) = Torrent::mock_multi();

        let fs = FS::new(MULTI_PATH.to_string(), &torrent.info).await;

        write_then_read(data, &torrent, fs).await;

        remove_dir_all(MULTI_PATH).await.unwrap();
    }

    async fn write_then_read<'a>(data: Vec<Vec<Vec<u8>>>, torrent: &'a Torrent, mut fs: FS<'a>) {
        write(&data, &mut fs).await;
        read(&data, torrent, &mut fs).await;
    }

    async fn write<'a>(data: &[Vec<Vec<u8>>], fs: &mut FS<'a>) {
        for (idx, data) in data
            .iter()
            .cloned()
            .enumerate()
            .choose_multiple(&mut rand::thread_rng(), data.len())
        {
            fs.write_piece(FinishedPiece { idx, data }).await;
        }
    }

    async fn read<'a>(data: &[Vec<Vec<u8>>], torrent: &'a Torrent, fs: &mut FS<'a>) {
        let mut buf = Box::new([0u8; BLOCK_SIZE]);

        read_full_pieces(data, fs, buf.as_mut_slice()).await;

        read_full_blocks_in_last_piece(data, torrent, fs, buf.as_mut_slice()).await;

        read_partial_block_in_last_piece(data, torrent, fs, buf.as_mut_slice()).await;
    }

    async fn read_full_pieces<'a>(data: &[Vec<Vec<u8>>], fs: &mut FS<'a>, buf: &mut [u8]) {
        for (piece_idx, piece) in data[..data.len() - 1].iter().enumerate() {
            for (block_idx, block) in piece.iter().enumerate() {
                fs.read_block(piece_idx, block_idx * BLOCK_SIZE, BLOCK_SIZE, buf)
                    .await;

                assert_eq!(buf, block.as_slice());
            }
        }
    }

    async fn read_full_blocks_in_last_piece<'a>(
        data: &[Vec<Vec<u8>>],
        torrent: &'a Torrent,
        fs: &mut FS<'a>,
        buf: &mut [u8],
    ) {
        let data_in_last_piece = (torrent.info.length() % torrent.info.piece_length) as usize;
        let last_block_with_data = data_in_last_piece / BLOCK_SIZE;

        let last_piece_idx = data.len() - 1;
        let last_piece = data.last().unwrap();

        for (block_idx, block) in last_piece[..last_block_with_data].iter().enumerate() {
            fs.read_block(last_piece_idx, block_idx * BLOCK_SIZE, BLOCK_SIZE, buf)
                .await;

            assert_eq!(buf, block.as_slice());
        }
    }

    async fn read_partial_block_in_last_piece<'a>(
        data: &[Vec<Vec<u8>>],
        torrent: &'a Torrent,
        fs: &mut FS<'a>,
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
        .await;

        let last_block = last_piece.get(last_block_with_data).unwrap();

        assert_eq!(buf, last_block.as_slice());
    }
}
