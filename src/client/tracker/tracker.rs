use super::announce::{Announce, AnnounceEvent};
use super::response::Response;
use crate::client::Peers;
use crate::data_structures::ID;
use crate::shutdown;
use crate::transcoding::metainfo::Torrent;
use anyhow::{anyhow, Result};
use bendy::decoding::{Decoder, FromBencode};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast::{self, Receiver};
use tokio::time::{sleep, Duration};
use tracing::{debug, instrument, trace, warn};

const TIMEOUT_SECS: u64 = 5;

pub fn spawn_tracker_managers(
    torrent: &Torrent,
    peer_id: &ID,
    peers: &Peers,
    pieces_downloaded: Arc<AtomicU64>,
    download_completed_tx: &broadcast::Sender<()>,
    tcp_port: u16,
    shutdown_rx: shutdown::Receiver,
) {
    for http_tracker in torrent.http_trackers() {
        let tracker_url = http_tracker.clone();
        let info_hash = torrent.info_hash.clone();
        let peer_id = peer_id.clone();
        let piece_count = torrent.count_pieces() as u64;
        let piece_length = torrent.info.piece_length;
        let pieces_downloaded = pieces_downloaded.clone();
        let peers = peers.clone();
        let announce_event_message = download_completed_tx.subscribe();
        let shutdown = shutdown_rx.clone();

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
                shutdown,
            )
            .await
        });
    }
}

#[instrument(skip_all, fields(tracker = tracker_url))]
async fn manage_http_tracker(
    tracker_url: String,
    info_hash: ID,
    peer_id: ID,
    piece_count: u64,
    piece_length: u64,
    pieces_downloaded: Arc<AtomicU64>,
    mut peers: Peers,
    mut announce_event_message: Receiver<()>, // this can be extended for pause feature
    tcp_port: u16,
    mut shutdown_rx: shutdown::Receiver,
) {
    trace!("spawn tracker");
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

    while let Some(response) = contact_tracker(&announce, &mut shutdown_rx).await {
        peers.extend(&response.peers);

        if announce.tracker_id.is_none() && response.tracker_id.is_some() {
            announce.tracker_id = response.tracker_id.clone();
        }

        tokio::select! {
            _ = shutdown_rx.recv() => {
                debug!("shutdown");
                announce.event = Some(AnnounceEvent::Stopped);
                announce.left = (piece_count - pieces_downloaded.load(Ordering::Relaxed)) * piece_length;
                // announce.uploaded = TODO
                // announce.downloaded = TODO
                let _ = contact_tracker(&announce, &mut shutdown_rx).await;
                break;
            },
            _ = sleep(Duration::from_secs(response.interval)) => {
                announce.event = None;
            }
            downloaded = announce_event_message.recv() => {
                match downloaded {
                    Ok(_) => {
                        announce.event = Some(AnnounceEvent::Completed);
                    },
                    Err(e) => {
                        warn!(?e);
                        break;
                    },
                }
            }
        }

        announce.left = (piece_count - pieces_downloaded.load(Ordering::Relaxed)) * piece_length;
        // announce.uploaded = TODO
        // announce.downloaded = TODO
    }

    trace!("exit");
}

async fn contact_tracker(
    announce: &Announce,
    shutdown_rx: &mut shutdown::Receiver,
) -> Option<Response> {
    let announce_url = announce.as_url();

    loop {
        let response = tokio::select! {
            _ = shutdown_rx.recv() => {
                debug!("shutdown");
                return None;
            },
            response = send_request(&announce_url) => {response}
        };

        match response {
            Ok(resp) => return Some(resp),
            Err(e) => warn!(?e),
        }
    }
}

async fn send_request(url: &str) -> Result<Response> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(TIMEOUT_SECS))
        .build()?;

    let response = client.get(url).send().await?;
    let body = response.bytes().await?;

    let mut decoder = Decoder::new(body.as_ref());
    let decoder_object = decoder
        .next_object()?
        .ok_or(anyhow!("response from tracker is empty"))?;

    let decoded_response = Response::decode_bencode_object(decoder_object)?;

    debug!(?decoded_response);

    Ok(decoded_response)
}
