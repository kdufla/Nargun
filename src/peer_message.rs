use anyhow::Result;
use bincode::Options;
use bytes::Bytes;
use serde::ser::SerializeTuple;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::mem::size_of;
use tokio::io::AsyncReadExt;
use tokio::io::ReadHalf;
use tokio::net::TcpStream;

#[derive(Debug)]
pub struct SerializableBytes(Bytes);

impl SerializableBytes {
    pub fn new(data: Bytes) -> SerializableBytes {
        SerializableBytes(data)
    }

    pub fn into_bytes(self) -> Bytes {
        self.0
    }

    pub fn as_bytes(&self) -> Bytes {
        self.0.clone()
    }
}

impl Serialize for SerializableBytes {
    fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        s.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for SerializableBytes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = SerializableBytes;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("Compact <ip=4><port=2> bytes")
            }

            fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(SerializableBytes(Bytes::from(Vec::from(v))))
            }
        }

        Ok(deserializer.deserialize_byte_buf(Visitor {})?)
    }
}

#[derive(Debug, Serialize)]
pub struct Request {
    pub index: u32,
    pub begin: u32,
    pub length: u32,
}

// TODO manually deserialize this or make NoSizeArray deserialize
#[derive(Debug, Serialize)]
pub struct Piece {
    pub index: u32,
    pub begin: u32,
    pub block: SerializableBytes,
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
    Port(u16),
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
                    seq.serialize_element(byte)?; // TODO this is most likely wrong because of the len 3 tuple
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
                    seq.serialize_element(byte)?; // TODO this is most likely wrong because of the len 3 tuple
                }
                seq.end()
            }
            Message::Port(listen_port) => {
                let mut seq = serializer.serialize_tuple(3)?;
                seq.serialize_element(&(3 as u32))?;
                seq.serialize_element(&(9 as u8))?;
                seq.serialize_element(listen_port)?;
                seq.end()
            }
        }
    }
}

macro_rules! read_n_into_buffer_or_err {
    ($stream:expr, $n:expr, $buffer:expr) => {{
        let mut read_pointer = 0;
        loop {
            let read_bytes = $stream.read(&mut $buffer[read_pointer..$n]).await?;
            read_pointer += read_bytes;

            if read_bytes == 0 {
                anyhow::bail!("disconnected")
            }

            if read_pointer == $n {
                break;
            }
        }
    }};
}

macro_rules! read_type_or_err {
    ($stream:expr, $t:ty) => {{
        let mut buffer = [0 as u8; size_of::<$t>()];
        read_n_into_buffer_or_err!($stream, size_of::<$t>(), buffer);
        <$t>::from_be_bytes(buffer)
    }};
}

macro_rules! slice_as_u32_be {
    ($slice:expr) => {
        (($slice[0] as u32) << 24)
            + (($slice[1] as u32) << 16)
            + (($slice[2] as u32) << 8)
            + (($slice[3] as u32) << 0)
    };
}

impl Message {
    pub async fn from_stream<'a>(stream: &mut ReadHalf<&mut TcpStream>) -> Result<Self> {
        let len = read_type_or_err!(stream, u32);
        print!("received with len={:?} ", len);

        if len == 0 {
            println!("KeepAlive");
            Ok(Message::KeepAlive)
        } else {
            let id = read_type_or_err!(stream, u8);
            println!("id={:?}", id);

            Ok(match id {
                0 => Message::Choke,
                1 => Message::Unchoke,
                2 => Message::Interested,
                3 => Message::NotInterested,
                4 => Message::Have(read_type_or_err!(stream, u32)),
                5 => {
                    let len = len as usize - 1;
                    let mut buf = vec![0 as u8; len];

                    read_n_into_buffer_or_err!(stream, len, &mut buf);

                    Message::Bitfield(SerializableBytes(Bytes::from(buf)))
                }
                6 => {
                    let mut buf = [0 as u8; 3 * 4];
                    read_n_into_buffer_or_err!(stream, 3 * 4, buf);

                    Message::Request(Request {
                        index: slice_as_u32_be!(buf[..]),
                        begin: slice_as_u32_be!(buf[4..]),
                        length: slice_as_u32_be!(buf[8..]),
                    })
                }

                7 => {
                    let len = len as usize - 1;
                    let mut buf = vec![0 as u8; len];

                    read_n_into_buffer_or_err!(stream, len, &mut buf);

                    let mut buf = Bytes::from(buf);

                    let index_buf = buf.split_to(4);
                    let begin_buf = buf.split_to(4);

                    Message::Piece(Piece::new(
                        slice_as_u32_be!(index_buf[..]),
                        slice_as_u32_be!(begin_buf[..]),
                        buf,
                    ))
                } // TODO why did I not implement an 8 (cancel)?
                9 => Message::Port(read_type_or_err!(stream, u16)),
                _ => unimplemented!(), // TODO this might cause problems. on random wrong message read pointer is stuck on a random place.
                                       // it might jump over the actual message. I should probably check message validity byte-by-byte with peek.
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
