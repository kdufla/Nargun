use crate::data_structures::SerializableBuf;
use anyhow::Result;
use bincode::Options;
use serde::ser::SerializeTuple;
use serde::{Serialize, Serializer};
use tracing::{debug, error, warn};

use super::connection_manager::BLOCK_SIZE;
use super::downloader::BlockAddress;

pub const BYTES_IN_LEN_PREFIX: usize = 4;
const ID_IDX: usize = 4;
const BITFIELD_START: usize = 5;
const BLOCK_START: usize = 13;
const FIRST_NUM_START: usize = 5;
const SECOND_NUM_START: usize = 9;
const THIRD_NUM_START: usize = 13;

const CHOKE_ID: u8 = 0;
const UNCHOKE_ID: u8 = 1;
const INTERESTED_ID: u8 = 2;
const NOT_INTERESTED_ID: u8 = 3;
const HAVE_ID: u8 = 4;
const BITFIELD_ID: u8 = 5;
const REQUEST_ID: u8 = 6;
const PIECE_ID: u8 = 7;
const CANCEL_ID: u8 = 8;
const PORT_ID: u8 = 9;

const CHOKE_LEN: u32 = 1;
const UNCHOKE_LEN: u32 = 1;
const INTERESTED_LEN: u32 = 1;
const NOT_INTERESTED_LEN: u32 = 1;
const HAVE_LEN: u32 = 5;
const BITFIELD_LEN: u32 = 1;
const REQUEST_LEN: u32 = 13;
const PIECE_LEN: u32 = 9;
const CANCEL_LEN: u32 = 13;
const PORT_LEN: u32 = 3;

#[derive(Debug, PartialEq)]
pub enum Message {
    KeepAlive,
    Choke,
    Unchoke,
    Interested,
    NotInterested,
    Have(u32),
    Bitfield(SerializableBuf),
    Request(Request),
    Piece(Piece),
    Cancel(Request),
    Port(u16),
}

#[derive(Debug, Serialize, PartialEq, Clone)]
pub struct Request {
    pub index: u32,
    pub begin: u32,
    pub length: u32,
}

#[derive(Debug, Serialize, PartialEq, Clone)]
pub struct Piece {
    pub index: u32,
    pub begin: u32,
    pub block: SerializableBuf,
}

macro_rules! u32_from_be_slice {
    ($slice:expr) => {
        (($slice[0] as u32) << 24)
            + (($slice[1] as u32) << 16)
            + (($slice[2] as u32) << 8)
            + ($slice[3] as u32)
    };
}

impl Message {
    pub fn len(buf: &[u8]) -> usize {
        u32_from_be_slice!(buf[0..BYTES_IN_LEN_PREFIX]) as usize
    }

    pub fn from_buf(buf: &[u8]) -> Option<Self> {
        let len = u32_from_be_slice!(buf[0..BYTES_IN_LEN_PREFIX]);
        let len: usize = len as usize + BYTES_IN_LEN_PREFIX;

        if buf.len() < len {
            let first_zeros = buf
                .chunks_exact(5)
                .position(|chunk| {
                    chunk[0] == 0
                        && chunk[1] == 0
                        && chunk[2] == 0
                        && chunk[3] == 0
                        && chunk[4] == 0
                })
                .map(|idx| idx * 5);

            error!("message len ({len}) is more than provided buffer {}. first five consecutive zeros are at {:?} (+-4)",
                buf.len(), first_zeros);

            return None;
        }

        let message = if len == BYTES_IN_LEN_PREFIX {
            Message::KeepAlive
        } else {
            let id = buf[ID_IDX];
            debug!(id);
            match id {
                CHOKE_ID => Message::Choke,
                UNCHOKE_ID => Message::Unchoke,
                INTERESTED_ID => Message::Interested,
                NOT_INTERESTED_ID => Message::NotInterested,
                HAVE_ID => Message::Have(u32_from_be_slice!(buf[FIRST_NUM_START..])),
                BITFIELD_ID => Message::Bitfield(SerializableBuf::from(&buf[BITFIELD_START..len])),
                REQUEST_ID => Message::Request(Request {
                    index: u32_from_be_slice!(buf[FIRST_NUM_START..]),
                    begin: u32_from_be_slice!(buf[SECOND_NUM_START..]),
                    length: u32_from_be_slice!(buf[THIRD_NUM_START..]),
                }),
                PIECE_ID => Message::Piece(Piece::new(
                    u32_from_be_slice!(buf[FIRST_NUM_START..]),
                    u32_from_be_slice!(buf[SECOND_NUM_START..]),
                    &buf[BLOCK_START..len],
                )),
                CANCEL_ID => Message::Cancel(Request {
                    index: u32_from_be_slice!(buf[FIRST_NUM_START..]),
                    begin: u32_from_be_slice!(buf[SECOND_NUM_START..]),
                    length: u32_from_be_slice!(buf[THIRD_NUM_START..]),
                }),
                PORT_ID => Message::Port(
                    ((buf[FIRST_NUM_START] as u16) << 8) + (buf[FIRST_NUM_START + 1] as u16),
                ),
                unsupported => {
                    warn!("unsupported id {}", unsupported);
                    return None;
                }
            }
        };

        Some(message)
    }

