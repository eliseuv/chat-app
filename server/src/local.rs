use std::{
    fmt::Display,
    net::{SocketAddr, TcpStream},
    sync::Arc,
};

/// Local Messages
/// Messages sent locally from client thread to server
#[derive(Debug)]
pub struct LocalMessage {
    /// Address of the client sending the message
    pub(crate) addr: SocketAddr,
    /// Content of the message
    pub(crate) request: ClientRequest,
}

impl Display for LocalMessage {
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
pub(crate) enum ClientRequest {
    ConnectRequest(Arc<TcpStream>),
    DisconnetRequest,
    BanRequest(BanReason),
    Broadcast(String),
}

impl Display for ClientRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                ClientRequest::ConnectRequest(_) => "Connect Request".to_owned(),
                ClientRequest::DisconnetRequest => "Disconnect Request".to_owned(),
                ClientRequest::BanRequest(reason) => {
                    "Ban Me for ".to_owned()
                        + match reason {
                            BanReason::Spamming => "Spamming",
                            BanReason::_Other(reason) => reason,
                        }
                }
                ClientRequest::Broadcast(text) => {
                    format!("Broadcast: {text}")
                }
            }
        )
    }
}

/// Reason for client to be banned from the server
#[derive(Debug)]
pub(crate) enum BanReason {
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
