use super::channel_message::FromConResp;
use crate::{
    data_structures::SerializableBuf,
    dht::krpc_message::{Arguments, Message, Response},
};
use std::collections::HashMap;
use tokio::sync::mpsc;

#[derive(Clone, Debug)]
pub struct PendingRequests(HashMap<SerializableBuf, (RequestType, Requester)>);

type Requester = mpsc::Sender<FromConResp>;

#[derive(Clone, Debug, PartialEq)]
pub enum RequestType {
    Ping,
    FindNode,
    GetPeers,
    AnnouncePeer,
}

impl PendingRequests {
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    pub fn get(&mut self, tid: &SerializableBuf, response_type: RequestType) -> Option<Requester> {
        let (requested_type, _) = self.0.get(tid)?;

        if *requested_type == response_type {
            self.0.remove(tid).map(|(_, requester)| requester)
        } else {
            None
        }
    }

    pub fn insert(&mut self, tid: SerializableBuf, req_type: RequestType, requester: Requester) {
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
            Arguments::FindNode { .. } => RequestType::FindNode,
            Arguments::GetPeers { .. } => RequestType::GetPeers,
            Arguments::AnnouncePeer { .. } => RequestType::AnnouncePeer,
        }
    }
}

impl From<&Response> for RequestType {
    fn from(response: &Response) -> Self {
        match response {
            Response::Ping { .. } => RequestType::Ping,
            Response::FindNode { .. } => RequestType::FindNode,
            Response::GetPeers { .. } => RequestType::GetPeers,
            Response::AnnouncePeer { .. } => RequestType::AnnouncePeer,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RequestType;
    use crate::{
        data_structures::{SerializableBuf, ID, ID_LEN},
        dht::krpc_message::{rand_tid, Message},
    };

    #[test]
    fn from_message() {
        let dec_digits_twice =
            ID::new([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);

        let message =
            Message::get_peers_query(&ID::new([0; ID_LEN]), &dec_digits_twice, rand_tid());

        let request_type: RequestType = (&message).into();

        assert_eq!(request_type, RequestType::GetPeers);
    }

    #[test]
    #[should_panic]
    fn from_message_panic() {
        let message = Message::announce_peer_resp(
            &ID::new([0; ID_LEN]),
            SerializableBuf::from(b"bytes".as_ref()),
        );

        let _: RequestType = (&message).into();
    }

    #[test]
    fn from_response() {
        let response = match Message::announce_peer_resp(
            &ID::new([0; ID_LEN]),
            SerializableBuf::from(b"bytes".as_ref()),
        ) {
            Message::Response { response, .. } => response,
            _ => panic!("message should be a response"),
        };

        let request_type: RequestType = (&response).into();

        assert_eq!(request_type, RequestType::AnnouncePeer);
    }
}
