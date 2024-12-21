use std::{
    fmt::Display,
    net::{SocketAddr, TcpStream},
    sync::Arc,
};

use chrono::{DateTime, Utc};

#[derive(Debug)]
pub struct LocalMessage {
    pub(crate) author_addr: SocketAddr,
    pub(crate) destination: Destination,
    pub(crate) timestamp: DateTime<Utc>,
    pub(crate) content: MessageContent,
}

impl Display for LocalMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{content}] {author} -> {dest} at {dt}",
            content = self.content,
            author = self.author_addr,
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
    BanMe,
    Bytes(Vec<u8>),
}

impl Display for MessageContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let content_fmt = match self {
            MessageContent::ConnectRequest(_) => "Connect Request",
            MessageContent::DisconnetRequest => "Disconnect Request",
            MessageContent::BanMe => "Ban Me",
            MessageContent::Bytes(_) => "Bytes",
        };
        write!(f, "{content_fmt}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Destination {
    Server,
    AllClients,
}

impl Display for Destination {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Destination::Server => write!(f, "Server"),
            Destination::AllClients => write!(f, "All Clients"),
        }
    }
}
