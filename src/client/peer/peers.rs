use super::peer::Peer;
use crate::data_structures::ID;
use std::collections::HashMap;
use std::mem::discriminant;
use std::sync::{Arc, Mutex as StdMutex};

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    // TODO these need Instants and weighted sort based on elapsed time
    Active,
    Unknown,
    UnableToConnect,
    ChokedForTooLong,
    NoUsefulPieces,
}

#[derive(Clone, Debug)]
pub struct Peers {
    info_hash: ID,
    data: Arc<StdMutex<HashMap<Peer, Status>>>,
}

impl Peers {
    pub fn new(info_hash: &ID) -> Self {
        Self {
            info_hash: info_hash.to_owned(),
            data: Arc::new(StdMutex::new(HashMap::new())),
        }
    }

    // TODO this name is bad. `peers.serve(info_hash)` is clear but only seeing it here is laughably bad.
    pub fn serve(&self, info_hash: &ID) -> bool {
        *info_hash == self.info_hash
    }

    pub fn peer_addresses(&self) -> Vec<Peer> {
        self.data.lock().unwrap().keys().cloned().collect()
    }

    // I know this name/function is shit but I don't want to block peers more than it's necessary
    // TODO for later but it's chabuduo for now
    pub fn return_batch_of_bad_peers_and_get_new_batch(
        &mut self,
        bad_peers: &[(Peer, Status)],
        limit: usize,
    ) -> Vec<Peer> {
        let status_priority_order = [
            Status::Unknown,
            Status::NoUsefulPieces,
            Status::ChokedForTooLong,
            Status::UnableToConnect,
        ];

        let mut data = self.data.lock().unwrap();

        self.update_locked_data_with_bad_peers(&mut data, bad_peers);

        self.activate_peers_in_locked_data(&mut data, limit, &status_priority_order)
    }

    fn update_locked_data_with_bad_peers(
        &self,
        data: &mut HashMap<Peer, Status>,
        bad_peers: &[(Peer, Status)],
    ) {
        for (peer, status) in bad_peers.iter().cloned() {
            data.insert(peer, status);
        }
    }

    fn activate_peers_in_locked_data(
        &self,
        data: &mut HashMap<Peer, Status>,
        limit: usize,
        status_priority_order: &[Status],
    ) -> Vec<Peer> {
        let mut newly_activated_peers = Vec::with_capacity(std::cmp::min(data.len(), limit));

        for cur_status in status_priority_order {
            let iter = data
                .iter_mut()
                .filter_map(|(peer, status)| {
                    if discriminant(status) == discriminant(cur_status) {
                        *status = Status::Active;
                        Some(peer.to_owned())
                    } else {
                        None
                    }
                })
                .take(limit - newly_activated_peers.len());

            newly_activated_peers.extend(iter);
        }

        newly_activated_peers
    }
}

impl<P> Extend<P> for Peers
where
    P: Into<Peer>,
{
    fn extend<T: IntoIterator<Item = P>>(&mut self, iter: T) {
        let mut data = self.data.lock().unwrap();

        data.extend(iter.into_iter().map(|peer| (peer.into(), Status::Unknown)));
    }
}

#[cfg(test)]
mod tests {
    use super::super::peer::Peer;
    use super::{Peers, Status};
    use crate::data_structures::ID;

    // TODO I don't like this test, but most of the logic is dependent on previous lines.
    // So if I want to split into 3 tests, I have to just write 1/3 same, 2/3 same and same.
    // Splitting up into multiple non-test functions might help, I guess, but not really
    #[test]
    fn peers() {
        let info_hash = ID::new(rand::random());
        let mut peers = Peers::new(&info_hash);

        assert!(peers.serve(&info_hash));
        assert!(!peers.serve(&ID::new(rand::random())));

        let new_peers: [Peer; 3] = [
            Peer::new("127.0.0.1:52".parse().unwrap()),
            Peer::new("127.0.0.1:53".parse().unwrap()),
            Peer::new("127.0.0.1:54".parse().unwrap()),
        ];

        // data = [(52, unknown),(53, unknown),(54, unknown)]
        peers.extend(new_peers.iter());

        let data = peers.data.lock().unwrap();
        assert_eq!(&Status::Unknown, data.get(&new_peers[0]).unwrap());
        assert_eq!(&Status::Unknown, data.get(&new_peers[1]).unwrap());
        assert_eq!(&Status::Unknown, data.get(&new_peers[2]).unwrap());
        drop(data);

        // data is two actives and one unknown
        let unknown_peers = peers.return_batch_of_bad_peers_and_get_new_batch(&Vec::new(), 2);
        let data = peers.data.lock().unwrap();

        // got two peers, in data they're marked as active
        assert_eq!(2, unknown_peers.len());
        assert_eq!(&Status::Active, data.get(&unknown_peers[0]).unwrap());
        assert_eq!(&Status::Active, data.get(&unknown_peers[1]).unwrap());

        let return_peers = vec![(unknown_peers[0], Status::UnableToConnect)];

        drop(data);
        let unknown_peers_2 = peers.return_batch_of_bad_peers_and_get_new_batch(&return_peers, 100);
        let data = peers.data.lock().unwrap();

        // returned 1 and asked for 100 but 1 left + 1 just_returned = 2
        assert_eq!(2, unknown_peers_2.len());
        assert_eq!(&Status::Active, data.get(&unknown_peers_2[0]).unwrap());
        assert_eq!(&Status::Active, data.get(&unknown_peers_2[1]).unwrap());
        drop(data);

        // returned had known status with lower priority, it's last/second in the list
        assert_ne!(return_peers[0].0, unknown_peers_2[0]);
        assert_eq!(return_peers[0].0, unknown_peers_2[1]);

        assert_eq!(3, peers.peer_addresses().len());
    }
}
