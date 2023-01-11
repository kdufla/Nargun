use crate::util::id::ID;

#[derive(Debug)]
pub enum AnnounceEvent {
    Started,
    Stopped,
    Completed,
}
#[derive(Debug)]
pub struct Announce {
    pub tracker_url: String,
    pub info_hash: ID,
    pub peer_id: ID,
    pub port: u16,
    pub uploaded: u64,
    pub downloaded: u64,
    pub left: u64,
    pub compact: bool,
    pub no_peer_id: bool,
    pub event: Option<AnnounceEvent>,
    pub tracker_id: Option<String>,
}

impl Announce {
    pub fn as_url(&self) -> String {
        let mut s = self.tracker_url.clone();

        s.push_str("?info_hash=");
        s.push_str(urlencoding::encode_binary(&self.info_hash.as_byte_ref()).as_ref());

        s.push_str("&peer_id=");
        s.push_str(urlencoding::encode_binary(&self.peer_id.as_byte_ref()).as_ref());

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

#[cfg(test)]
mod tests {
    use super::{Announce, AnnounceEvent};
    use crate::{constants::ID_LEN, util::id::ID};

    const TRACKER_URL: &str = "http://example.com/announce";
    const INFO_HASH: [u8; ID_LEN] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 0];
    const PEER_ID: [u8; ID_LEN] = [
        11, 22, 33, 44, 55, 66, 77, 88, 99, 0, 11, 22, 33, 44, 55, 66, 77, 88, 99, 0,
    ];
    const ENCODED_URL: &str = "http://example.com/announce?info_hash=%01%02%03%04%05%06%07%08%09%00%01%02%03%04%05%06%07%08%09%00&peer_id=%0B%16%21%2C7BMXc%00%0B%16%21%2C7BMXc%00&port=6887&uploaded=776241&downloaded=277518&left=78907&compact=1&event=started";

    #[test]
    fn announce_as_url() {
        let info_hash = ID::new(INFO_HASH);
        let peer_id = ID::new(PEER_ID);

        let announce = Announce {
            tracker_url: TRACKER_URL.to_string(),
            info_hash,
            peer_id,
            port: 6887,
            uploaded: 776241,
            downloaded: 277518,
            left: 78907,
            compact: true,
            no_peer_id: true,
            event: Some(AnnounceEvent::Started),
            tracker_id: None,
        };

        assert_eq!(announce.as_url(), ENCODED_URL);
    }
}
