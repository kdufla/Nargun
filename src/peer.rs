use crate::constants;

use core::fmt;
use std::net::SocketAddr;

use anyhow::Result;
use bincode::Options;
use bytes::Bytes;
use serde::de::{SeqAccess, Visitor};
use serde::ser::SerializeTuple;
use serde::{Deserialize, Deserializer, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

#[derive(Serialize, Deserialize, Debug)]
struct Handshake {
    pstrlen: u8,
    pstr: Pstr,
    reserved: [u8; 8],
    info_hash: [u8; 20],
    peer_id: [u8; 20],
}

impl Handshake {
    fn new(info_hash: [u8; 20], peer_id: [u8; 20]) -> Handshake {
        Handshake {
            pstrlen: constants::PSTRLEN,
            pstr: Pstr(constants::PSTR.to_string()),
            reserved: [0; 8],
            info_hash: info_hash,
            peer_id: peer_id,
        }
    }

    pub fn from_bytes(buff: Bytes) -> Result<Handshake> {
        Ok(bincode::DefaultOptions::new()
            .with_big_endian()
            .with_fixint_encoding()
            .deserialize(buff.as_ref())?)
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

#[derive(Debug)]
pub struct Pstr(String);

impl serde::Serialize for Pstr {
    fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut tuple = s.serialize_tuple(19)?;
        for byte in self.0.as_bytes().iter() {
            tuple.serialize_element(byte)?;
        }
        tuple.end()
    }
}

impl<'de> Deserialize<'de> for Pstr {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PstrVisitor;

        impl<'de> Visitor<'de> for PstrVisitor {
            type Value = Pstr;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("Pstr")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut res = String::with_capacity(19);

                for _ in 0..19 {
                    res.push(
                        seq.next_element()?
                            .ok_or_else(|| serde::de::Error::invalid_length(1, &self))?,
                    );
                }

                return Ok(Pstr(res));
            }
        }

        return Ok(deserializer.deserialize_tuple(1 << 16, PstrVisitor)?);
    }
}

pub async fn initiate_handshake(
    sa: &SocketAddr,
    info_hash: &[u8; 20],
    peer_id: &[u8; 20],
) -> Result<TcpStream> {
    let mut stream = TcpStream::connect(sa).await?;

    let handshake = Handshake::new(info_hash.clone(), peer_id.clone());

    stream.write_all(handshake.into_bytes()?.as_ref()).await?;

    Ok(stream)
}
