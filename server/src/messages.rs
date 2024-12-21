//! Remote Messages

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Message {
    author: Author,
    timestamp: i64,
    content: Content,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Content {
    ConnectRequest,
    DisconnetRequest,
    Text(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Author {
    Server,
    Client(String),
}
