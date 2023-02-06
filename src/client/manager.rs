use super::peer::connection::{
    ConMessageType, Connection, ConnectionMessage, FinishedPiece, ManagerChannels, PendingPieces,
};
use super::peer::{manage_peer, PeerManagerCommand};
use super::peer::{Peer, Peers, Status as PeerStatus};
use crate::data_structures::{Bitmap, ID};
use crate::transcoding::metainfo::Torrent;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::time::Duration;
use tokio::{select, sync::mpsc, time::interval};
use tracing::{debug, error, warn};

const MAX_ACTIVE_PEERS: usize = 10; // TODO this should come from config
const MAX_ACTIVE_PIECES: usize = 9; // TODO this should come from config
const MAX_PIECE_PER_PEER: usize = 4;
const MAX_PEER_PER_PIECE: usize = 3;

// TODO ideally this must have its own Torrent with its peers and shit. Currently, my dht assumes just one torrent.
// I know what you're thinking, future G, and no - I can't. Port message needs to know that there's a dht server running.
// I can't put a port in dht if I don't, at least partially, assume the same. I'll try my best to make your life easy.
pub struct TorrentManager {
    metainfo: Torrent,
    active_peers: HashMap<Peer, PeerInfo>,
    inactive_peers: Vec<(Peer, PeerStatus)>,
    pending_pieces: PendingPieces,
    peers: Peers,
    client_id: ID,
    local_data: Bitmap,
    active_pieces: Bitmap,
    frequency_map: Vec<u16>,
    assigned_peer_count: Vec<u8>,
    finished_piece_tx: mpsc::Sender<FinishedPiece>,
    dht_tx: mpsc::Sender<ConnectionMessage>,
    self_tx: mpsc::Sender<ConnectionMessage>,
}

struct PeerInfo {
    choking: bool,
    haves: Bitmap,
    working: Bitmap,
    manager_tx: mpsc::Sender<PeerManagerCommand>,
}

impl TorrentManager {
    pub fn spawn(
        metainfo: Torrent,
        client_id: ID,
        peers: Peers,
        dht_tx: mpsc::Sender<ConnectionMessage>,
    ) {
        let (self_tx, self_rx) = mpsc::channel(1 << 5);
        let (finished_piece_tx, finished_piece_rx) = mpsc::channel(1 << 4);

        tokio::spawn(async move {
            Self {
                pending_pieces: PendingPieces::new(metainfo.info.piece_length as usize),
                active_peers: HashMap::with_capacity(MAX_ACTIVE_PEERS),
                inactive_peers: Vec::with_capacity(MAX_ACTIVE_PEERS),
                peers,
                client_id,
                finished_piece_tx,
                local_data: Bitmap::new(metainfo.count_pieces()),
                active_pieces: Bitmap::new(metainfo.count_pieces()),
                frequency_map: vec![0; metainfo.count_pieces()],
                assigned_peer_count: vec![0; metainfo.count_pieces()],
                metainfo,
                dht_tx,
                self_tx,
            }
            .manage(self_rx, finished_piece_rx)
            .await
        });
    }

    async fn manage(
        mut self,
        mut self_rx: mpsc::Receiver<ConnectionMessage>,
        mut finished_piece_rx: mpsc::Receiver<FinishedPiece>,
    ) {
        let mut interval = interval(Duration::from_secs(1));
        interval.tick().await;

        loop {
            select! {
                _ = interval.tick() => { self.refresh_peer_list(); self.download_new_pieces().await; },
                command = self_rx.recv() => { self.handle_command_tmp(command) },
                finished_piece = finished_piece_rx.recv() => { debug!("finished {}", finished_piece.unwrap().idx) },
            };
        }
    }

    async fn download_new_pieces(&mut self) {
        for _ in 0..(MAX_ACTIVE_PIECES - self.active_pieces.weight()) {
            let Some((peer, piece_idx, piece_hash)) = self.select_piece() else {
                break;
            };

            self.download_new_piece(&peer, piece_idx, piece_hash).await;
        }
    }

    async fn download_new_piece(&mut self, peer: &Peer, piece_idx: usize, piece_hash: ID) {
        let Some(peer_info) = self.active_peers.get_mut(peer) else {
            warn!("this should not get called on non-active peer");
            return;
        };

        // TODO this will fail because MAX_ACTIVE_PIECES and MAX_PIECES_DOWNLOADING are separate
        let _ = self.pending_pieces.add_piece(piece_idx);

        if let Err(e) = peer_info
            .manager_tx
            .send(PeerManagerCommand::StartPiece((piece_hash, piece_idx)))
            .await
        {
            error!("peer({:?}) manager is dead. e={:?}", peer, e);
        }

        self.mark_piece_as_ongoing(peer, piece_idx);
    }

    fn mark_piece_as_ongoing(&mut self, peer: &Peer, piece_idx: usize) {
        let Some(peer_info) = self.active_peers.get_mut(peer) else {
            warn!("this should not get called on non-active peer");
            return;
        };

        self.active_pieces.change(piece_idx, true);
        peer_info.working.change(piece_idx, true);
    }

