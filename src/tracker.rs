use anyhow::{anyhow, Result};
use bendy::decoding::{Decoder, FromBencode, Object};
use bendy::encoding::AsString;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;
use tokio::time::{sleep, Duration};

use crate::metainfo::{Torrent, TrackerAddr};
use crate::peer::Peers;

const ANNOUNCE_RETRY: u8 = 3;
const TRACKER_RETRY_INTERVAL: u64 = 300;

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

#[derive(Debug, Clone)]
pub struct TrackerResponse {
    pub warning_message: Option<String>,
    pub interval: u64,
    pub min_interval: Option<u64>,
    pub tracker_id: Option<String>,
    pub complete: u64,
    pub incomplete: u64,
    pub peers: Vec<SocketAddr>,
}

impl TrackerResponse {
    fn retry_dummy() -> TrackerResponse {
        TrackerResponse {
            warning_message: None,
            interval: TRACKER_RETRY_INTERVAL,
            min_interval: None,
            tracker_id: None,
            complete: 0,
            incomplete: 0,
            peers: Vec::<SocketAddr>::new(),
        }
    }
}

impl Announce {
    fn as_url(&self, tracker_url: &str) -> String {
        let mut s = String::from(tracker_url);

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

async fn fetch_response(url: &String) -> Result<TrackerResponse> {
    match reqwest::get(url).await {
        Ok(response) => match response.bytes().await {
            Ok(bytes) => match Decoder::new(bytes.as_ref()).next_object() {
                Ok(decoder_object_option) => match decoder_object_option {
                    Some(decoder_object) => {
                        match TrackerResponse::decode_bencode_object(decoder_object) {
                            Ok(tracker_response) => Ok(tracker_response),
                            Err(_) => {
                                Err(anyhow!("tracker response is not struct TrackerResponse"))
                            }
                        }
                    }
                    None => Err(anyhow!("should be unreachable for the first object")),
                },
                Err(_) => Err(anyhow!("tracker response: not bencoded")),
            },
            Err(_) => Err(anyhow!("tracker response: without body")),
        },
        Err(_) => Err(anyhow!("tracker unreachable")),
    }
}

pub async fn contact_tracker(
    a: Arc<Announce>,
    tracker: String,
    tx_tracker_response: Sender<(u32, String, TrackerResponse)>,
    id: u32,
) {
    let url = a.as_url(tracker.as_str());

    if cfg!(feature = "verbose") {
        println!("try announce {}", tracker);
    }

    let mut tracker_response = fetch_response(&url).await;

    if tracker_response.is_err() {
        if cfg!(feature = "verbose") {
            println!("retry announce {}", tracker);
        }
        for _ in 0..ANNOUNCE_RETRY {
            tracker_response = fetch_response(&url).await;

            if tracker_response.is_ok() {
                break;
            }
        }
    }

    match tracker_response {
        Ok(tracker_response) => {
            if cfg!(feature = "verbose") {
                println!("success announce {}", tracker);
            }
            tx_tracker_response
                .send((id, tracker, tracker_response))
                .await
                .unwrap();
        }
        Err(e) => {
            if cfg!(feature = "verbose") {
                println!("failed announce {}", tracker);
            }
            tx_tracker_response
                .send((id, tracker, TrackerResponse::retry_dummy()))
                .await
                .unwrap();
            println!("tracker_{} | {}", id, e);
        }
    }
}

pub async fn announce(
    torrent: &Torrent,
    peer_id: &[u8; 20],
    tx_tracker_response: Sender<(u32, String, TrackerResponse)>,
) {
    let a = Arc::new(Announce {
        info_hash: torrent.info_hash.clone(),
        peer_id: peer_id.clone(),
        port: 6887, // TODO
        uploaded: 0,
        downloaded: 0,
        left: torrent.info.length(),
        compact: true,
        no_peer_id: true,
        event: Some(AnnounceEvent::Started),
        tracker_id: None,
    });

    let mut count = 0;

    for tracker in torrent.announce.iter() {
        if let TrackerAddr::Http(http_tracker) = tracker {
            let a = a.clone();
            let http_tracker = http_tracker.clone();
            let tx_tracker_response = tx_tracker_response.clone();
            tokio::spawn(async move {
                contact_tracker(a, http_tracker, tx_tracker_response, count).await
            });
            count += 1;
        }
    }
}

fn owned_id_or_none(tracker: &TrackerResponse) -> Option<String> {
    match &tracker.tracker_id {
        Some(id) => Some(id.clone()),
        None => None,
    }
}

fn update_tracker_list(
    trackers: &mut Vec<Option<TrackerResponse>>,
    mut tr: TrackerResponse,
    idx: usize,
) {
    let old_response = std::mem::replace(&mut trackers[idx], None);

    tr.tracker_id = if tr.tracker_id.is_none() && old_response.is_some() {
        old_response.unwrap().tracker_id
    } else {
        None
    };

    trackers[idx] = Some(tr);
}

fn reannounce_after_interval(
    torrent: &Torrent,
    peer_id: &[u8; 20],
    tr: &TrackerResponse,
    tx_tracker_response: &Sender<(u32, String, TrackerResponse)>,
    tracker_url: String,
    id: u32,
) {
    let a = Arc::new(Announce {
        info_hash: torrent.info_hash.clone(),
        peer_id: peer_id.clone(),
        port: 6887,
        uploaded: 0, // TODO impl stats
        downloaded: 0,
        left: torrent.info.length(),
        compact: true,
        no_peer_id: true,
        event: None,
        tracker_id: owned_id_or_none(tr),
    });

    let sleep_mills = Duration::from_millis(1000 * tr.interval);

    if cfg!(feature = "verbose") {
        println!("announce {} in {:?}", tracker_url, sleep_mills);
    }

    let tx_tracker_response = tx_tracker_response.clone();
    tokio::spawn(async move {
        sleep(sleep_mills).await;
        contact_tracker(a, tracker_url, tx_tracker_response, id).await
    });
}

pub async fn tracker_manager(torrent: &Torrent, peer_id: &[u8; 20], peers: Peers) {
    let (tx_tracker_response, mut rx_tracker_response) = mpsc::channel(32);

    announce(torrent, peer_id, tx_tracker_response.clone()).await;

    let mut http_trackers: Vec<Option<TrackerResponse>> =
        vec![None; torrent.count_http_announcers()];

    loop {
        let tr = rx_tracker_response.recv().await;
        if let Some((id, tracker_url, tracker_response)) = tr {
            reannounce_after_interval(
                torrent,
                peer_id,
                &tracker_response,
                &tx_tracker_response,
                tracker_url,
                id,
            );

            peers.insert(&tracker_response.peers);

            update_tracker_list(&mut http_trackers, tracker_response, id as usize);
        } else {
            break;
        }
    }
}
