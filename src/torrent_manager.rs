use crate::data_structures::{Bitmap, ID};
use crate::fs::FS;
use crate::peers::active_peers::{Peers, Status as PeerStatus};
use crate::peers::connection::connection_handle::Connection;
use crate::peers::connection::connection_manager::{ConMessageType, ConnectionMessage};
use crate::peers::peer::Peer;
use crate::transcoding::metainfo::Torrent;
use crate::{shutdown, tracker};
use std::collections::hash_map::Entry::Occupied;
use std::collections::HashMap;
use std::net::SocketAddrV4;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::{select, sync::mpsc, time::interval};
use tracing::{debug, error, instrument, warn};

const MAX_ACTIVE_PEERS: usize = 10; // TODO this should come from config
const MAX_ACTIVE_PIECES: usize = 9; // TODO this should come from config
const MAX_PIECE_PER_PEER: usize = 4;
const MAX_PEER_PER_PIECE: usize = 3;

pub fn start_client(
    own_peer_id: ID,
    peers: Peers,
    dht_tx: mpsc::Sender<SocketAddrV4>,
    torrent: Torrent,
    pieces_downloaded: Arc<AtomicU64>,
    tx: &broadcast::Sender<()>,
    tcp_port: u16,
    shutdown_rx: shutdown::Receiver,
) {
    tracker::spawn_tracker_managers(
        &torrent,
        &own_peer_id,
        &peers,
        pieces_downloaded,
        tx,
        tcp_port,
        shutdown_rx,
    );

    TorrentManager::spawn(torrent, own_peer_id, peers, dht_tx);
}

// TODO ideally this must have its own Torrent with its peers and shit. Currently, my dht assumes just one torrent.
// I know what you're thinking, future G, and no - I can't. Port message needs to know that there's a dht server running.
// I can't put a port in dht if I don't, at least partially, assume the same. I'll try my best to make your life easy.
pub struct TorrentManager {
    metainfo: Torrent,
    connection: Connection,
    fs: FS,
    active_peers: HashMap<Peer, PeerInfo>,
    inactive_peers: Vec<(Peer, PeerStatus)>,
    peers: Peers,
    own_peer_id: ID,
    local_data: Bitmap,
    active_pieces: Bitmap,
    frequency_map: Vec<u16>,
    assigned_peer_count: Vec<u8>,
}

struct PeerInfo {
    choking: bool,
    interested: bool,
    haves: Bitmap,
    working: Bitmap,
}

impl PeerInfo {
    fn new(piece_count: usize) -> Self {
        Self {
            choking: true,
            interested: false,
            haves: Bitmap::new(piece_count),
            working: Bitmap::new(piece_count),
        }
    }
}

impl TorrentManager {
    pub fn spawn(
        metainfo: Torrent,
        own_peer_id: ID,
        peers: Peers,
        dht_tx: mpsc::Sender<SocketAddrV4>,
    ) {
        tokio::spawn(async move {
            // let metainfo = Arc::new(metainfo);
            // let x = metainfo.borrow();
            Self {
                connection: Connection::new(
                    own_peer_id,
                    metainfo.info_hash,
                    metainfo.info.piece_length as usize,
                    dht_tx,
                ),
                fs: FS::new("/home/gvelesa/rust/narpath", metainfo.info.to_owned()).await,
                active_peers: HashMap::with_capacity(MAX_ACTIVE_PEERS),
                inactive_peers: Vec::with_capacity(MAX_ACTIVE_PEERS),
                peers,
                own_peer_id,
                local_data: Bitmap::new(metainfo.count_pieces()),
                active_pieces: Bitmap::new(metainfo.count_pieces()),
                frequency_map: vec![0; metainfo.count_pieces()],
                assigned_peer_count: vec![0; metainfo.count_pieces()],
                metainfo,
            }
            .manage()
            .await
        });
    }

    #[instrument(skip_all)]
    async fn manage(mut self) {
        let mut interval = interval(Duration::from_secs(1));
        interval.tick().await;

        loop {
            select! {
                msg = self.connection.recv() => { self.handle_command_tmp(msg) },
                _ = interval.tick() => {
                    self.refresh_peer_list();
                    self.start_downloading_new_pieces().await;
                },
            };
        }
    }

    fn refresh_peer_list(&mut self) {
        let free_peer_slots = MAX_ACTIVE_PEERS - self.active_peers.len();

        if free_peer_slots == 0 {
            return;
        }

        let new_peers = self
            .peers
            .return_batch_of_bad_peers_and_get_new_batch(&self.inactive_peers, free_peer_slots);

        self.inactive_peers.clear();

        for peer in new_peers {
            self.connection.connect(peer);

            self.active_peers
                .insert(peer, PeerInfo::new(self.local_data.len()));
        }
    }

    async fn start_downloading_new_pieces(&mut self) {
        for _ in 0..(MAX_ACTIVE_PIECES - self.active_pieces.weight()) {
            let Some((peer, piece_idx, piece_hash)) = self.select_piece() else {
                break;
            };

            self.start_downloading_piece(piece_idx, piece_hash, peer)
                .await;
        }
    }

    async fn start_downloading_piece(&mut self, piece_idx: usize, piece_hash: ID, peer: Peer) {
        match self
            .connection
            .download_piece(peer, piece_idx, piece_hash)
            .await
        {
            Err(e) => error!(?e),
            Ok(_) => {
                self.mark_piece_as_ongoing(peer, piece_idx);
            }
        }
    }

