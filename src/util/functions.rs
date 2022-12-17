use std::{
    fmt::Display,
    net::{Ipv4Addr, SocketAddrV4},
};

use anyhow::{bail, Result};

use crate::constants::SIX;

use super::id::ID;

pub fn ok_or_missing_field<T>(
    opt: Option<T>,
    field_name: impl Display,
) -> Result<T, bendy::decoding::Error> {
    opt.ok_or_else(|| bendy::decoding::Error::missing_field(field_name))
}

pub fn vec_to_id(v: Vec<u8>) -> ID {
    // TODO should not unwrap, improve error handling
    ID(v.try_into().unwrap())
}

pub fn socketaddr_from_compact_bytes(buff: &[u8]) -> Result<SocketAddrV4> {
    match buff.len() {
        SIX => Ok(SocketAddrV4::new(
            Ipv4Addr::new(buff[0], buff[1], buff[2], buff[3]),
            ((buff[4] as u16) << 8) | buff[5] as u16,
        )),
        _ => bail!(
            "socketaddr_from_compact_bytes: buffer len expected {} found {}",
            SIX,
            buff.len()
        ),
    }
}
