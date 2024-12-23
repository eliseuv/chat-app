use core::str;
use std::{
    collections::HashMap,
    fmt::Display,
    io::Write,
    net::{self, IpAddr, SocketAddr, TcpStream},
    sync::{mpsc::Receiver, Arc},
};

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, TimeDelta, Utc};
use getrandom::getrandom;

use crate::{
    local::{BanReason, ClientRequest, LocalMessage},
    remote,
    utils::insert_or_get_mut,
};

// TODO: Fix vulnerability to `slow loris reader`
// TODO: Move more load to the client thread

// Server constants
const TOTAL_BAN_TIME: TimeDelta = TimeDelta::seconds(5 * 60);
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

#[derive(Debug)]
struct ClientInfo {
    id: usize,
    auth_timestamp: Option<DateTime<Utc>>,
}

impl ClientInfo {
    fn new(id: usize) -> Self {
        Self {
            id,
            auth_timestamp: None,
        }
    }
}

// TODO: Use something better than `wait_list`
#[derive(Debug)]
pub struct Server {
    receiver: Receiver<LocalMessage>,
    access_token: Token,
    ban_list: HashMap<IpAddr, DateTime<Utc>>,
    clients: HashMap<SocketAddr, ClientInfo>,
    conns: HashMap<SocketAddr, Arc<TcpStream>>,
    wait_list: HashMap<SocketAddr, Arc<TcpStream>>,
}

impl Server {
    /// Create new empty Server
    pub fn new(receiver: Receiver<LocalMessage>) -> Result<Self> {
        log::trace!("Creating new Server");

        // Generate access token
        let access_token = Token::generate()?;
        log::info!("Access token: {access_token}");

        Ok(Self {
            receiver,
            access_token,
            ban_list: HashMap::new(),
            clients: HashMap::new(),
            conns: HashMap::new(),
            wait_list: HashMap::new(),
        })
    }

    fn connect_client(&mut self, addr: SocketAddr, stream: Arc<TcpStream>) -> Result<()> {
        let stream_addr = stream.as_ref().peer_addr()?;
        log::debug!("Connecting Client {stream_addr}");

        // Check if author is the same as client connecting
        if stream_addr != addr {
            bail!("Client {addr} requesting connection for different Client {stream_addr}",);
        }

        // Check if client is already connected
        if self.conns.contains_key(&addr) {
            bail!("Client {addr} is already connected");
        }

        // Perform connection to Server
        if let Some(client) = self.clients.get_mut(&addr) {
            // Present token challenge
            if client.auth_timestamp.is_none() {
                ciborium::into_writer(
                    &remote::Message::new(
                        remote::Author::Server,
                        "Provide the access token please.".to_owned(),
                    ),
                    stream.as_ref(),
                )
                .context("Unable to send token challenge")?;
            }
            self.wait_list.insert(addr, stream);
        }

        Ok(())
    }

    fn disconnect_client(&mut self, addr: SocketAddr) -> Result<()> {
        log::info!("Disconneting Client {addr}");
        match self.conns.remove(&addr) {
            None => bail!("Attempting to disconnect already disconnected Client {addr}"),
            Some(stream) => {
                stream
                    .as_ref()
                    .shutdown(net::Shutdown::Both)
                    .context("Unable to shutdown stream while disconnecting Client {addr}")?;
                Ok(())
            }
        }
    }

    fn get_client_id(&self, addr: SocketAddr) -> Result<usize> {
        let id = self
            .clients
            .get(&addr)
            .ok_or(anyhow!("Client {addr} id not found"))?
            .id;
        Ok(id)
    }

    fn broadcast(&self, author_addr: SocketAddr, text: &str) -> Result<()> {
        log::trace!("Broadcasting message from client {author_addr}");
        let id = self.get_client_id(author_addr)?;
        let author = remote::Author::Client(id);
        let message = remote::Message::new(author, text.to_owned());
        log::debug!("Sending {message:?}");
        for (peer_addr, peer_stream) in self.conns.iter() {
            if *peer_addr != author_addr {
                log::debug!("Sending message from Client {author_addr} to Client {peer_addr}");
                if let Err(err) = ciborium::into_writer(&message, peer_stream.as_ref())
                    .context("Unable to serialize message")
                    .and_then(|()| {
                        peer_stream
                            .as_ref()
                            .flush()
                            .context("Unable to flush stream")
                    })
                {
                    log::error!(
                        "Unable to broadcast message from Client {author_addr} to Client {peer_addr}: {err}"
                    );
                }
            }
        }
        Ok(())
    }

