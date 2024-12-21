//! Server Specification

use std::{
    fmt::Display,
    net::{SocketAddr, TcpStream},
    sync::Arc,
};

use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, Utc};
use getrandom::getrandom;

/// Server access token length in bytes
pub(crate) const TOKEN_LENGTH: usize = 8;

/// Server Access Token
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Token(pub(crate) [u8; TOKEN_LENGTH]);

impl Token {
    /// New empty buffer to store access token
    pub(crate) const fn new_buffer() -> [u8; TOKEN_LENGTH] {
        [0; TOKEN_LENGTH]
    }

    /// Generate new random access token
    pub(crate) fn generate() -> Result<Token> {
        let mut buffer = Token::new_buffer();
        getrandom(&mut buffer).map_err(|err| anyhow!("Unable to generate random token: {err}"))?;
        Ok(Token(buffer))
    }

    /// Attempts to parse access token from hex representation string
    pub(crate) fn from_str(s: &str) -> Result<Self> {
        log::debug!("Token string: {s}");

        if !s.is_ascii() {
            bail!("Token string must be ASCII")
        }
        let str_len = s.len();
        if str_len != (2 * TOKEN_LENGTH) {
            bail!("Invalid token string length: {str_len}")
        }

        // let buffer: Vec<u8> = (0..str_len)
        //     .step_by(2)
        //     .map(|k| {
        //         u8::from_str_radix(&s[k..k + 2], 16)
        //             .context("Unable to convert string to hex value")
        //     })
        //     .collect()?;
        // Ok(Token(buffer.try_into::()))

        let mut buffer = Token::new_buffer();
        for (b, k) in buffer.iter_mut().zip((0..s.len()).step_by(2)) {
            *b = u8::from_str_radix(&s[k..k + 2], 16)?;
        }
        Ok(Token(buffer))
    }
}

impl Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for b in self.0.iter() {
            write!(f, "{b:02X}")?;
        }
        Ok(())
    }
}

/// Local Messages
/// Messages sent locally from client thread to server
#[derive(Debug)]
pub struct LocalMessage {
    /// Address of the client sending the message
    pub(crate) addr: SocketAddr,
    /// Timestamp of the message creation
    pub(crate) timestamp: DateTime<Utc>,
    /// Content of the message
    pub(crate) request: ClientRequest,
}

impl Display for LocalMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{author} at {dt}: {content}",
            author = self.addr,
            dt = self
                .timestamp
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, false),
            content = self.request,
        )
    }
}

/// Content of a message sent locally from client thread to server
#[derive(Debug)]
pub(crate) enum ClientRequest {
    /// Client is requesting the server to be connected using the stream provided
    ConnectRequest(Arc<TcpStream>),
    /// Client is requesting the server to be disconnected
    DisconnetRequest,
    /// Client is requesting the server to be banned for the reason given
    BanRequest(BanReason),
    /// Client is requesting the server to broadcast a text message to all other clients
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
