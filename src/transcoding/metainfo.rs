use crate::client::BLOCK_SIZE;
use crate::data_structures::{ID, ID_LEN};
use crate::{ok_or_missing_field, unsigned_ceil_div};
use anyhow::{anyhow, Result};
use bendy::decoding::{Decoder, FromBencode, Object};
use bendy::encoding::AsString;
use openssl::sha;
use std::collections::HashSet;
use std::fmt;
use std::fs::File as fsFile;
use std::io::Read;
use std::path::Path;

// TODO I'm not gonna touch FromBencode parts, but it's not good. one more reason to get rid of that shit crate.

#[derive(Debug)]
pub struct Torrent {
    pub info: Info,
    pub info_hash: ID,
    pub announce: HashSet<TrackerAddr>,
}

#[derive(Debug)]
pub struct Info {
    pub name: String,
    pub piece_length: u64,
    pub pieces: Vec<ID>,
    pub mode: Mode,
}

#[derive(Debug)]
pub enum Mode {
    Single { length: u64 },
    Multi { files: Vec<File> },
}

#[derive(Debug)]
pub struct File {
    pub path: Vec<String>,
    pub length: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TrackerAddr {
    Http(String),
    Udp(String),
}

impl Torrent {
    pub fn from_buf(buf: &[u8]) -> Result<Self> {
        Ok(Self::from_bencode(buf)?)
    }

    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut buf = Vec::new();

        let mut file = fsFile::open(path)?;
        file.read_to_end(&mut buf)?;

        Self::from_buf(&buf)
    }

    pub fn is_valid_piece(&self, piece_idx: usize, piece_data: &Vec<Vec<u8>>) -> bool {
        piece_data.len() == unsigned_ceil_div!(self.info.piece_length as usize, BLOCK_SIZE)
            && piece_data.iter().all(|block| block.len() == BLOCK_SIZE)
            && self.info.pieces[piece_idx] == Self::piece_hash(piece_data)
    }

    pub fn http_trackers(&self) -> impl Iterator<Item = &String> {
        self.announce
            .iter()
            .filter_map(|tracker_addr| match tracker_addr {
                TrackerAddr::Http(addr_string) => Some(addr_string),
                _ => None,
            })
    }

    pub fn count_pieces(&self) -> usize {
        self.info.pieces.len()
    }

    pub fn piece_hash(piece_data: &Vec<Vec<u8>>) -> ID {
        let mut hasher = sha::Sha1::new();

        for block in piece_data {
            hasher.update(block);
        }

        ID::new(hasher.finish())
    }
}

#[cfg(test)]
impl Torrent {
    pub fn mock_single() -> (Vec<Vec<Vec<u8>>>, Self) {
        use rand::{distributions::Alphanumeric, Rng};

        let mut rng = rand::thread_rng();
        let info_hash = ID::new(rng.gen());
        let piece_length = 1 << rng.gen_range(15..18);
        let piece_count = rng.gen_range(4..8);
        let block_per_piece = unsigned_ceil_div!(piece_length, BLOCK_SIZE);
        let file_length =
            rng.gen_range((((piece_count - 1) * piece_length) + 1)..(piece_count * piece_length)); // somewhere within last piece

        let mut data: Vec<Vec<Vec<u8>>> = (0..piece_count)
            .map(|_| {
                (0..block_per_piece)
                    .map(|_| (0..BLOCK_SIZE).map(|_| rng.gen()).collect())
                    .collect()
            })
            .collect();

        let data_in_last_piece = file_length % piece_length;
        let last_block_with_data = data_in_last_piece / BLOCK_SIZE;
        let data_in_last_block = data_in_last_piece % BLOCK_SIZE;

        data.last_mut().unwrap()[last_block_with_data + 1..]
            .iter_mut()
            .for_each(|block| *block = vec![0; BLOCK_SIZE]);

        data.last_mut()
            .unwrap()
            .get_mut(last_block_with_data)
            .unwrap()[data_in_last_block..]
            .iter_mut()
            .for_each(|byte| *byte = 0);

        let piece_hashes: Vec<ID> = data.iter().map(Self::piece_hash).collect();

        let info = Info {
            name: rng
                .sample_iter(&Alphanumeric)
                .take(8)
                .map(char::from)
                .collect(),
            piece_length: piece_length as u64,
            pieces: piece_hashes,
            mode: Mode::Single {
                length: file_length as u64,
            },
        };

        (
            data,
            Self {
                info_hash,
                announce: HashSet::from([
                    TrackerAddr::Http("http://open.acgnxtracker.com:80/announce".to_string()),
                    TrackerAddr::Http("https://tracker2.dler.org:80/announce".to_string()),
                    TrackerAddr::Udp("udp://exodus.desync.com:6969/announce".to_string()),
                ]),
                info,
            },
        )
    }

