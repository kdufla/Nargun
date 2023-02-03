use super::announce::{Announce, AnnounceEvent};
use super::response::Response;
use crate::client::Peers;
use crate::data_structures::ID;
use crate::transcoding::metainfo::Torrent;
use anyhow::{anyhow, Result};
use bendy::decoding::{Decoder, FromBencode};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast::{self, Receiver};
use tokio::time::{interval, Duration, MissedTickBehavior};

const ANNOUNCE_RETRY: usize = 3;

async fn fetch_response(url: &String) -> Result<Response> {
    // TODO lmao I didn't know about let-else
    match reqwest::get(url).await {
        Ok(response) => match response.bytes().await {
            Ok(bytes) => match Decoder::new(bytes.as_ref()).next_object() {
                Ok(decoder_object_option) => match decoder_object_option {
                    Some(decoder_object) => match Response::decode_bencode_object(decoder_object) {
                        Ok(tracker_response) => Ok(tracker_response),
                        Err(_) => Err(anyhow!("tracker response is not struct Response")),
                    },
                    None => Err(anyhow!("should be unreachable for the first object")),
                },
                Err(_) => Err(anyhow!("tracker response: not bencoded")),
            },
            Err(_) => Err(anyhow!("tracker response: without body")),
        },
        Err(_) => Err(anyhow!("tracker unreachable")),
    }
}

async fn contact_tracker(announce: &Announce) -> Result<Response> {
    let announce_url = announce.as_url();

    println!("contact: {announce_url}");

    let mut tracker_response = fetch_response(&announce_url).await;

    if tracker_response.is_err() {
        println!("retry: {}", announce.tracker_url);
        for _ in 0..ANNOUNCE_RETRY {
            tracker_response = fetch_response(&announce_url).await;

            if tracker_response.is_ok() {
                break;
            }
        }
    }

    println!("done: {} |||| {:?}", announce.tracker_url, tracker_response);
    tracker_response
}

async fn manage_http_tracker(
    tracker_url: String,
    info_hash: ID,
    peer_id: ID,
    piece_count: u64,
    piece_length: u64,
    pieces_downloaded: Arc<AtomicU64>,
    mut peers: Peers,
    mut announce_event_message: Receiver<bool>,
    tcp_port: u16,
) {
    println!("spawned {tracker_url}");
    let mut announce = Announce {
        tracker_url,
        info_hash,
        peer_id,
        port: tcp_port,
        uploaded: 0,
        downloaded: 0,
        left: (piece_count - pieces_downloaded.load(Ordering::Relaxed)) * piece_length,
        compact: true,
        no_peer_id: true,
        event: Some(AnnounceEvent::Started),
        tracker_id: None,
    };

    if let Ok(mut response) = contact_tracker(&announce).await {
        let mut interval = interval(Duration::from_secs(response.interval));
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        interval.tick().await;

        loop {
            peers.extend(&response.peers);

            if announce.tracker_id.is_none() && response.tracker_id.is_some() {
                announce.tracker_id = response.tracker_id.clone();
            }

            tokio::select! {
                _ = interval.tick() => {
                    announce.event = None;
                }
                downloaded = announce_event_message.recv() => {
                    if downloaded.is_ok() {
                        if downloaded.unwrap() {
                            announce.event = Some(AnnounceEvent::Completed);
                        } else {
                            announce.event = Some(AnnounceEvent::Stopped);
                            announce.left = (piece_count - pieces_downloaded.load(Ordering::Relaxed)) * piece_length;
                            // announce.uploaded = TODO
                            // announce.downloaded = TODO
                            let _ = contact_tracker(&announce).await;
                            break;
                        }
                    }
                }
            }

            announce.left =
                (piece_count - pieces_downloaded.load(Ordering::Relaxed)) * piece_length;
            // announce.uploaded = TODO
            // announce.downloaded = TODO

            if let Ok(r) = contact_tracker(&announce).await {
                response = r;
            }
        }
    }
}

pub fn spawn_tracker_managers(
    torrent: &Torrent,
    peer_id: &ID,
    peers: &Peers,
    pieces_downloaded: Arc<AtomicU64>,
    tx: &broadcast::Sender<bool>, // TODO tx? really? nice naming. figure out what this is. that's what you get by kicking the naming-bucket down the road
    tcp_port: u16,
) {
    println!("start");
    for tracker in torrent.http_trackers() {
        println!("start: {tracker}");
        let tracker_url = tracker.clone();
        let info_hash = torrent.info_hash.clone();
        let peer_id = peer_id.clone();
        let piece_count = torrent.count_pieces() as u64;
        let piece_length = torrent.info.piece_length;
        let pieces_downloaded = pieces_downloaded.clone();
        let peers = peers.clone();
        let announce_event_message = tx.subscribe();

        tokio::spawn(async move {
            manage_http_tracker(
                tracker_url,
                info_hash,
                peer_id,
                piece_count,
                piece_length,
                pieces_downloaded,
                peers,
                announce_event_message,
                tcp_port,
            )
            .await
        });
    }
}