    fn mark_piece_as_ongoing(&mut self, peer: Peer, piece_idx: usize) {
        let peer_info = self
            .active_peers
            .entry(peer)
            .or_insert(PeerInfo::new(self.active_pieces.len()));

        self.active_pieces.change(piece_idx, true);
        peer_info.working.change(piece_idx, true);
    }

    fn select_piece(&mut self) -> Option<(Peer, usize, ID)> {
        self.sequential_select_piece().map(|(peer, piece_idx)| {
            (
                peer,
                piece_idx,
                self.metainfo.info.pieces.get(piece_idx).unwrap().to_owned(),
            )
        })
    }

    // fn rarest_first_select_piece(&self) -> Option<(&Peer, usize)> {}
    // fn common_first_select_piece(&self) -> Option<(&Peer, usize)> {}

    fn iter_over_remaining_downloadable_pieces<'a>(
        &'a self,
    ) -> impl Iterator<Item = (usize, u16, u8)> + Clone + 'a {
        // for future G with question "did I forget to filter active pieces here?", NO!
        // active pieces can be assigned to other peers... that was why you changed pending pieces
        self.local_data
            .iter()
            .enumerate()
            .zip(self.frequency_map.iter())
            .zip(self.assigned_peer_count.iter())
            .filter(|(((_, downloaded), frequency), _)| !(*downloaded) && **frequency > 0)
            .map(|(((piece_idx, _), frequency), assigned_peers)| {
                (piece_idx, *frequency, *assigned_peers)
            })
    }

    fn sequential_select_piece(&self) -> Option<(Peer, usize)> {
        let iter = self.iter_over_remaining_downloadable_pieces();

        let mut iter_clone = iter.clone();

        let mut iteration_count = 0;
        let mut peers_per_piece = 0;
        let mut pieces_per_peer = 0;

        loop {
            let x = iter_clone.find(|(piece_idx, _, _)| {
                self.assigned_peer_count[*piece_idx] as usize == peers_per_piece
            });

            if let Some((piece_idx, _, _)) = x {
                if let Some(peer) = self.find_peer_with_piece(piece_idx, pieces_per_peer) {
                    break Some((peer.to_owned(), piece_idx));
                }
            }

            iter_clone = iter.clone();
            iteration_count += 1;
            pieces_per_peer = iteration_count % MAX_PIECE_PER_PEER;
            peers_per_piece = iteration_count / MAX_PIECE_PER_PEER;

            if iteration_count == MAX_PEER_PER_PIECE * MAX_PIECE_PER_PEER {
                break None;
            }
        }
    }

    fn find_peer_with_piece(&self, piece_idx: usize, pieces_per_peer: usize) -> Option<&Peer> {
        let (peer, _) = self.active_peers.iter().find(|(_, peer_info)| {
            !peer_info.choking
                && peer_info.haves.get(piece_idx)
                && peer_info.working.weight() == pieces_per_peer
        })?;

        Some(peer)
    }

    fn handle_command_tmp(&mut self, con_msg: ConnectionMessage) {
        let ConnectionMessage { message, peer } = con_msg;

        match message {
            ConMessageType::Unreachable => {
                self.mark_peer_as_inactive(peer, PeerStatus::UnableToConnect)
            }
            ConMessageType::Disconnected => self.mark_peer_as_inactive(peer, PeerStatus::Unknown),
            ConMessageType::KeepAlive => (),
            ConMessageType::Choke => {
                self.active_peers
                    .entry(peer)
                    .and_modify(|peer_info| peer_info.choking = true);
            }
            ConMessageType::Unchoke => {
                self.active_peers
                    .entry(peer)
                    .and_modify(|peer_info| peer_info.choking = false);
            }
            ConMessageType::Interested => {
                self.active_peers
                    .entry(peer)
                    .and_modify(|peer_info| peer_info.interested = true);
            }
            ConMessageType::NotInterested => {
                self.active_peers
                    .entry(peer)
                    .and_modify(|peer_info| peer_info.interested = false);
            }
            ConMessageType::Have(piece_idx) => {
                if let Occupied(mut peer_info) = self.active_peers.entry(peer) {
                    let old_have = peer_info.get_mut().haves.change(piece_idx as usize, true);
                    if !old_have {
                        self.frequency_map[piece_idx as usize] += 1;
                    }
                }
            }
            ConMessageType::Bitfield(bitfield) => {
                self.accept_bitfield_if_its_first(bitfield.as_ref(), peer)
            }
            ConMessageType::Request(_) => todo!(),
            ConMessageType::FinishedPiece(_) => todo!(),
            ConMessageType::Cancel(_) => todo!(),
        }
    }

    fn mark_peer_as_inactive(&mut self, peer: Peer, status: PeerStatus) {
        if let Some(peer_info) = self.active_peers.remove(&peer) {
            self.inactive_peers.push((peer, status));

            self.frequency_map
                .iter_mut()
                .zip(peer_info.haves.iter())
                .filter(|(_, peer_has_piece)| *peer_has_piece)
                .for_each(|(frequency, _)| *frequency -= 1)
        }
    }

    fn accept_bitfield_if_its_first(&mut self, new_data: &[u8], peer: Peer) {
        let Some(peer_info) = self.active_peers.get_mut(&peer) else {
            return;
        };

        let bitmap = &mut peer_info.haves;

        if bitmap.weight() > 0 {
            return;
        }

        if let Err(e) = bitmap.replace_data(new_data.as_ref()) {
            warn!(?e);
            return;
        }

        let bitmap = bitmap.clone();

        for (i, v) in bitmap.iter().enumerate() {
            if v {
                self.frequency_map[i] += 1;
            }
        }
    }
}
