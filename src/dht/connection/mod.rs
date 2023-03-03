mod channel_message;
mod connection_handle;
mod connection_manager;
mod error;
mod pending;

pub use channel_message::{FromCon, FromConQuery, FromConResp};
pub use connection_handle::{Connection, QueryingConnection, DHT_MTU};
pub use connection_manager::QueryId;
pub use error::Error as ConError;
