use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct RemoteMessage {
    author: Author,
    content: Content,
    timestamp: i64,
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
