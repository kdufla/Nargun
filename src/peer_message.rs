use anyhow::Result;
use bincode::Options;
use bytes::Bytes;
use serde::ser::SerializeTuple;
use serde::ser::Serializer;
use serde::Serialize;
use std::mem::size_of;

#[derive(Debug)]
pub struct SerializableBytes(Bytes);

impl serde::Serialize for SerializableBytes {
    fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut tuple = s.serialize_tuple(self.0.len())?;
        for byte in self.0.iter() {
            tuple.serialize_element(byte)?;
        }
        tuple.end()
    }
}

#[derive(Debug, Serialize)]
pub struct Request {
    index: u32,
    begin: u32,
    length: u32,
}

// TODO manually deserialize this or make NoSizeArray deserialize
#[derive(Debug, Serialize)]
pub struct Piece {
    index: u32,
    begin: u32,
    block: SerializableBytes,
}

impl Piece {
    pub fn new(index: u32, begin: u32, block: Bytes) -> Piece {
        Piece {
            index,
            begin,
            block: SerializableBytes(block),
        }
    }

    pub fn len(&self) -> usize {
        self.block.0.len()
    }
}

#[derive(Debug)]
pub enum Message {
    KeepAlive,
    Choke,
    Unchoke,
    Interested,
    NotInterested,
    Have(u32),
    Bitfield(SerializableBytes),
    Request(Request),
    Piece(Piece),
}

impl Serialize for Message {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Message::KeepAlive => serializer.serialize_u32(0),
            Message::Choke => {
                let mut seq = serializer.serialize_tuple(2)?;
                seq.serialize_element(&(1 as u32))?;
                seq.serialize_element(&(0 as u8))?;
                seq.end()
            }
            Message::Unchoke => {
                let mut seq = serializer.serialize_tuple(2)?;
                seq.serialize_element(&(1 as u32))?;
                seq.serialize_element(&(1 as u8))?;
                seq.end()
            }
            Message::Interested => {
                let mut seq = serializer.serialize_tuple(2)?;
                seq.serialize_element(&(1 as u32))?;
                seq.serialize_element(&(2 as u8))?;
                seq.end()
            }
            Message::NotInterested => {
                let mut seq = serializer.serialize_tuple(2)?;
                seq.serialize_element(&(1 as u32))?;
                seq.serialize_element(&(3 as u8))?;
                seq.end()
            }
            Message::Have(piece_index) => {
                let mut seq = serializer.serialize_tuple(3)?;
                seq.serialize_element(&(1 as u32))?;
                seq.serialize_element(&(4 as u8))?;
                seq.serialize_element(&piece_index)?;
                seq.end()
            }
            Message::Bitfield(bitfield) => {
                let mut seq = serializer.serialize_tuple(3)?;
                seq.serialize_element(&(1 + bitfield.0.len() as u32))?;
                seq.serialize_element(&(5 as u8))?;
                for byte in bitfield.0.iter() {
                    seq.serialize_element(byte)?;
                }
                seq.end()
            }
            Message::Request(request) => {
                let mut seq = serializer.serialize_tuple(3)?;
                seq.serialize_element(&(13 as u32))?;
                seq.serialize_element(&(6 as u8))?;
                seq.serialize_element(&request)?;
                seq.end()
            }
            Message::Piece(piece) => {
                let mut seq = serializer.serialize_tuple(3)?;
                seq.serialize_element(&(9 + piece.len() as u32))?;
                seq.serialize_element(&(7 as u8))?;
                seq.serialize_element(&piece.index)?;
                seq.serialize_element(&piece.begin)?;
                for byte in piece.block.0.iter() {
                    seq.serialize_element(byte)?;
                }
                seq.end()
            }
        }
    }
}

macro_rules! next_element {
    ($buff:expr, $t:ty) => {{
        <$t>::from_be_bytes($buff.split_to(size_of::<$t>()).as_ref().try_into().unwrap())
    }};
}

impl Message {
    pub fn from_bytes(mut buff: Bytes) -> Result<Message> {
        let len = next_element!(buff, u32);

        if len == 0 {
            Ok(Message::KeepAlive)
        } else {
            let id = next_element!(buff, u8);

            Ok(match id {
                0 => Message::Choke,
                1 => Message::Unchoke,
                2 => Message::Interested,
                3 => Message::NotInterested,
                4 => Message::Have(next_element!(buff, u32)),
                5 => Message::Bitfield(SerializableBytes(buff)),
                6 => Message::Request(Request {
                    index: next_element!(buff, u32),
                    begin: next_element!(buff, u32),
                    length: next_element!(buff, u32),
                }),
                7 => Message::Piece(Piece::new(
                    next_element!(buff, u32),
                    next_element!(buff, u32),
                    buff,
                )),
                _ => unimplemented!(),
            })
        }
    }

    pub fn into_bytes(self) -> Result<Bytes> {
        Ok(Bytes::from(
            bincode::DefaultOptions::new()
                .with_big_endian()
                .with_fixint_encoding()
                .serialize(&self)?,
        ))
    }
}
