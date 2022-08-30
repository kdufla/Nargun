use anyhow::Result;
use bendy::decoding::{Decoder, FromBencode, Object};
use bendy::encoding::AsString;
use futures::future;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use crate::metainfo::{Torrent, TrackerAddr};

#[derive(Debug)]
pub enum AnnounceEvent {
    Started,
    Stopped,
    Completed,
}
#[derive(Debug)]
pub struct Announce {
    pub info_hash: [u8; 20],
    pub peer_id: [u8; 20],
    pub port: u16,
    pub uploaded: u64,
    pub downloaded: u64,
    pub left: u64,
    pub compact: bool,
    pub no_peer_id: bool,
    pub event: Option<AnnounceEvent>,
    pub tracker_id: Option<String>,
}

#[derive(Debug)]
pub struct TrackerResponse {
    pub warning_message: Option<String>,
    pub interval: u64,
    pub min_interval: Option<u64>,
    pub tracker_id: Option<String>,
    pub complete: u64,
    pub incomplete: u64,
    pub peers: Vec<SocketAddr>,
}

impl FromBencode for TrackerResponse {
    const EXPECTED_RECURSION_DEPTH: usize = 2;

    fn decode_bencode_object(object: Object) -> Result<Self, bendy::decoding::Error> {
        let mut warning_message = None;
        let mut interval = None;
        let mut min_interval = None;
        let mut tracker_id = None;
        let mut complete = None;
        let mut incomplete = None;
        let mut peers = None;

        let mut info = object.try_into_dictionary()?;
        while let Some(kv) = info.next_pair()? {
            match kv {
                (b"warning message", value) => {
                    warning_message = Some(String::decode_bencode_object(value)?);
                }
                (b"interval", value) => {
                    interval = Some(u64::decode_bencode_object(value)?);
                }
                (b"min interval", value) => {
                    min_interval = Some(u64::decode_bencode_object(value)?);
                }
                (b"tracker id", value) => {
                    tracker_id = Some(String::decode_bencode_object(value)?);
                }
                (b"complete", value) => {
                    complete = Some(u64::decode_bencode_object(value)?);
                }
                (b"incomplete", value) => {
                    incomplete = Some(u64::decode_bencode_object(value)?);
                }
                (b"peers", value) => {
                    if let Some(peer_bytes) =
                        AsString::decode_bencode_object(value).map(|bytes| Some(bytes.0))?
                    {
                        // let mut tmp = Vec::new();
                        // for chunk in peer_bytes.chunks_exact(6) {
                        //     // dbg!(chunk);
                        //     let ip =
                        //         IpAddr::V4(Ipv4Addr::new(chunk[0], chunk[1], chunk[2], chunk[3]));
                        //     let p = ((chunk[4] as u16) << 8) | chunk[5] as u16;
                        //     let sa = SocketAddr::new(ip, p);
                        //     tmp.push(sa);
                        // }
                        // dbg!(tmp);

                        peers = Some(
                            peer_bytes
                                .chunks_exact(6)
                                .map(|chunk| {
                                    SocketAddr::new(
                                        IpAddr::V4(Ipv4Addr::new(
                                            chunk[0], chunk[1], chunk[2], chunk[3],
                                        )),
                                        ((chunk[4] as u16) << 8) | chunk[5] as u16,
                                    )
                                })
                                .collect(),
                        );
                    }
                }
                _ => (),
            }
        }

        Ok(TrackerResponse {
            warning_message,
            interval: interval.ok_or_else(|| bendy::decoding::Error::missing_field("interval"))?,
            min_interval,
            tracker_id,
            complete: complete.ok_or_else(|| bendy::decoding::Error::missing_field("complete"))?,
            incomplete: incomplete
                .ok_or_else(|| bendy::decoding::Error::missing_field("incomplete"))?,
            peers: peers.ok_or_else(|| bendy::decoding::Error::missing_field("peers"))?,
        })
    }
}

pub async fn tmp_first_contact(a: Arc<Announce>, tracker: String) -> Option<TrackerResponse> {
    let url = a.as_url(tracker.as_str());

    let resp = reqwest::get(url).await.unwrap().bytes().await.unwrap();

    let mut decoder = Decoder::new(resp.as_ref());
    let obj = decoder.next_object().unwrap();

    match obj {
        Some(object) => Some(TrackerResponse::decode_bencode_object(object).unwrap()),
        None => None,
    }
}

// pub async fn announce(torrent: Torrent) -> Result<Vec<TrackerResponse>> {
pub async fn announce(torrent: Torrent) -> Result<()> {
    let a = Arc::new(Announce {
        info_hash: torrent.info_hash.clone(),
        peer_id: [1, 2, 3, 4, 5, 6, 7, 8, 9, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 0],
        port: 6887,
        uploaded: 0,
        downloaded: 0,
        left: torrent.info.length(),
        compact: true,
        no_peer_id: true,
        event: Some(AnnounceEvent::Started),
        tracker_id: None,
    });

    let handles = torrent.announce.iter().filter_map(|tracker| match tracker {
        TrackerAddr::Http(addr) => {
            let a = a.clone();
            let addr = addr.clone();
            dbg!(&addr);
            Some(tokio::spawn(
                async move { tmp_first_contact(a, addr).await },
            ))
        }
        _ => None,
    });

    let results: Vec<Option<TrackerResponse>> = future::join_all(handles)
        .await
        .into_iter()
        .flatten()
        .collect();

    // vec.into_iter().flatten().collect()

    dbg!(results);

    Ok(())
}

impl Announce {
    fn as_url(&self, url: &str) -> String {
        let mut s = String::from(url);

        s.push_str("?info_hash=");
        s.push_str(urlencoding::encode_binary(&self.info_hash).as_ref());

        s.push_str("&peer_id=");
        s.push_str(urlencoding::encode_binary(&self.peer_id).as_ref());

        s.push_str("&port=");
        s.push_str(&self.port.to_string());

        s.push_str("&uploaded=");
        s.push_str(&self.uploaded.to_string());

        s.push_str("&downloaded=");
        s.push_str(&self.downloaded.to_string());

        s.push_str("&left=");
        s.push_str(&self.left.to_string());

        s.push_str("&compact=");
        s.push_str(&(if self.compact { 1 } else { 0 }).to_string());

        if !self.compact {
            s.push_str("&no_peer_id=");
            s.push_str(&(if self.no_peer_id { 1 } else { 0 }).to_string());
        }

        if let Some(event) = &self.event {
            s.push_str("&event=");
            s.push_str(match event {
                AnnounceEvent::Started => "started",
                AnnounceEvent::Stopped => "stopped",
                AnnounceEvent::Completed => "completed",
            });
        }

        if let Some(tracker_id) = &self.tracker_id {
            s.push_str("&trackerid=");
            s.push_str(tracker_id);
        }

        s
    }
}
