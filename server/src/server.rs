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
    local_messages::{Destination, LocalMessage, MessageContent},
    remote_messages::RemoteMessage,
    utils::insert_or_get_mut,
};

// TODO: Fix vulnerability to `slow loris reader`
// TODO: Move more load to the client thread

// Server constants
const TOTAL_BAN_TIME: TimeDelta = TimeDelta::seconds(5 * 60);
pub const TOKEN_LENGTH: usize = 8;

// Access token
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Token(pub(crate) [u8; TOKEN_LENGTH]);

impl Token {
    // New empty buffer to store token
    pub(crate) const fn new_buffer() -> [u8; TOKEN_LENGTH] {
        [0; TOKEN_LENGTH]
    }

    // Generate new random access token
    fn generate() -> Result<Token> {
        let mut buffer = Token::new_buffer();
        if let Err(err) = getrandom(&mut buffer) {
            Err(anyhow!("Unable to generate random token: {err}"))
        } else {
            Ok(Token(buffer))
        }
    }

    pub(crate) fn from_str(s: &str) -> Result<Self> {
        log::debug!("Token string: {s}");
        let str_len = s.len();
        if str_len != (2 * TOKEN_LENGTH) {
            Err(anyhow!("Invalid token string length: {str_len}"))
        } else {
            let mut buffer = Token::new_buffer();
            for (b, k) in buffer.iter_mut().zip((0..s.len()).step_by(2)) {
                *b = u8::from_str_radix(&s[k..k + 2], 16)?;
            }
            Ok(Token(buffer))
        }
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
    // Create new empty server
    pub fn new(receiver: Receiver<LocalMessage>) -> Result<Self> {
        log::debug!("Creating new Server");

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
            log::warn!("Client {addr} is already connected");
            return Ok(());
        }

        // Perform connection to Server
        if let Some(client) = self.clients.get_mut(&addr) {
            // Present token challenge
            if client.auth_timestamp.is_none() {
                stream
                    .as_ref()
                    .write_all("Token: ".as_bytes())
                    .and_then(|()| stream.as_ref().flush())
                    .context("Unable to send token challenge")?;
            }
            self.wait_list.insert(addr, stream);
        }

        Ok(())
    }

    fn disconnect_client(&mut self, addr: SocketAddr) -> Result<()> {
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

    // Broadcast message to clients
    fn broadcast_message(&self, message: LocalMessage) -> Result<()> {
        let author_addr = message.author_addr;
        let author_id = self
            .clients
            .get(&author_addr)
            .ok_or(anyhow!("Client {author_addr} id not found"))?
            .id;
        match message.content {
            MessageContent::Bytes(bytes) => {
                for (peer_addr, peer_stream) in self.conns.iter() {
                    if *peer_addr != message.author_addr {
                        log::debug!("Sending message from {author_addr} to Client {peer_addr}");
                        if let Err(err) = peer_stream
                            .as_ref()
                            .write_all(format!("user {author_id}: ").as_bytes())
                            .and_then(|()| peer_stream.as_ref().write_all(&bytes))
                            .and_then(|()| peer_stream.as_ref().write_all(b"\n"))
                            .and_then(|()| peer_stream.as_ref().flush())
                        {
                            log::error!("Unable to broadcast message from {author_addr} to {peer_addr}: {err}");
                        }
                    }
                }
                Ok(())
            }
            _ => Err(anyhow!("Invalid message type for broadcasting")),
        }
    }

    fn remote_message(&self, local_message: LocalMessage) -> Result<RemoteMessage> {
        todo!()
    }

    // Filter messages from banned IPs. Returns is banned boolean.
    fn ban_filter(&mut self, message: &LocalMessage) -> bool {
        let author_addr = message.author_addr;
        let author_ip = author_addr.ip();
        log::debug!("Checking IP {author_ip} ban status");
        if let Some(banned_at) = self.ban_list.get(&author_ip) {
            // Calculate ban time remaining
            let remaining_secs = (*banned_at + TOTAL_BAN_TIME)
                .signed_duration_since(Utc::now())
                .num_seconds();
            if remaining_secs > 0 {
                log::info!(
                    "IP {author_ip} is currently banned. Remaining time: {remaining_secs} seconds"
                );
                // Disconnect banned client if currently connected
                if let Some(stream) = self.conns.remove(&author_addr) {
                    let _ = stream
                        .as_ref()
                        .write_all(
                            format!(
                            "You are currently banned\nRemaining time: {remaining_secs} seconds\n"
                        )
                            .as_bytes(),
                        )
                        .and_then(|()| stream.as_ref().flush());
                    let _ = stream.as_ref().shutdown(net::Shutdown::Both);
                } else {
                    // Refuse Connect Request
                    if let MessageContent::ConnectRequest(stream) = &message.content {
                        let _ = (*stream)
                            .as_ref()
                            .write_all(
                                format!(
                            "You are currently banned\nRemaining time: {remaining_secs} seconds\n"
                        )
                                .as_bytes(),
                            )
                            .and_then(|()| stream.as_ref().flush());
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
                let _ = stream
                    .as_ref()
                    .write_all(text.as_bytes())
                    .and_then(|()| stream.as_ref().flush());
            }
            let _ = stream.as_ref().shutdown(net::Shutdown::Both);
        }
    }

    // Ban a given client
    fn ban_client(&mut self, addr: SocketAddr, reason: &str) {
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
                "You have been banned\nReason: {reason}\nBan time: {ban_time} seconds\n",
                ban_time = TOTAL_BAN_TIME.num_seconds()
            )),
        );
    }