    pub fn mock_multi() -> (Vec<Vec<Vec<u8>>>, Self) {
        use rand::{distributions::Alphanumeric, seq::SliceRandom, Rng};

        let (data, mut single_mock) = Self::mock_single();

        let mut rng = rand::thread_rng();

        let number_of_files = rng.gen_range(2..10);

        let dir_names: Vec<String> = (0..8)
            .map(|_| {
                rand::thread_rng()
                    .sample_iter(&Alphanumeric)
                    .take(6)
                    .map(char::from)
                    .collect()
            })
            .collect();

        let file_names: Vec<String> = (0..8)
            .map(|_| {
                rand::thread_rng()
                    .sample_iter(&Alphanumeric)
                    .take(10)
                    .map(char::from)
                    .collect()
            })
            .collect();

        let torrent_size = single_mock.info.piece_length * single_mock.info.pieces.len() as u64;
        let mut file_ends: Vec<u64> = (0..number_of_files - 1)
            .map(|_| rng.gen_range(0..torrent_size - 1))
            .collect();
        file_ends.push(single_mock.info.length()); // one that ends in last piece

        file_ends.sort();

        let mut prev_end = 0;
        for file_end in file_ends.iter_mut() {
            let tmp = *file_end;
            *file_end -= prev_end;
            prev_end = tmp;
        }

        let file_lengths = file_ends;

        let mut files: Vec<File> = file_lengths
            .into_iter()
            .map(|length| {
                let file_depth = rng.gen_range(1..6);

                File {
                    length,
                    path: dir_names
                        .choose_multiple(&mut rng, file_depth)
                        .cloned()
                        .collect(),
                }
            })
            .collect();

        for (idx, file) in files.iter_mut().enumerate() {
            file.path
                .push(format!("{}{}", file_names.choose(&mut rng).unwrap(), idx))
        }

        single_mock.info.mode = Mode::Multi { files };
        (data, single_mock)
    }
}

impl Info {
    pub fn length(&self) -> u64 {
        match &self.mode {
            Mode::Single { length } => *length,
            Mode::Multi { files } => files.iter().map(|file| file.length).sum(),
        }
    }
}

impl TrackerAddr {
    fn new(addr: String) -> Result<Self> {
        if addr.starts_with("udp") {
            Ok(Self::Udp(addr))
        } else if addr.starts_with("http") {
            Ok(Self::Http(addr))
        } else {
            Err(anyhow!("tracker addr must be http or udp. got {addr}."))
        }
    }
}

impl FromBencode for Torrent {
    const EXPECTED_RECURSION_DEPTH: usize = 10;

    fn decode_bencode_object(object: Object) -> Result<Self, bendy::decoding::Error> {
        let mut info = None;
        let mut info_hash = None;
        let mut announce = HashSet::new();

        let mut torrent = object.try_into_dictionary()?;
        while let Some(kv) = torrent.next_pair()? {
            match kv {
                (b"info", value) => {
                    let bytes = value.try_into_dictionary()?.into_raw()?;

                    let mut hasher = sha::Sha1::new();
                    hasher.update(bytes);
                    info_hash = Some(ID::new(hasher.finish()));

                    let mut decoder = Decoder::new(bytes);
                    let obj = decoder.next_object()?;

                    if let Some(object) = obj {
                        info = Some(Info::decode_bencode_object(object)?);
                    }
                }
                (b"announce", value) => {
                    if let Ok(tracker_addr) =
                        TrackerAddr::new(String::decode_bencode_object(value)?)
                    {
                        announce.insert(tracker_addr);
                    }
                }
                (b"announce-list", value) => {
                    let list = Vec::<Vec<String>>::decode_bencode_object(value)?;
                    for intermediate in list {
                        for url_string in intermediate {
                            if let Ok(tracker_addr) = TrackerAddr::new(url_string) {
                                announce.insert(tracker_addr);
                            }
                        }
                    }
                }
                _ => (),
            }
        }

        Ok(Torrent {
            announce: ok_or_missing_field!((!announce.is_empty()).then_some(announce), "announce")?,
            info: ok_or_missing_field!(info)?,
            info_hash: ok_or_missing_field!(info_hash, "info")?,
        })
    }
}