    pub fn into_bytes(self) -> Vec<u8> {
        bincode::DefaultOptions::new()
            .with_big_endian()
            .with_fixint_encoding()
            .serialize(&self)
            .unwrap()
    }
}

impl Serialize for Message {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Message::KeepAlive => serializer.serialize_u32(0),
            Message::Choke => {
                let mut tup = serializer.serialize_tuple(2)?;
                tup.serialize_element(&CHOKE_LEN)?;
                tup.serialize_element(&CHOKE_ID)?;
                tup.end()
            }
            Message::Unchoke => {
                let mut tup = serializer.serialize_tuple(2)?;
                tup.serialize_element(&UNCHOKE_LEN)?;
                tup.serialize_element(&UNCHOKE_ID)?;
                tup.end()
            }
            Message::Interested => {
                let mut tup = serializer.serialize_tuple(2)?;
                tup.serialize_element(&INTERESTED_LEN)?;
                tup.serialize_element(&INTERESTED_ID)?;
                tup.end()
            }
            Message::NotInterested => {
                let mut tup = serializer.serialize_tuple(2)?;
                tup.serialize_element(&NOT_INTERESTED_LEN)?;
                tup.serialize_element(&NOT_INTERESTED_ID)?;
                tup.end()
            }
            Message::Have(piece_index) => {
                let mut tup = serializer.serialize_tuple(3)?;
                tup.serialize_element(&HAVE_LEN)?;
                tup.serialize_element(&HAVE_ID)?;
                tup.serialize_element(&piece_index)?;
                tup.end()
            }
            Message::Bitfield(bitfield) => {
                let mut tup = serializer.serialize_tuple(bitfield.len() + 2)?;
                tup.serialize_element(&(BITFIELD_LEN + bitfield.len() as u32))?;
                tup.serialize_element(&BITFIELD_ID)?;
                for byte in bitfield.iter() {
                    tup.serialize_element(byte)?;
                }
                tup.end()
            }
            Message::Request(request) => {
                let mut tup = serializer.serialize_tuple(3)?;
                tup.serialize_element(&REQUEST_LEN)?;
                tup.serialize_element(&REQUEST_ID)?;
                tup.serialize_element(&request)?;
                tup.end()
            }
            Message::Piece(piece) => {
                let mut tup = serializer.serialize_tuple(piece.len() + 4)?;
                tup.serialize_element(&(PIECE_LEN + piece.len() as u32))?;
                tup.serialize_element(&PIECE_ID)?;
                tup.serialize_element(&piece.index)?;
                tup.serialize_element(&piece.begin)?;
                for byte in piece.block.iter() {
                    tup.serialize_element(byte)?;
                }
                tup.end()
            }
            Message::Cancel(request) => {
                let mut tup = serializer.serialize_tuple(3)?;
                tup.serialize_element(&CANCEL_LEN)?;
                tup.serialize_element(&CANCEL_ID)?;
                tup.serialize_element(&request)?;
                tup.end()
            }
            Message::Port(listen_port) => {
                let mut tup = serializer.serialize_tuple(3)?;
                tup.serialize_element(&PORT_LEN)?;
                tup.serialize_element(&PORT_ID)?;
                tup.serialize_element(listen_port)?;
                tup.end()
            }
        }
    }
}

impl Piece {
    pub fn new(index: u32, begin: u32, block: &[u8]) -> Piece {
        Piece {
            index,
            begin,
            block: SerializableBuf::from(block),
        }
    }

    pub fn len(&self) -> usize {
        self.block.len()
    }
}

