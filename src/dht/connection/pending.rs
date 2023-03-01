use crate::data_structures::{ID, ID_LEN};
use crate::dht::dht_manager::DhtCommand;
use crate::dht::krpc_message::{Arguments, Message, Response};
use bytes::Bytes;
use std::collections::HashMap;
use tokio::sync::mpsc;

#[derive(Clone, Debug)]
pub struct PendingRequests(HashMap<Bytes, (RequestType, Requester)>);

type Requester = mpsc::Sender<DhtCommand>;

#[derive(Clone, Debug, PartialEq)]
pub enum RequestType {
    Ping,
    FindNode(ID),
    GetPeers(ID),
    AnnouncePeer,
}

impl RequestType {
    fn eq_discriminant(&self, other: &Self) -> bool {
        std::mem::discriminant(self) != std::mem::discriminant(other)
    }
}

impl PendingRequests {
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    pub fn get(&self, tid: &Bytes, response_type: RequestType) -> Option<(RequestType, Requester)> {
        let (requested_type, _) = self.0.get(tid)?;

        if requested_type.eq_discriminant(&response_type) {
            self.0.remove(tid)
        } else {
            None
        }
    }

    pub fn insert(&mut self, tid: Bytes, req_type: RequestType, requester: Requester) {
        self.0.insert(tid, (req_type, requester));
    }
}

impl From<&Message> for RequestType {
    fn from(item: &Message) -> Self {
        let Message::Query{arguments, ..} = item else{
            panic!("this should be a query");
        };

        match arguments {
            Arguments::Ping { .. } => RequestType::Ping,
            Arguments::FindNode { target, .. } => RequestType::FindNode(target.to_owned()),
            Arguments::GetPeers { info_hash, .. } => RequestType::GetPeers(info_hash.to_owned()),
            Arguments::AnnouncePeer { .. } => RequestType::AnnouncePeer,
        }
    }
}

impl From<&Response> for RequestType {
    fn from(response: &Response) -> Self {
        match response {
            Response::Ping { .. } => RequestType::Ping,
            Response::FindNode { .. } => RequestType::FindNode(ID::new([0; ID_LEN])),
            Response::GetPeers { .. } => RequestType::GetPeers(ID::new([0; ID_LEN])),
            Response::AnnouncePeer { .. } => RequestType::AnnouncePeer,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RequestType;
    use crate::{
        data_structures::{ID, ID_LEN},
        dht::krpc_message::{rand_tid, Message},
    };
    use bytes::Bytes;

    #[test]
    fn from_message() {
        let dec_digits_twice =
            ID::new([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);

        let message =
            Message::get_peers_query(&ID::new([0; ID_LEN]), &dec_digits_twice, rand_tid());

        let request_type: RequestType = (&message).into();

        assert_eq!(request_type, RequestType::GetPeers(dec_digits_twice));
    }

    #[test]
    #[should_panic]
    fn from_message_panic() {
        let message =
            Message::announce_peer_resp(&ID::new([0; ID_LEN]), Bytes::from_static(b"bytes"));

        let _: RequestType = (&message).into();
    }

    #[test]
    fn from_response() {
        let response = match Message::announce_peer_resp(
            &ID::new([0; ID_LEN]),
            Bytes::from_static(b"bytes"),
        ) {
            Message::Response { response, .. } => response,
            _ => panic!("message should be a response"),
        };

        let request_type: RequestType = (&response).into();

        assert_eq!(request_type, RequestType::AnnouncePeer);
    }
}
