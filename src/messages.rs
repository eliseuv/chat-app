use std::{
    fmt::Display,
    net::{SocketAddr, TcpStream},
    sync::Arc,
};

use chrono::Utc;

#[derive(Debug)]
pub struct Message {
    pub(crate) author: Author,
    pub(crate) destination: Destination,
    pub(crate) timestamp: chrono::DateTime<Utc>,
    pub(crate) content: MessageContent,
}

impl Display for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let content = match &self.content {
            MessageContent::ConnectRequest(_) => "Connection Request",
            MessageContent::DisconnetRequest => "Disconnection Request",
            MessageContent::Bytes(_) => "Data",
        };
        write!(
            f,
            "[{content}] {author} -> {dest} at {dt}",
            author = self.author,
            dest = self.destination,
            dt = self
                .timestamp
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, false)
        )
    }
}

#[derive(Debug)]
pub(crate) enum MessageContent {
    ConnectRequest(Arc<TcpStream>),
    DisconnetRequest,
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Author {
    Server,
    Client(SocketAddr),
}

impl Display for Author {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Author::Server => write!(f, "Server"),
            Author::Client(addr) => write!(f, "Client {addr}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Destination {
    Server,
    AllClients,
    Client(SocketAddr),
}

impl Display for Destination {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Destination::Server => write!(f, "Server"),
            Destination::AllClients => write!(f, "All Clients"),
            Destination::Client(addr) => write!(f, "Client {addr}"),
        }
    }
}