impl FromBencode for Info {
    const EXPECTED_RECURSION_DEPTH: usize = 10;

    fn decode_bencode_object(object: Object) -> Result<Self, bendy::decoding::Error> {
        let mut piece_length = None;
        let mut pieces = None;
        let mut mode = None;
        let mut name = None;

        let mut info = object.try_into_dictionary()?;
        while let Some(kv) = info.next_pair()? {
            match kv {
                (b"name", value) => {
                    name = Some(String::decode_bencode_object(value)?);
                }
                (b"pieces", value) => {
                    pieces = {
                        let raw = AsString::decode_bencode_object(value)?.0;
                        Some(deserialize_pieces_field(raw)?)
                    };
                }
                (b"length", value) => {
                    mode = Some(Mode::Single {
                        length: u64::decode_bencode_object(value)?,
                    });
                }
                (b"piece length", value) => {
                    piece_length = Some(u64::decode_bencode_object(value)?);
                }
                (b"files", value) => {
                    mode = Some(Mode::Multi {
                        files: Vec::<File>::decode_bencode_object(value)?,
                    });
                }
                _ => (),
            }
        }

        Ok(Info {
            piece_length: ok_or_missing_field!(piece_length)?,
            pieces: ok_or_missing_field!(pieces)?,
            name: ok_or_missing_field!(name)?,
            mode: ok_or_missing_field!(mode, "length or files")?,
        })
    }
}

impl FromBencode for File {
    const EXPECTED_RECURSION_DEPTH: usize = 10;

    fn decode_bencode_object(object: Object) -> Result<Self, bendy::decoding::Error> {
        let mut path = None;
        let mut length = None;

        let mut file = object.try_into_dictionary()?;
        while let Some(kv) = file.next_pair()? {
            match kv {
                (b"path", value) => {
                    path = Some(Vec::<String>::decode_bencode_object(value)?);
                }
                (b"length", value) => {
                    length = Some(u64::decode_bencode_object(value)?);
                }
                _ => (),
            }
        }

        Ok(File {
            path: ok_or_missing_field!(path)?,
            length: ok_or_missing_field!(length)?,
        })
    }
}

fn deserialize_pieces_field(mut raw: Vec<u8>) -> Result<Vec<ID>, bendy::decoding::Error> {
    if raw.is_empty() || raw.len() % ID_LEN > 0 {
        return Err(bendy::decoding::Error::missing_field(format!(
            "Info::pieces must be 20-byte SHA1 hash values but it has len={}",
            raw.len()
        )));
    }

    let piece_hashes = unsafe {
        let length = raw.len() / ID_LEN;
        let capacity = raw.capacity() / ID_LEN;
        let ptr = raw.as_mut_ptr() as *mut ID;
        std::mem::forget(raw);
        Vec::from_raw_parts(ptr, length, capacity)
    };

    Ok(piece_hashes)
}

impl fmt::Display for Torrent {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "announce:\t{:?}\n\
            name:\t\t{}\n\
            piece length:\t{:?}\n\
            piece count:\t{:?}\n\
            mode:\t\t{:?}",
            self.announce,
            self.info.name,
            self.info.piece_length,
            self.info.pieces.len(),
            self.info.mode,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{Mode, Torrent, TrackerAddr};
    use crate::client::BLOCK_SIZE;
    use crate::{data_structures::ID, unsigned_ceil_div};
    use std::fs;
    use std::io::Read;

