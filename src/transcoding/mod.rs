pub mod metainfo;

use anyhow::{bail, Result};
use std::net::{Ipv4Addr, SocketAddrV4};

pub const COMPACT_SOCKADDR_LEN: usize = 6;

pub fn socketaddr_from_compact_bytes(buf: &[u8]) -> Result<SocketAddrV4> {
    match buf.len() {
        COMPACT_SOCKADDR_LEN => Ok(SocketAddrV4::new(
            Ipv4Addr::new(buf[0], buf[1], buf[2], buf[3]),
            ((buf[4] as u16) << 8) | buf[5] as u16,
        )),
        _ => bail!(
            "socketaddr_from_compact_bytes: buffer len expected {} found {}",
            COMPACT_SOCKADDR_LEN,
            buf.len()
        ),
    }
}
