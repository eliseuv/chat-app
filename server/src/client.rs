use core::str;
use std::{
    fmt::Display,
    io::Read,
    net::{SocketAddr, TcpStream},
    sync::{mpsc::Sender, Arc},
};

use anyhow::{bail, Context, Result};
use chrono::{DateTime, TimeDelta, Utc};
use log::debug;

use crate::{
    client_requests::{BanReason, ClientRequest, Request},
    remote::{self, BUFFER_SIZE},
    server::Token,
};

// TODO: Receive message struct from remote client
// TODO: Let client know when server is offline

const MESSAGE_COOLDOWN_TIME: TimeDelta = TimeDelta::milliseconds(300);
const MAX_STRIKE_COUNT: u32 = 5;

/// Handle incoming data
fn parse_text(bytes: &[u8]) -> Result<String> {
    // Filter out escape codes
    let bytes_safe: Vec<u8> = bytes.iter().copied().filter(|c| *c >= 32).collect();
    // Read UTF-8
    let text = str::from_utf8(&bytes_safe).context("Data is not valid UTF-8")?;
    Ok(text.to_owned())
}

/// Client thread
#[derive(Debug, Clone)]
pub struct Client {
    /// Remote address
    addr: SocketAddr,
    /// Remote stream
    stream: Arc<TcpStream>,
    /// Channel to send request to server
    sender: Sender<ClientRequest>,
    /// Buffer to read incoming data into
    buffer: [u8; BUFFER_SIZE],
    /// Time of the last message sent by the client
    last_message_time: DateTime<Utc>,
    /// Number of strikes of the client to avoid spamming
    strike_count: u32,
}

impl Display for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Client {addr}", addr = self.addr)
    }
}

impl Client {
    /// Construct new Client
    pub fn new(stream: TcpStream, sender: Sender<ClientRequest>) -> Result<Self> {
        let addr = stream
            .peer_addr()
            .context("Unable to identify client address")?;

        Ok(Self {
            addr,
            stream: Arc::new(stream),
            sender,
            buffer: [0; BUFFER_SIZE],
            last_message_time: Utc::now(),
            strike_count: 0,
        })
    }

    /// Get client address
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Authenticate client using the server access token
    pub fn authenticate(&mut self, access_token: Token) -> Result<()> {
        remote::Message::new(remote::Author::Server, "Provide access token.".to_owned())
            .write_to(self.stream.as_ref())
            .context("Unable to send token challenge")?;
        let bytes = self.read_stream()?;
        let token_str = parse_text(bytes)?;
        if Token::from_str(&token_str)? == access_token {
            log::info!("{self} successfully authenticated");
            Ok(())
        } else {
            bail!("Invalid token")
        }
    }

    fn read_stream(&mut self) -> Result<&[u8]> {
        log::trace!("{self} attempting to read from stream");
        // ciborium::from_reader_with_buffer(self.stream.as_ref(), &mut self.buffer)
        //     .context("Unable to deserialize incomeing data")
        // TODO: Sanitize data
        let n = self
            .stream
            .as_ref()
            .read(&mut self.buffer)
            .context("Unable to read from stream")?;
        log::debug!("{self} read {n} bytes into buffer");
        Ok(&self.buffer[0..n])
    }

    /// Limit rate of messages sent from Client
    fn rate_limiter(&mut self) -> Result<bool> {
        let message_time = Utc::now();
        if message_time.signed_duration_since(self.last_message_time) < MESSAGE_COOLDOWN_TIME {
            // Client is spamming, add strike
            self.strike_count += 1;
            log::info!(
                "{self}: Strike {n}/{total}",
                n = self.strike_count,
                total = MAX_STRIKE_COUNT
            );
            if self.strike_count >= MAX_STRIKE_COUNT {
                // Ban offending client
                self.strike_count = 0;
                return Ok(true);
            }
        } else {
            // Reset strikes
            self.strike_count = 0;
        }
        self.last_message_time = message_time;
        Ok(false)
    }

    /// Send a Request to the Server
    fn send_request(&self, request: Request) -> Result<()> {
        let message = ClientRequest {
            addr: self.addr,
            request,
        };

        debug!("{self} sending {message}");
        self.sender
            .send(message)
            .context("{self} unable to send {message}")
    }

    /// Send Connect Request to Server
    fn request_connect(&self) -> Result<()> {
        log::trace!("{self} sending Connect Request");
        self.send_request(Request::Connect(self.stream.clone()))
            .context("{self} unable to send Connect Request to Server")
    }

    /// Send Disconnect Request to Server
    fn request_disconnect(&self) -> Result<()> {
        log::trace!("{self} sending Disconnect Request");
        self.send_request(Request::Disconnet)
            .context("{self} unable to send Disconnect Request to Server")
    }

    /// Send Disconnect Request to Server
    fn send_text(&self, text: String) -> Result<()> {
        log::trace!("{self} sending Text: {text}");
        self.send_request(Request::Broadcast(text))
            .context("{self} unable to send text message to Server")
    }

    /// Run client
    pub fn run(&mut self, access_token: Token) -> Result<()> {
        log::trace!("Spawned thread for {self}");

        // Authenticate client using server access token
        self.authenticate(access_token)?;

        // Send connection request to server
        self.request_connect()?;

        // Chat loop
        loop {
            // Message rate limit
            if self.rate_limiter()? {
                return self.send_request(Request::Ban(BanReason::Spamming));
            }

            // Read incoming data
            let bytes = self.read_stream()?;

            if bytes.is_empty() {
                log::debug!("{self} reached EOF");
                return self.request_disconnect();
            } else {
                // Handle data read from stream
                match parse_text(bytes) {
                    Err(err) => {
                        log::error!("{self} could not parse text: {err}");
                    }
                    Ok(text) => {
                        log::debug!("{self} says: {text}");
                        if let Err(err) = self.send_text(text) {
                            log::error!("{self} could not send text Message to server: {err}");
                        };
                    }
                }
            }
        }
    }

    /// Shutdown client
    pub fn shutdown(&self) -> Result<()> {
        log::debug!("Shutting down {self} stream");
        self.stream
            .as_ref()
            .shutdown(std::net::Shutdown::Both)
            .context("{self} was unable to shutdown stream")
    }
}
