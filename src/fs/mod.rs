mod file_cache;
mod multi_file_iter;
mod multi_file_rw;
mod single_file_rw;

use crate::{peers::connection::pending_pieces::FinishedPiece, transcoding::metainfo::Info};

#[async_trait::async_trait]
pub trait Fs {
    async fn read_block(
        &mut self,
        index: usize,
        begin: usize,
        length: usize,
        buf: &mut [u8],
    ) -> anyhow::Result<()>;
    async fn write_piece(&mut self, piece: FinishedPiece) -> anyhow::Result<()>;
}

pub struct FS(Box<dyn Fs + Send>);

impl FS {
    pub async fn new(base_dir: &str, info: Info) -> Self {
        match &info.mode {
            crate::transcoding::metainfo::Mode::Single { .. } => {
                return Self(Box::new(single_file_rw::SingleFileRw::new(base_dir, info)))
            }
            crate::transcoding::metainfo::Mode::Multi { .. } => {
                return Self(Box::new(
                    multi_file_rw::MultiFileRw::new(base_dir, info).await,
                ))
            }
        }
    }
}

#[async_trait::async_trait]
impl Fs for FS {
    async fn read_block(
        &mut self,
        index: usize,
        begin: usize,
        length: usize,
        buf: &mut [u8],
    ) -> anyhow::Result<()> {
        self.0.read_block(index, begin, length, buf).await
    }
    async fn write_piece(&mut self, piece: FinishedPiece) -> anyhow::Result<()> {
        self.0.write_piece(piece).await
    }
}
