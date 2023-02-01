use anyhow::Result;
use std::collections::HashMap;
use std::time::Duration;
use tokio::fs::{File, OpenOptions};
use tokio::time::Instant;

const FILE_CACHE_SIZE: usize = 1 << 4;

pub struct FileCache {
    data: HashMap<String, CacheEntry>,
}

struct CacheEntry {
    file: File,
    read: Option<Instant>,
    write: Option<Instant>,
}

impl FileCache {
    pub fn new() -> Self {
        Self {
            data: HashMap::with_capacity(FILE_CACHE_SIZE),
        }
    }

    pub async fn get_read(&mut self, path: &str) -> Result<&mut File> {
        let entry = self.get_entry(path).await?;

        entry.read = Some(Instant::now());

        Ok(&mut entry.file)
    }

    pub async fn get_write(&mut self, path: &str) -> Result<&mut File> {
        let entry = self.get_entry(path).await?;

        entry.write = Some(Instant::now());

        Ok(&mut entry.file)
    }

    async fn get_entry(&mut self, path: &str) -> Result<&mut CacheEntry> {
        if self.data.len() == FILE_CACHE_SIZE && !self.data.contains_key(path) {
            self.evict();
        }

        Ok(self.data.entry(path.to_string()).or_insert(CacheEntry {
            file: OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .open(path)
                .await?,
            read: None,
            write: None,
        }))
    }

    fn evict(&mut self) {
        let Some((key, _)) =  self.data.iter().max_by_key(|(_,v)| v.weighted_elapsed()) else {
            return;
        };

        let key = key.to_owned();

        self.data.remove(&key);
    }
}

impl CacheEntry {
    fn weighted_elapsed(&self) -> Duration {
        match self {
            CacheEntry {
                read: Some(read),
                write: Some(write),
                ..
            } => read.elapsed() + write.elapsed(),
            CacheEntry {
                read: Some(read), ..
            } => read.elapsed() * 3,
            CacheEntry {
                write: Some(write), ..
            } => write.elapsed() * 3,
            _ => Duration::MAX,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FileCache, FILE_CACHE_SIZE};
    use rand::{distributions::Alphanumeric, Rng};
    use tokio::{
        fs::{remove_dir_all, DirBuilder},
        io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    };

    // tests can't use same dirs because they're async
    const TMP_DIR_1: &str = "tmp1";
    const TMP_DIR_2: &str = "tmp2";

    fn get_random_files(n: usize, path: &str) -> Vec<String> {
        let mut rv = Vec::with_capacity(n);

        for _ in 0..n {
            let name: String = rand::thread_rng()
                .sample_iter(&Alphanumeric)
                .take(rand::thread_rng().gen_range(5..10))
                .map(char::from)
                .collect();

            rv.push(format!("{}/{}", path, name));
        }

        rv
    }

    #[tokio::test]
    async fn rw() {
        DirBuilder::new()
            .recursive(true)
            .create(TMP_DIR_1)
            .await
            .unwrap();

        let mut cache = FileCache::new();

        let file_path = get_random_files(1, TMP_DIR_1).remove(0);

        let data = [1, 2, 3, 4, 5, 6, 7, 8, 9, 0u8];

        let seek_offset = 33;

        let file = cache.get_write(&file_path).await.unwrap();
        file.seek(std::io::SeekFrom::Start(seek_offset))
            .await
            .unwrap();

        file.write_all(&data).await.unwrap();

        let file = cache.get_read(&file_path).await.unwrap();
        file.seek(std::io::SeekFrom::Start(0)).await.unwrap();

        let mut read_buf = [1u8; 5];

        file.read_exact(&mut read_buf).await.unwrap();
        assert_eq!([0u8; 5], read_buf);

        file.seek(std::io::SeekFrom::Start(seek_offset))
            .await
            .unwrap();

        file.read_exact(&mut read_buf).await.unwrap();
        assert_eq!(data[..read_buf.len()], read_buf);

        assert_eq!(1, cache.data.len());

        drop(cache);

        remove_dir_all(TMP_DIR_1).await.unwrap();
    }

    #[tokio::test]
    async fn evict() {
        DirBuilder::new()
            .recursive(true)
            .create(TMP_DIR_2)
            .await
            .unwrap();

        let mut cache = FileCache::new();

        let files = get_random_files(FILE_CACHE_SIZE, TMP_DIR_2);
        let extra_file = get_random_files(1, TMP_DIR_2).remove(0);

        for file in &files {
            let _ = cache.get_write(file).await.unwrap();
        }

        let _ = cache.get_write(&extra_file).await.unwrap();

        assert!(cache.data.contains_key(&extra_file));

        for file in &files[1..] {
            assert!(cache.data.contains_key(file));
        }

        assert!(!cache.data.contains_key(&files[0]));

        remove_dir_all(TMP_DIR_2).await.unwrap();
    }
}