    #[test]
    fn multi_parse() {
        let torrent = Torrent::from_file(String::from("resources/38WarBreaker.torrent")).unwrap();

        assert_eq!(torrent.announce.len(), 11);
        assert!(torrent.announce.contains(&TrackerAddr::Http(
            "http://tracker.files.fm:6969/announce".to_string()
        )));
        assert_eq!(
            torrent.info_hash,
            ID::new([
                0x55, 0x52, 0x08, 0x7e, 0xc1, 0x98, 0x40, 0xac, 0xe8, 0x79, 0x5a, 0xf9, 0x3e, 0x13,
                0x7d, 0x2b, 0xd7, 0x14, 0x50, 0xd7
            ])
        );
        match &torrent.info.mode {
            Mode::Multi { files } => {
                assert_eq!(23, files.len())
            }
            Mode::Single { .. } => panic!("expected multi file mode"),
        }
        assert_eq!(torrent.info.name, "WarBreaker");
        assert_eq!(torrent.info.piece_length, 16777216);
        assert_eq!(
            *torrent.info.pieces.first().unwrap(),
            ID::new([
                0xA8, 0xB4, 0x44, 0x51, 0x01, 0x32, 0x24, 0xC9, 0x2D, 0xC9, 0x10, 0x5F, 0xB8, 0x29,
                0x04, 0xF6, 0xC4, 0x37, 0xB5, 0x91
            ])
        );
        assert_eq!(
            torrent.info.pieces.len() as u64,
            unsigned_ceil_div!(torrent.info.length(), torrent.info.piece_length)
        );
    }

    #[test]
    fn single_parse() {
        let torrent = Torrent::from_file(String::from(
            "resources/ubuntu-22.04.1-desktop-amd64.iso.torrent",
        ))
        .unwrap();

        assert_eq!(torrent.announce.len(), 2);
        assert!(torrent.announce.contains(&TrackerAddr::Http(
            "https://torrent.ubuntu.com/announce".to_string()
        )));
        assert_eq!(
            torrent.info_hash,
            ID::new([
                0x3b, 0x24, 0x55, 0x04, 0xcf, 0x5f, 0x11, 0xbb, 0xdb, 0xe1, 0x20, 0x1c, 0xea, 0x6a,
                0x6b, 0xf4, 0x5a, 0xee, 0x1b, 0xc0
            ])
        );
        match torrent.info.mode {
            Mode::Multi { .. } => panic!("expected single file mode"),
            Mode::Single { length } => {
                assert_eq!(3826831360, length);
            }
        }
        assert_eq!(torrent.info.name, "ubuntu-22.04.1-desktop-amd64.iso");
        assert_eq!(torrent.info.piece_length, 262144);
        assert_eq!(
            *torrent.info.pieces.first().unwrap(),
            ID::new([
                0x56, 0x7B, 0x9B, 0x5C, 0x3D, 0x06, 0x17, 0xB2, 0xDA, 0xAB, 0x0C, 0xE8, 0x88, 0xED,
                0x2B, 0x9A, 0xC2, 0x33, 0x02, 0xFC
            ])
        );
        assert_eq!(
            torrent.info.pieces.len() as u64,
            unsigned_ceil_div!(torrent.info.length(), torrent.info.piece_length)
        );
    }

    // future G, when you inevitably get a bilbo moment and start wondering "why shouldn't I?"
    // mock is cool but you need a real piece to make sure that hasher works. don't delete this test.
    #[test]
    fn hasher() {
        let torrent = Torrent::from_file(
            "resources/RuPauls.Drag.Race.S15E04.1080p.WEB.H264-SPAMnEGGS[rartv]-[rarbg.to].torrent",
        )
        .unwrap();

        let blocks_in_piece = torrent.info.piece_length as usize / BLOCK_SIZE;
        let mut buf = vec![vec![0u8; BLOCK_SIZE]; blocks_in_piece];

        let mut file = fs::File::open("resources/rupaul_piece_01").unwrap();

        for block_buf in buf.iter_mut() {
            file.read_exact(block_buf).unwrap();
        }

        assert!(torrent.is_valid_piece(1, &buf));
    }

    #[test]
    fn mock() {
        let (data, torrent) = Torrent::mock_multi();

        assert_eq!(
            torrent.info.pieces.len(),
            unsigned_ceil_div!(torrent.info.length(), torrent.info.piece_length) as usize
        );

        for (piece_idx, piece_data) in data.iter().enumerate() {
            assert!(torrent.is_valid_piece(piece_idx, piece_data))
        }
    }
}
