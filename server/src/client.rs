use core::str;
use std::{
    fmt::Display,
    io::Read,
    net::{SocketAddr, TcpStream},
    sync::{mpsc::Sender, Arc},
};

use anyhow::{Context, Result};
use chrono::{DateTime, TimeDelta, Utc};
use log::debug;

use crate::server_specs::{BanReason, ClientRequest, LocalMessage};

const MESSAGE_COOLDOWN_TIME: TimeDelta = TimeDelta::milliseconds(300);
const MAX_STRIKE_COUNT: u32 = 5;

fn parse_text(bytes: &[u8]) -> Result<String> {
    // Filter out escape codes
    let bytes_safe: Vec<u8> = bytes.iter().copied().filter(|c| *c >= 32).collect();
    // Read UTF-8
    let text = str::from_utf8(&bytes_safe).context("Data is not valid UTF-8")?;
    Ok(text.to_owned())
}

#[derive(Debug, Clone)]
pub struct Client {
    addr: SocketAddr,
    stream: Arc<TcpStream>,
    sender: Sender<LocalMessage>,
    last_message_time: DateTime<Utc>,
    strike_count: u32,
}

impl Display for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Client {addr}", addr = self.addr)
    }
}

impl Client {
    pub fn new(stream: TcpStream, sender: Sender<LocalMessage>) -> Result<Self> {
        let addr = stream
            .peer_addr()
            .context("Unable to identify client address")?;

        Ok(Self {
            addr,
            stream: Arc::new(stream),
            sender,
            last_message_time: Utc::now(),
            strike_count: 0,
        })
    }

    /// Get client address
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Limit rate of messages sent from client
    fn rate_limiter(&mut self) -> Result<()> {
        let message_time = Utc::now();
        if message_time.signed_duration_since(self.last_message_time) < MESSAGE_COOLDOWN_TIME {
            self.strike_count += 1;
            log::info!(
                "{self}: Strike {n}/{total}",
                n = self.strike_count,
                total = MAX_STRIKE_COUNT
            );
            if self.strike_count >= MAX_STRIKE_COUNT {
                // Ban offending client
                self.send_request(ClientRequest::BanRequest(BanReason::Spamming))?;
                self.strike_count = 0;
            }
        } else {
            // Reset strikes
            self.strike_count = 0;
        }
        self.last_message_time = message_time;
        Ok(())
    }

    /// Attempts to read data from stream
    fn read_stream(&self, buffer: &mut [u8]) -> Result<usize> {
        self.stream
            .as_ref()
            .read(buffer)
            .context("Unable to read data from stream")
    }

    /// Send a request to the server
    pub(crate) fn send_request(&self, request: ClientRequest) -> Result<()> {
        let message = LocalMessage {
            addr: self.addr,
            timestamp: chrono::Utc::now(),
            request,
        };

        debug!("{self} sending {message}");
        self.sender
            .send(message)
            .context("{self} unable to send Request {content} to Server")
    }

    /// Send Connect Request to Server
    fn request_connect(&self) -> Result<()> {
        self.send_request(ClientRequest::ConnectRequest(self.stream.clone()))
            .context("{self} unable to send Connect Request to Server")
    }

    /// Send Disconnect Request to Server
    fn request_disconnect(&self) -> Result<()> {
        self.send_request(ClientRequest::DisconnetRequest)
            .context("{self} unable to send Disconnect Request to Server")
    }

    /// Attempts to handle incoming data
    fn handle_data(&mut self, bytes: &[u8]) -> Result<()> {
        let text = parse_text(bytes)?;
        self.send_request(ClientRequest::Broadcast(text))
    }

    /// Shutdown client
    pub fn shutdown(&self) -> Result<()> {
        log::debug!("Shutting down {self}");
        self.request_disconnect().and_then(|()| {
            self.stream
                .as_ref()
                .shutdown(std::net::Shutdown::Both)
                .context("{self} was unable to shutdown properly")
        })
    }

    /// Run client
    pub fn run(&mut self) -> Result<()> {
        log::info!("Spawned thread for {self}");

        // Send Connect Request to Server
        self.request_connect()?;

        // Chat loop
        let mut buffer = [0; 64];
        loop {
            buffer.fill(0);
            let n = self.read_stream(&mut buffer)?;
            if n > 0 {
                log::debug!("{self} read {n} bytes into buffer");
                // Message rate limit
                self.rate_limiter()?;
                // Handle data read from stream
                if let Err(err) = self.handle_data(&buffer) {
                    log::error!("{self} could not handle data: {err}");
                }
            } else {
                log::debug!("{self} reached EOF");
                return self.request_disconnect();
            }
        }
    }
}
