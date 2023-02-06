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
use tracing::log::trace;

const ANNOUNCE_RETRY: usize = 3;

async fn fetch_response(url: &String) -> Result<Response> {
    let response = reqwest::get(url).await?;
    let bytes = response.bytes().await?;

    let mut decoder = Decoder::new(bytes.as_ref());
    let decoder_object = decoder
        .next_object()?
        .ok_or(anyhow!("response from tracker is empty"))?;
    let tracker_response = Response::decode_bencode_object(decoder_object)?;

    Ok(tracker_response)
}

async fn contact_tracker(announce: &Announce) -> Result<Response> {
    let announce_url = announce.as_url();

    let mut tracker_response = fetch_response(&announce_url).await;

    if tracker_response.is_err() {
        for _ in 0..ANNOUNCE_RETRY {
            tracker_response = fetch_response(&announce_url).await;

            if tracker_response.is_ok() {
                break;
            }
        }
    }

    trace!("{:?}", tracker_response);
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
    trace!("spawn tracker: {tracker_url}");
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
    for tracker in torrent.http_trackers() {
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