impl From<BlockAddress> for Request {
    fn from(value: BlockAddress) -> Self {
        Self {
            index: value.piece_idx as u32,
            begin: (value.block_idx * BLOCK_SIZE) as u32,
            length: BLOCK_SIZE as u32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Message, Piece, Request};
    use crate::data_structures::SerializableBuf;

    fn long_buf_from_message_slice(raw_message: &[u8]) -> [u8; 1 << 6] {
        let mut buf = [0u8; 1 << 6];
        buf[..raw_message.len()].copy_from_slice(raw_message);
        buf
    }

    fn test_message_ser_de(raw_message: &[u8], expected_message: Message) {
        let buf = long_buf_from_message_slice(raw_message);

        let message = Message::from_buf(&buf).unwrap();

        assert_eq!(expected_message, message);

        let bytes = message.into_bytes();

        assert_eq!(&buf[..raw_message.len()], &bytes);
    }

    #[test]
    fn keep_alive() {
        let raw_message = [0, 0, 0, 0];
        let message = Message::KeepAlive;
        test_message_ser_de(&raw_message, message);
    }

    #[test]
    fn choke() {
        let raw_message = [0, 0, 0, 1, 0];
        let message = Message::Choke;
        test_message_ser_de(&raw_message, message);
    }

    #[test]
    fn unchoke() {
        let raw_message = [0, 0, 0, 1, 1];
        let message = Message::Unchoke;
        test_message_ser_de(&raw_message, message);
    }

    #[test]
    fn interested() {
        let raw_message = [0, 0, 0, 1, 2];
        let message = Message::Interested;
        test_message_ser_de(&raw_message, message);
    }

    #[test]
    fn not_interested() {
        let raw_message = [0, 0, 0, 1, 3];
        let message = Message::NotInterested;
        test_message_ser_de(&raw_message, message);
    }

    #[test]
    fn have() {
        let piece_index: u32 = 726049813;

        let mut raw_message = [0, 0, 0, 5, 4, 0, 0, 0, 0];
        raw_message[5..9].copy_from_slice(&piece_index.to_be_bytes());

        let message = Message::Have(piece_index);

        test_message_ser_de(&raw_message, message);
    }

    #[test]
    fn bitfield() {
        let raw_message = [0, 0, 0, 8, 5, 23, 113, 254, 203, 0, 17, 224];

        let bitfield = &raw_message[5..];

        let message = Message::Bitfield(SerializableBuf::from(bitfield));

        test_message_ser_de(&raw_message, message);
    }

    #[test]
    fn request() {
        let index: u32 = 726049813;
        let begin: u32 = 3456;
        let length: u32 = 11166679;

        let mut raw_message = [0, 0, 0, 13, 6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        raw_message[5..9].copy_from_slice(&index.to_be_bytes());
        raw_message[9..13].copy_from_slice(&begin.to_be_bytes());
        raw_message[13..17].copy_from_slice(&length.to_be_bytes());

        let message = Message::Request(Request {
            index,
            begin,
            length,
        });

        test_message_ser_de(&raw_message, message);
    }

    #[test]
    fn piece() {
        let index: u32 = 726049813;
        let begin: u32 = 3456;

        let mut raw_message = [
            0, 0, 0, 20, 7, 0, 0, 0, 0, 0, 0, 0, 0, 247, 251, 239, 152, 196, 66, 34, 33, 90, 29, 97,
        ];
        raw_message[5..9].copy_from_slice(&index.to_be_bytes());
        raw_message[9..13].copy_from_slice(&begin.to_be_bytes());

        let block = SerializableBuf::from(&raw_message[13..]);

        let message = Message::Piece(Piece {
            index,
            begin,
            block,
        });

        test_message_ser_de(&raw_message, message);
    }

    #[test]
    fn cancel() {
        let index: u32 = 726049813;
        let begin: u32 = 3456;
        let length: u32 = 11166679;

        let mut raw_message = [0, 0, 0, 13, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        raw_message[5..9].copy_from_slice(&index.to_be_bytes());
        raw_message[9..13].copy_from_slice(&begin.to_be_bytes());
        raw_message[13..17].copy_from_slice(&length.to_be_bytes());

        let message = Message::Cancel(Request {
            index,
            begin,
            length,
        });

        test_message_ser_de(&raw_message, message);
    }

    #[test]
    fn port() {
        let piece_index = 45678u16;
        let left = (piece_index >> 8) as u8;
        let right = (piece_index % (1 << 8)) as u8;

        let raw_message = [0, 0, 0, 3, 9, left, right];

        let message = Message::Port(piece_index);

        test_message_ser_de(&raw_message, message);
    }

    #[test]
    fn incomplete_message() {
        let raw_message = [
            0, 0, 0, 69, 7, 1, 2, 3, 4, 5, 6, 7, 8, 247, 251, 239, 152, 196, 66, 34, 33, 90,
        ];

        let buf = long_buf_from_message_slice(&raw_message);

        assert_eq!(None, Message::from_buf(&buf));
    }
}
