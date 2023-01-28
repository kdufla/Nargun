use crate::data_structures::{ID, ID_LEN};
use crate::ok_or_missing_field;
use bendy::decoding::{Decoder, FromBencode, Object};
use bendy::encoding::AsString;
use openssl::sha;
use std::collections::HashSet;
use std::fmt;
use std::fs::File as fsFile;
use std::io::Read;

// TODO this is the first thing I did. it's shit most likely

#[derive(Debug)]
pub struct File {
    pub path: Vec<String>,
    pub length: u64,
}

#[derive(Debug)]
pub struct Info {
    pub name: String,
    pub piece_length: u64,
    pub pieces: Vec<ID>,
    pub length: Option<u64>,
    pub files: Option<Vec<File>>,
}

#[derive(Debug)]
pub struct Torrent {
    pub info: Info,
    pub info_hash: ID,
    pub announce: HashSet<TrackerAddr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TrackerAddr {
    Http(String),
    Udp(String),
}

pub fn from_buffer(buffer: &[u8]) -> Torrent {
    Torrent::from_bencode(buffer).unwrap()
}
pub fn from_file(filename: &String) -> Torrent {
    let mut buffer = Vec::new();

    let mut file = fsFile::open(filename).unwrap();
    file.read_to_end(&mut buffer).unwrap();

    from_buffer(buffer.as_slice())
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

impl Info {
    pub fn in_single_file_mode(&self) -> bool {
        self.length.is_some()
    }

    pub fn in_multi_file_mode(&self) -> bool {
        self.files.is_some()
    }

    pub fn length(&self) -> u64 {
        match &self.files {
            Some(files) => files.iter().map(|f| f.length).sum(),
            None => self.length.unwrap(),
        }
    }

    pub fn number_of_pieces(&self) -> usize {
        self.pieces.len()
    }
}

fn deserialize_pieces(mut raw: Vec<u8>) -> Result<Vec<ID>, bendy::decoding::Error> {
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

impl FromBencode for Info {
    const EXPECTED_RECURSION_DEPTH: usize = 10;

    fn decode_bencode_object(object: Object) -> Result<Self, bendy::decoding::Error> {
        let mut piece_length = None;
        let mut pieces = None;
        let mut length = None;
        let mut name = None;
        let mut files = None;

        let mut info = object.try_into_dictionary()?;
        while let Some(kv) = info.next_pair()? {
            match kv {
                (b"name", value) => {
                    name = Some(String::decode_bencode_object(value)?);
                }
                (b"pieces", value) => {
                    pieces = {
                        let raw = AsString::decode_bencode_object(value)?.0;
                        Some(deserialize_pieces(raw)?)
                    };
                }
                (b"length", value) => {
                    length = Some(u64::decode_bencode_object(value)?);
                }
                (b"piece length", value) => {
                    piece_length = Some(u64::decode_bencode_object(value)?);
                }
                (b"files", value) => {
                    files = Some(Vec::<File>::decode_bencode_object(value)?);
                }
                _ => (),
            }
        }

        if length.is_none() && files.is_none() {
            return Err(bendy::decoding::Error::missing_field("length and files"));
        }

        Ok(Info {
            piece_length: ok_or_missing_field!(piece_length)?,
            pieces: ok_or_missing_field!(pieces)?,
            name: ok_or_missing_field!(name)?,
            length,
            files,
        })
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
                    if let Some(tracker_addr) =
                        TrackerAddr::from_string(String::decode_bencode_object(value)?)
                    {
                        announce.insert(tracker_addr);
                    }
                }
                (b"announce-list", value) => {
                    let list = Vec::<Vec<String>>::decode_bencode_object(value)?;
                    for intermediate in list {
                        for url_string in intermediate {
                            if let Some(tracker_addr) = TrackerAddr::from_string(url_string) {
                                announce.insert(tracker_addr);
                            }
                        }
                    }
                }
                _ => (),
            }
        }

        Ok(Torrent {
            announce: ok_or_missing_field!((!announce.is_empty()).then(|| announce), "announce")?,
            info: ok_or_missing_field!(info)?,
            info_hash: ok_or_missing_field!(info_hash, "info")?,
        })
    }
}

impl TrackerAddr {
    fn from_string(s: String) -> Option<TrackerAddr> {
        if s.starts_with("udp") {
            Some(TrackerAddr::Udp(s))
        } else if s.starts_with("http") {
            Some(TrackerAddr::Http(s))
        } else {
            None
        }
    }
}

impl fmt::Display for Torrent {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut formatted_files = Vec::new();

        if let Some(files) = &self.info.files {
            for (i, file) in files.iter().enumerate() {
                formatted_files.push(format!(
                    "file_{:?} length:\t{}\tpath:\t{:?}\n",
                    i, file.length, file.path
                ));
            }
        };

        write!(
            f,
            "announce:\t{:?}\n\
            name:\t\t{}\n\
            piece length:\t{:?}\n\
            piece count:\t{:?}\n\
            length:\t\t{:?}\n\
            {}",
            self.announce,
            self.info.name,
            self.info.piece_length,
            self.info.pieces.len(),
            self.info.length,
            formatted_files.join(""),
        )
    }
}

impl Torrent {
    pub fn count_http_announcers(&self) -> usize {
        self.announce
            .iter()
            .fold(0, |count, announcer| match *announcer {
                TrackerAddr::Http(_) => count + 1,
                _ => count,
            })
    }

    pub fn http_trackers(&self) -> impl Iterator<Item = &String> {
        self.announce.iter().filter_map(|x| match x {
            TrackerAddr::Http(s) => Some(s),
            _ => None,
        })
    }

    pub fn count_pieces(&self) -> u64 {
        self.info.pieces.len() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::{from_file, TrackerAddr};
    use crate::{data_structures::ID, unsigned_ceil_div};

    const METAINFO_MULTI: &str = "resources/38WarBreaker.torrent";
    const METAINFO_SINGLE: &str = "resources/ubuntu-22.04.1-desktop-amd64.iso.torrent";

    #[test]
    fn multi_parse() {
        let torrent = from_file(&String::from(METAINFO_MULTI));

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
        assert!(torrent.info.length.is_none());
        assert!(torrent.info.files.is_some());
        assert_eq!(
            match &torrent.info.files {
                Some(files) => files.len(),
                None => 0,
            },
            23
        );
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
        let torrent = from_file(&String::from(METAINFO_SINGLE));

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
        assert!(torrent.info.length.is_some());
        assert!(torrent.info.files.is_none());
        assert_eq!(torrent.info.length.unwrap(), 3826831360);
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
}
