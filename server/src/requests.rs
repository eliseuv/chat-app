use std::{
    fmt::Display,
    net::{SocketAddr, TcpStream},
    sync::Arc,
};

use serde::{Deserialize, Serialize};

/// Messages sent locally from client thread to server
#[derive(Debug)]
pub struct ClientRequest {
    /// Address of the client sending the request
    pub(crate) addr: SocketAddr,
    /// The request itself
    pub(crate) request: Request,
}

impl Display for ClientRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{author}: {content}",
            author = self.addr,
            content = self.request,
        )
    }
}

/// Request from client thread to server
#[derive(Debug)]
pub(crate) enum Request {
    Connect(Arc<TcpStream>),
    Disconnet,
    Ban(BanReason),
    Broadcast(String),
}

impl Display for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Request::Connect(_) => "Connect Request".to_owned(),
                Request::Disconnet => "Disconnect Request".to_owned(),
                Request::Ban(reason) => {
                    "Ban Me for ".to_owned()
                        + match reason {
                            BanReason::Spamming => "Spamming",
                            BanReason::_Other(reason) => reason,
                        }
                }
                Request::Broadcast(text) => {
                    format!("Broadcast: {text}")
                }
            }
        )
    }
}

/// Reason for client to be banned from the server
#[derive(Debug, Serialize, Deserialize)]
pub enum BanReason {
    Spamming,
    _Other(String),
}

impl Display for BanReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                BanReason::Spamming => "Spamming",
                BanReason::_Other(reason) => reason,
            }
        )
    }
}