    /// Filter messages from banned IPs. Returns is banned boolean.
    fn ban_filter(&mut self, message: &LocalMessage) -> bool {
        let author_addr = message.addr;
        let author_ip = author_addr.ip();
        log::trace!("Checking IP {author_ip} ban status");
        if let Some(banned_at) = self.ban_list.get(&author_ip) {
            // Calculate ban time remaining
            let remaining_secs = (*banned_at + TOTAL_BAN_TIME)
                .signed_duration_since(Utc::now())
                .num_seconds();
            if remaining_secs > 0 {
                log::debug!(
                    "IP {author_ip} is currently banned. Remaining time: {remaining_secs} seconds"
                );
                // Disconnect banned client if currently connected
                if let Some(stream) = self.conns.remove(&author_addr) {
                    let _ = ciborium::into_writer(
                        &remote::Message::new(
                            remote::Author::Server,
                            format!(
                            "You are currently banned\nRemaining time: {remaining_secs} seconds\n"
                        ),
                        ),
                        stream.as_ref(),
                    );
                    let _ = stream.as_ref().shutdown(net::Shutdown::Both);
                } else {
                    // Refuse Connect Request
                    if let ClientRequest::ConnectRequest(stream) = &message.request {
                        let _ = ciborium::into_writer(
                            &remote::Message::new(
                                remote::Author::Server,
                                format!(
                            "You are currently banned\nRemaining time: {remaining_secs} seconds\n"
                        ),
                            ),
                            stream.as_ref(),
                        );
                        let _ = (*stream).as_ref().shutdown(net::Shutdown::Both);
                    }
                }
                // Client is still banned
                true
            } else {
                // Client no longer banned
                log::info!("Client {author_ip} has been unbanned");
                let _ = self.ban_list.remove(&author_ip);
                false
            }
        } else {
            // Client was not banned
            false
        }
    }

    // Shutdown client, optionally sending a final message
    fn shutdown_client(&mut self, addr: SocketAddr, text: Option<&str>) {
        log::info!("Shutting down Client {addr}");
        if let Some(stream) = self
            .wait_list
            .remove(&addr)
            .or_else(|| self.conns.remove(&addr))
        {
            if let Some(text) = text {
                let _ = ciborium::into_writer(
                    &remote::Message::new(remote::Author::Server, text.to_owned()),
                    stream.as_ref(),
                );
            }
            let _ = stream.as_ref().shutdown(net::Shutdown::Both);
        }
    }

    // Ban a given client
    fn ban_client(&mut self, addr: SocketAddr, reason: BanReason) {
        let ip = addr.ip();
        log::info!(
            "Banning IP {ip}. Reason: {reason}. Ban time: {ban_time} seconds",
            ban_time = TOTAL_BAN_TIME.num_seconds()
        );
        self.ban_list.insert(ip, Utc::now());
        // Disconnect client
        self.shutdown_client(
            addr,
            Some(&format!(
                "You have been banned. Reason: {reason}. Ban time: {ban_time} seconds\n",
                ban_time = TOTAL_BAN_TIME.num_seconds()
            )),
        );
    }

    /// Attempts to authenticate a client with the token it provided
    fn authenticate_client(&mut self, addr: SocketAddr, token_str: &str) -> Result<()> {
        log::debug!("Attempting validation from Client {addr} with token: {token_str}");
        if Token::from_str(token_str)? == self.access_token {
            log::info!("Client {addr} successfully authenticated");
            let client = self
                .clients
                .get_mut(&addr)
                .ok_or(anyhow!("Unable to find Client {addr} information"))?;
            client.auth_timestamp = Some(Utc::now());
            let stream = self
                .wait_list
                .remove(&addr)
                .ok_or(anyhow!("Client {addr} not found in wait list"))?;
            ciborium::into_writer(
                &remote::Message::new(
                    remote::Author::Server,
                    format!(
                        "Welcome to the chat server! You are user {id}.\n",
                        id = client.id
                    ),
                ),
                stream.as_ref(),
            )
            .context("Unable to send welcome message to Client {addr}")?;
            stream.as_ref().flush()?;
            let _ = self.conns.insert(addr, stream);
            Ok(())
        } else {
            bail!("Invalid token")
        }
    }

    /// Run server
    pub fn run(mut self) -> Result<()> {
        log::trace!("Launching chat server");

        loop {
            // Try to receive a message
            let message = match self.receiver.recv() {
                Err(err) => {
                    log::error!("Server could not receive message: {err}");
                    continue;
                }
                Ok(message) => message,
            };
            log::debug!("Server received message: {message}");

            // Ban filter
            if self.ban_filter(&message) {
                continue;
            }

            let client_addr = message.addr;

            // Get reference to client info
            let client = {
                let clients_count = self.clients.len();
                insert_or_get_mut(
                    &mut self.clients,
                    client_addr,
                    ClientInfo::new(clients_count + 1),
                )
            };

            // Handle message
            match message.request {
                ClientRequest::ConnectRequest(stream) => {
                    if let Err(err) = self.connect_client(client_addr, stream.clone()) {
                        log::error!("Unable to connect Client {client_addr}: {err}");
                        let _ = stream.shutdown(net::Shutdown::Both);
                    }
                }

                ClientRequest::DisconnetRequest => {
                    if let Err(err) = self.disconnect_client(client_addr) {
                        log::error!("Unable to disconnect Client {client_addr}: {err}");
                    }
                }

                ClientRequest::BanRequest(reason) => {
                    self.ban_client(client_addr, reason);
                }

                ClientRequest::Broadcast(text) => {
                    // Token challenge for unauthenticated clients
                    if client.auth_timestamp.is_none() {
                        if let Err(err) = self.authenticate_client(client_addr, &text) {
                            log::error!("Unable to authenticate Client {client_addr}: {err}");
                            self.shutdown_client(client_addr, Some("Invalid token!\n"));
                        }
                        continue;
                    }

                    log::info!("Client {client_addr} says: {text}");
                    if let Err(err) = self.broadcast(client_addr, &text) {
                        log::error!("Unable to broadcast message: {err}");
                    }
                }
            }
        }
    }
}
