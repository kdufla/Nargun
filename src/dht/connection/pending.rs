use crate::data_structures::{ID, ID_LEN};
use crate::dht::krpc_message::{Arguments, Message, Response};
use bytes::Bytes;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

#[derive(Clone, Debug)]
pub struct PendingRequests(Arc<Mutex<HashMap<Bytes, RequestType>>>);

#[derive(Clone, Debug, PartialEq)]
pub enum RequestType {
    Ping,
    FindNode(ID),
    GetPeers(ID),
    AnnouncePeer,
}

impl PendingRequests {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(HashMap::<Bytes, RequestType>::new())))
    }

    pub fn get(&self, k: &Bytes) -> Option<RequestType> {
        self.0.lock().unwrap().get(k).cloned()
    }

    pub fn insert(&mut self, k: Bytes, v: RequestType) -> Option<RequestType> {
        self.0.lock().unwrap().insert(k, v)
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
        dht::krpc_message::Message,
    };
    use bytes::Bytes;

    #[test]
    fn from_message() {
        let dec_digits_twice =
            ID::new([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);

        let message = Message::get_peers_query(&ID::new([0; ID_LEN]), &dec_digits_twice);

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