    fn count_available_pieces(&self) -> usize {
        self.frequency_map
            .iter()
            .zip(self.local_data.iter())
            .zip(self.active_pieces.iter())
            .filter(|((freq, local), active)| !local && !active && **freq > 0)
            .count()
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

    fn iter_over_downloadable_left_pieces<'a>(
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
        let iter = self.iter_over_downloadable_left_pieces();

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

    fn handle_command_tmp(&mut self, command: Option<ConnectionMessage>) {
        let Some(command) = command else {
            error!("impossible");
            return;
        };

        match command.message {
            ConMessageType::Unreachable
            | ConMessageType::Disconnected
            | ConMessageType::Choke
            | ConMessageType::Unchoke
            | ConMessageType::Interested
            | ConMessageType::NotInterested => self.handel_status(command),
            ConMessageType::Have(_) | ConMessageType::Bitfield(_) => {
                self.handel_remote_data(command)
            }
            _ => warn!("torrent manager received {:?}", command),
        }
    }

    fn handel_status(&mut self, message: ConnectionMessage) {
        match message.message {
            ConMessageType::Unreachable => {
                if let Some(peer_info) = self.active_peers.remove(&message.peer) {
                    self.subtract_from_frequency_map(&peer_info.haves);
                    self.inactive_peers
                        .push((message.peer, PeerStatus::UnableToConnect));
                }
            }
            ConMessageType::Disconnected => {
                if let Some(peer_info) = self.active_peers.remove(&message.peer) {
                    self.subtract_from_frequency_map(&peer_info.haves);
                    self.inactive_peers
                        .push((message.peer, PeerStatus::Unknown));
                }
            }
            ConMessageType::Choke => {
                self.active_peers
                    .entry(message.peer)
                    .and_modify(|peer_info| peer_info.choking = true);
            }
            ConMessageType::Unchoke => {
                self.active_peers
                    .entry(message.peer)
                    .and_modify(|peer_info| peer_info.choking = false);
            }
            ConMessageType::Interested => (), // TODO this needs so be implemented. for now, I'm going to just upload everything
            ConMessageType::NotInterested => (),
            _ => panic!("function is misused. status change handler can't handle other messages"),
        }
    }

    fn subtract_from_frequency_map(&mut self, bitmap: &Bitmap) {
        self.frequency_map
            .iter_mut()
            .zip(bitmap.iter())
            .for_each(|(frequency, val)| {
                if val {
                    *frequency -= 1;
                }
            })
    }

    fn handel_remote_data(&mut self, message: ConnectionMessage) {
        match &message.message {
            ConMessageType::Have(piece_idx) => {
                if let Entry::Occupied(mut entry) = self.active_peers.entry(message.peer) {
                    if !entry.get_mut().haves.change(*piece_idx as usize, true) {
                        self.frequency_map[*piece_idx as usize] += 1;
                    }
                }
            }
            ConMessageType::Bitfield(new_data) => {
                self.accept_bitfield_if_its_first(new_data.as_ref(), message.peer)
            }
            _ => {
                panic!("function is misused. this handler can only update state of peer's bitfield")
            }
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

    fn refresh_peer_list(&mut self) {
        let free_peer_slots = MAX_ACTIVE_PEERS - self.active_peers.len();

        if free_peer_slots == 0 {
            return;
        }

        let new_peers = self
            .peers
            .return_batch_of_bad_peers_and_get_new_batch(&self.inactive_peers, free_peer_slots);

        self.inactive_peers.clear();

        let mut new_peer_data_for_this_task = Vec::with_capacity(new_peers.len());
        let mut new_peer_data_for_peer_managers = Vec::with_capacity(new_peers.len());

        for peer in new_peers {
            let (peer_manager_tx, peer_manager_rx) = mpsc::channel(1 << 4);

            new_peer_data_for_this_task.push((
                peer,
                PeerInfo {
                    haves: Bitmap::new(self.local_data.len()),
                    working: Bitmap::new(self.local_data.len()),
                    choking: true,
                    manager_tx: peer_manager_tx,
                },
            ));

            new_peer_data_for_peer_managers.push((peer, peer_manager_rx));
        }

        self.spawn_peers(new_peer_data_for_peer_managers);

        self.active_peers.extend(new_peer_data_for_this_task);
    }

    fn spawn_peers(&self, peers: Vec<(Peer, mpsc::Receiver<PeerManagerCommand>)>) {
        for (peer, new_piece_rx) in peers {
            let (down_tx, download_manager_rx) = mpsc::channel(1 << 5);

            let managers = ManagerChannels {
                connection_status: self.self_tx.clone(),
                peer_bitmap: self.self_tx.clone(),
                downloader: down_tx,
                uploader: self.self_tx.clone(), //TODO tmp placeholder
                dht: self.dht_tx.clone(),
            };

            let connection = Connection::new(
                peer,
                self.client_id.to_owned(),
                self.metainfo.info_hash.to_owned(),
                managers.to_owned(),
            );

            let mut pending_pieces = self.pending_pieces.clone();
            pending_pieces.set_owner_peer(peer);

            let finished_piece_tx = self.finished_piece_tx.clone();

            tokio::spawn(async move {
                manage_peer(
                    new_piece_rx,
                    download_manager_rx,
                    pending_pieces,
                    finished_piece_tx,
                    connection,
                )
                .await;
            });
        }
    }
}
