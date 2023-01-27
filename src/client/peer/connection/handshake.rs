use crate::data_structures::ID;
use anyhow::{anyhow, bail, Result};
use std::mem::size_of;
use std::net::SocketAddrV4;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const PSTRLEN: u8 = 19;
const PSTR: &[u8] = b"BitTorrent protocol";
const HANDSHAKE_LENGTH_FOR_BITTORRENT_PROTOCOL: usize = 68;

pub async fn initiate_handshake(
    sock_addr: &SocketAddrV4,
    info_hash: &ID,
    peer_id: &ID,
) -> Result<TcpStream> {
    let mut stream = TcpStream::connect(sock_addr).await?;
    let handshake = Handshake::new(info_hash, peer_id);

    stream.write_all(handshake.as_ref()).await?;

    let mut buf = [0 as u8; HANDSHAKE_LENGTH_FOR_BITTORRENT_PROTOCOL];
    let received_byte_cnt = stream.read(&mut buf).await?;

    if received_byte_cnt != HANDSHAKE_LENGTH_FOR_BITTORRENT_PROTOCOL {
        Err(anyhow!("can't connect to peer"))
    } else {
        Handshake::check_validity(buf.as_ref())?;
        Ok(stream)
    }
}

#[derive(Debug)]
#[repr(C)]
struct Handshake {
    pstrlen: u8,
    pstr: [u8; PSTRLEN as usize],
    reserved: [u8; 8],
    info_hash: ID,
    peer_id: ID,
}

impl Handshake {
    fn new(info_hash: &ID, peer_id: &ID) -> Self {
        Self {
            pstrlen: PSTRLEN,
            pstr: PSTR.try_into().unwrap(),
            reserved: [0, 0, 0, 0, 0, 0, 0, 1],
            info_hash: info_hash.to_owned(),
            peer_id: peer_id.to_owned(),
        }
    }

    fn as_ref(&self) -> &[u8] {
        let ptr = self as *const Self;
        let ptr = ptr as *const u8;
        unsafe { std::slice::from_raw_parts(ptr, size_of::<Self>()) }
    }

    fn check_validity(buf: &[u8]) -> Result<()> {
        if buf.len() != HANDSHAKE_LENGTH_FOR_BITTORRENT_PROTOCOL {
            bail!(
                "handshake should be {} bytes long, got {}",
                HANDSHAKE_LENGTH_FOR_BITTORRENT_PROTOCOL,
                buf.len()
            );
        }

        let pstrlen = buf[0];
        let pstr = &buf[1..(PSTRLEN as usize + 1)];

        if pstrlen != PSTRLEN || pstr != PSTR {
            bail!("unsupported protocol");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Handshake, HANDSHAKE_LENGTH_FOR_BITTORRENT_PROTOCOL, PSTR, PSTRLEN};
    use crate::data_structures::{ID, ID_LEN};

    const INFO_HASH: [u8; ID_LEN] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
    const PEER_ID: [u8; ID_LEN] = [9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0];

    fn serialized_handshake() -> [u8; HANDSHAKE_LENGTH_FOR_BITTORRENT_PROTOCOL] {
        let mut rv = [0u8; HANDSHAKE_LENGTH_FOR_BITTORRENT_PROTOCOL];

        rv[0] = 19;

        let pstr = &mut rv[1..20];
        for (idx, byte) in b"BitTorrent protocol".iter().enumerate() {
            pstr[idx] = *byte;
        }

        for idx in 20..27 {
            rv[idx] = 0;
        }
        rv[27] = 1;

        let info_hash = &mut rv[28..48];
        for (idx, byte) in INFO_HASH.iter().enumerate() {
            info_hash[idx] = *byte;
        }

        let peer_id = &mut rv[48..];
        for (idx, byte) in PEER_ID.iter().enumerate() {
            peer_id[idx] = *byte;
        }

        rv
    }

    #[test]
    fn serialize() {
        let info_hash = ID::new(INFO_HASH);
        let peer_id = ID::new(PEER_ID);

        let raw = serialized_handshake();

        let handshake = Handshake::new(&info_hash, &peer_id);
        let serialized_handshake = handshake.as_ref();

        assert_eq!(
            HANDSHAKE_LENGTH_FOR_BITTORRENT_PROTOCOL,
            serialized_handshake.len()
        );
        assert_eq!(PSTRLEN, serialized_handshake[0]);
        assert_eq!(PSTR[0], serialized_handshake[1]);
        assert_eq!(0, serialized_handshake[20]);
        assert_eq!(INFO_HASH[0], serialized_handshake[28]);
        assert_eq!(PEER_ID[0], serialized_handshake[48]);

        assert_eq!(raw.as_ref(), serialized_handshake);
    }

    #[test]
    fn check_validity() {
        let raw = serialized_handshake();
        assert!(Handshake::check_validity(raw.as_ref()).is_ok());
    }

    #[test]
    #[should_panic]
    fn raw_handshake_with_wrong_size() {
        let raw = serialized_handshake();
        let _ = Handshake::check_validity(
            &raw[0..(HANDSHAKE_LENGTH_FOR_BITTORRENT_PROTOCOL as usize - 2)],
        )
        .unwrap();
    }

    #[test]
    #[should_panic]
    fn raw_handshake_with_wrong_protocol() {
        let mut raw = serialized_handshake();
        raw[17] = b'X';
        let _ = Handshake::check_validity(raw.as_ref()).unwrap();
    }
}
