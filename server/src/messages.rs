use std::io::{self, Read, Write};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::requests::BanReason;

/// Message to be sent to remote client
#[derive(Debug, Serialize, Deserialize)]
pub struct MessageToClient {
    pub timestamp: i64,
    pub author: MessageAuthor,
}

impl MessageToClient {
    /// New message to remote client
    pub fn new(content: MessageAuthor) -> Self {
        Self {
            timestamp: Utc::now().timestamp(),
            author: content,
        }
    }

    pub fn write_to(&self, writer: impl Write) -> Result<(), ciborium::ser::Error<io::Error>> {
        ciborium::into_writer(self, writer)
    }

    pub fn read_from(reader: impl Read) -> Result<Self, ciborium::de::Error<io::Error>> {
        ciborium::from_reader(reader)
    }
}

/// Content of the message to be received by remote client
#[derive(Debug, Serialize, Deserialize)]
pub enum MessageAuthor {
    Server(ServerMessage),
    Peer { id: usize, content: PeerMessage },
}

/// Messages the server
#[derive(Debug, Serialize, Deserialize)]
pub enum ServerMessage {
    Ban(BanReason),
    Text(String),
}

/// Messages from a remote peer
#[derive(Debug, Serialize, Deserialize)]
pub enum PeerMessage {
    Text(String),
}

pub struct ClientMessage {
    pub timestamp: i64,
    pub text: String,
}

impl ClientMessage {
    pub fn new(text: String) -> Self {
        Self {
            timestamp: Utc::now().timestamp(),
            text,
        }
    }
}
