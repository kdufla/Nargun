use super::file_cache::FileCache;
use super::multi_file_iter::FilesInBounds;
use super::Fs;
use crate::peers::connection::connection_manager::BLOCK_SIZE;
use crate::peers::connection::pending_pieces::FinishedPiece;
use crate::transcoding::metainfo::{self, Info, Mode};
use anyhow::{bail, Result};
use async_trait::async_trait;
use std::io::SeekFrom;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

pub struct MultiFileRw {
    torrent_name: String,
    files: FileCache,
    info: Info,
    file_infos: Vec<metainfo::File>,
}
impl MultiFileRw {
    pub async fn new(base_dir: &str, info: Info) -> Self {
        let Mode::Multi { files } = info.mode.clone() else {
            panic!("mock torrent is not in multi-file mode");
        };

        dir_builder::build_dir_tree(base_dir, &info.name, &files)
            .await
            .unwrap();

        Self {
            torrent_name: format!("{}/{}", base_dir, &info.name),
            files: FileCache::new(),
            info,
            file_infos: files,
        }
    }
}

#[async_trait]
impl Fs for MultiFileRw {
    async fn read_block(
        &mut self,
        index: usize,
        begin: usize,
        length: usize,
        buf: &mut [u8],
    ) -> Result<()> {
        let piece_start = index * self.info.piece_length as usize;
        let block_start = piece_start + begin;
        let block_end = block_start + length;

        for file_info in FilesInBounds::new(&self.file_infos, block_start, block_end) {
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

    async fn write_piece(&mut self, piece: FinishedPiece) -> Result<()> {
        let Self {
            files, file_infos, ..
        } = self;

        let piece_start = piece.idx * self.info.piece_length as usize;
        let piece_end = (piece.idx + 1) * self.info.piece_length as usize;

        for file_info in FilesInBounds::new(file_infos, piece_start, piece_end) {
            let path = format!("{}/{}", self.torrent_name, file_info.file.path.join("/"));
            let fs_file = files.get_write(&path).await?;

            write_file_chunk_from_multi_file_piece(
                fs_file,
                &piece.data,
                file_info.file_bytes_before_iter,
                file_info.start_within_iter_bounds,
                file_info.end_within_iter_bounds,
            )
            .await?;
        }

        Ok(())
    }
}

async fn write_file_chunk_from_multi_file_piece(
    file: &mut tokio::fs::File,
    piece_data: &[Vec<u8>],
    start_within_file: usize,
    start_within_piece: usize,
    end_within_piece: usize,
) -> Result<()> {
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
            file.write_all(&file_fits_in_block[start_within_block..start_within_block + length])
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

mod dir_builder {
    use crate::transcoding::metainfo;
    use anyhow::{bail, Result};
    use std::collections::HashSet;
    use tokio::fs::{remove_dir_all, DirBuilder};

    pub async fn build_dir_tree(
        base_dir: &str,
        torrent_dir: &str,
        files: &[metainfo::File],
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

    fn dirs_by_asc_depth(working_dir: &str, files: &[metainfo::File]) -> Vec<String> {
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
}

#[cfg(test)]
mod tests {
    use super::MultiFileRw;
    use crate::fs::Fs;
    use crate::peers::connection::connection_manager::BLOCK_SIZE;
    use crate::peers::connection::pending_pieces::FinishedPiece;
    use crate::transcoding::metainfo::Torrent;
    use rand::seq::IteratorRandom;
    use tokio::fs::{remove_dir_all, DirBuilder};
    use tracing_test::traced_test;

    const PATH: &str = "multi_test";

    #[tokio::test]
    #[traced_test]
    async fn write_then_read_multi() {
        DirBuilder::new()
            .recursive(true)
            .create(PATH)
            .await
            .unwrap();

        let (data, torrent) = Torrent::mock_multi();

        let fs = MultiFileRw::new(PATH, torrent.info.clone()).await;

        write_then_read(data, &torrent, fs).await;

        remove_dir_all(PATH).await.unwrap();
    }

    async fn write_then_read<'a>(
        data: Vec<Vec<Vec<u8>>>,
        torrent: &'a Torrent,
        mut fs: MultiFileRw,
    ) {
        write(&data, &mut fs).await;
        read(&data, torrent, &mut fs).await;
    }

    async fn write<'a>(data: &[Vec<Vec<u8>>], fs: &mut MultiFileRw) {
        for (idx, data) in data
            .iter()
            .cloned()
            .enumerate()
            .choose_multiple(&mut rand::thread_rng(), data.len())
        {
            fs.write_piece(FinishedPiece { idx, data }).await.unwrap();
        }
    }

    async fn read<'a>(data: &[Vec<Vec<u8>>], torrent: &'a Torrent, fs: &mut MultiFileRw) {
        let mut buf = Box::new([0u8; BLOCK_SIZE]);

        read_full_pieces(data, fs, buf.as_mut_slice()).await;

        read_full_blocks_in_last_piece(data, torrent, fs, buf.as_mut_slice()).await;

        read_partial_block_in_last_piece(data, torrent, fs, buf.as_mut_slice()).await;
    }

    async fn read_full_pieces<'a>(data: &[Vec<Vec<u8>>], fs: &mut MultiFileRw, buf: &mut [u8]) {
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
        fs: &mut MultiFileRw,
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
        fs: &mut MultiFileRw,
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