    fn authenticate_client(&mut self, addr: SocketAddr, bytes: &[u8]) -> Result<()> {
        let bytes_len = bytes.len();
        if bytes.len() != 2 * TOKEN_LENGTH {
            bail!("Invalid token length: {bytes_len} bytes");
        }
        let token_str = str::from_utf8(bytes)?;
        log::debug!("Attempting validation from Client {addr} with token: {token_str}");
        if Token::from_str(token_str)? == self.access_token {
            log::info!("Client {addr} successfully authenticated");
            let client = self
                .clients
                .get_mut(&addr)
                .ok_or(anyhow!("Unable to find Client {addr} information"))?;
            client.auth_timestamp = Some(Utc::now());
            self.wait_list
                .remove(&addr)
                .ok_or(anyhow!("Client {addr} not found in wait list"))
                .and_then(|stream| {
                    stream
                        .as_ref()
                        .write_all(
                            format!(
                                "# Welcome to the chat server #\nYou are user {id}\n",
                                id = client.id
                            )
                            .as_bytes(),
                        )
                        .context("Unable to send welcome message to Client {addr}")?;
                    stream.as_ref().flush()?;
                    let _ = self.conns.insert(addr, stream);
                    Ok(())
                })?;
        } else {
            bail!("Invalid token")
        }
        Ok(())
    }

    // Run server
    pub fn run(mut self) -> Result<()> {
        log::debug!("Launching chat server");

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

            let client_addr = message.author_addr;

            // Get reference to client info
            let client = {
                let clients_count = self.clients.len();
                insert_or_get_mut(
                    &mut self.clients,
                    client_addr,
                    ClientInfo::new(clients_count),
                )
            };

            // Handle message
            match message.content {
                MessageContent::ConnectRequest(stream) => {
                    if let Err(err) = self.connect_client(client_addr, stream.clone()) {
                        log::error!("Unable to connect Client {client_addr}: {err}");
                        let _ = stream.shutdown(net::Shutdown::Both);
                    }
                }

                MessageContent::DisconnetRequest => {
                    if let Err(err) = self.disconnect_client(client_addr) {
                        log::error!("Unable to disconnect Client {client_addr}: {err}");
                    }
                }

                MessageContent::BanMe => {
                    self.ban_client(client_addr, "Spamming");
                }

                MessageContent::Bytes(bytes) => {
                    // Filter out escape codes
                    let bytes_safe: Vec<u8> = bytes.into_iter().filter(|c| *c >= 32).collect();

                    // Token challenge for unauthenticated clients
                    if client.auth_timestamp.is_none() {
                        if let Err(err) = self.authenticate_client(client_addr, &bytes_safe) {
                            log::error!("Unable to authenticate Client {client_addr}: {err}");
                            self.shutdown_client(client_addr, Some("Invalid token!\n"));
                        }
                        continue;
                    }

                    // Verify if message if valid UTF-8
                    let text = match str::from_utf8(&bytes_safe) {
                        Err(err) => {
                            log::error!("Text from message in not valid UTF-8: {err}");
                            continue;
                        }
                        Ok(string) => string,
                    };

                    log::info!(
                        "Client {addr} -> {dest}: {text_clean}",
                        addr = message.author_addr,
                        dest = message.destination,
                        text_clean = text.trim_end()
                    );

                    let message_safe = LocalMessage {
                        author_addr: message.author_addr,
                        destination: message.destination,
                        timestamp: message.timestamp,
                        content: MessageContent::Bytes(bytes_safe),
                    };

                    match message.destination {
                        Destination::Server => {
                            todo!("Handle messages sent to Server")
                        }
                        Destination::AllClients => {
                            // Broadcast message to other clients
                            if let Err(err) = self.broadcast_message(message_safe) {
                                log::error!("Unable to broadcast message: {err}");
                            }
                        }
                    }
                }
            }
        }
    }
}
