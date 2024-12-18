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
    messages::{Destination, Message, MessageContent},
    utils::insert_or_get_mut,
};

// TODO: Fix vulnerability to `slow loris reader`
// TODO: Move more load to the client thread

// Server constants
const TOTAL_BAN_TIME: TimeDelta = TimeDelta::seconds(5 * 60);
const MESSAGE_COOLDOWN_TIME: TimeDelta = TimeDelta::milliseconds(300);
const MAX_STRIKE_COUNT: u32 = 5;
const WELCOME_MESSAGE: &str = "# Welcome to the epic Чат server #\n";
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
        let slen = s.len();
        if slen % (2 * TOKEN_LENGTH) != 0 {
            Err(anyhow!("Invalid token string length: {slen}"))
        } else {
            let mut buffer = Token::new_buffer();
            for (b, k) in buffer.iter_mut().zip((0..s.len()).step_by(2)) {
                *b = u8::from_str_radix(&s[k..k + 2], 16)?;
            }
            Ok(Token(buffer))
        }
    }

    fn validate_bytes(&self, bytes: &[u8]) -> Result<DateTime<Utc>> {
        if bytes.len() != 2 * TOKEN_LENGTH {
            bail!("Invalid token length");
        }
        if &Token::from_str(str::from_utf8(bytes)?)? == self {
            Ok(Utc::now())
        } else {
            bail!("Invalid token")
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
struct Client {
    stream: Option<Arc<TcpStream>>,
    auth_timestamp: Option<DateTime<Utc>>,
    last_message_timestamp: DateTime<Utc>,
    strike_count: u32,
}

impl Client {}

#[derive(Debug)]
pub struct Server {
    receiver: Receiver<Message>,
    access_token: Token,
    clients: HashMap<SocketAddr, Client>,
    ban_list: HashMap<IpAddr, DateTime<Utc>>,
}

impl Server {
    // Create new empty server
    pub fn new(receiver: Receiver<Message>) -> Result<Self> {
        log::debug!("Creating new Server");

        // Generate access token
        let access_token = Token::generate()?;
        log::info!("Access token: {access_token}");

        Ok(Self {
            receiver,
            access_token,
            clients: HashMap::new(),
            ban_list: HashMap::new(),
        })
    }

    fn connect_client(&mut self, addr: SocketAddr, stream: Arc<TcpStream>) -> Result<()> {
        let stream_addr = stream.as_ref().peer_addr()?;
        log::debug!("Connecting Client {stream_addr}");

        // Check if author is the same as client connecting
        if stream_addr != addr {
            bail!("Client {addr} requesting connection for different Client {stream_addr}",);
        }

        // Send welcome message
        stream
            .as_ref()
            .write_all(WELCOME_MESSAGE.as_bytes())
            .context("Unable to send welcome message")?;

        // Perform connection to Server
        if let Some(client) = self.clients.get_mut(&addr) {
            // Present token challenge
            if client.auth_timestamp.is_none() {
                let _ = stream.as_ref().write_all("Token: ".as_bytes());
            }
            // Update state
            *client = Client {
                stream: Some(stream),
                auth_timestamp: client.auth_timestamp,
                last_message_timestamp: Utc::now(),
                strike_count: client.strike_count,
            };
        }

        Ok(())
    }

    fn disconnect_client(&mut self, addr: SocketAddr) -> Result<()> {
        match self.clients.remove(&addr) {
            None => bail!("Attempting to disconnect Client unknown to Server"),
            Some(client) => match client.stream {
                None => bail!("Attempting to disconnect already disconnected client"),
                Some(stream) => {
                    stream
                        .as_ref()
                        .shutdown(net::Shutdown::Both)
                        .context("Unable to shutdown stream while disconnecting Client")?;
                    Ok(())
                }
            },
        }
    }

    // Broadcast message to clients
    fn broadcast_message(&self, message: Message) -> Result<()> {
        let author_addr = message.author_addr;
        match message.content {
            MessageContent::Bytes(bytes) => {
                for (peer_addr, peer_client) in self.clients.iter() {
                    if *peer_addr != message.author_addr && peer_client.auth_timestamp.is_some() {
                        if let Some(stream) = &peer_client.stream {
                            log::debug!("Sending message from {author_addr} to Client {peer_addr}");
                            let nbytes = stream.as_ref().write(&bytes)?;
                            match nbytes.cmp(&bytes.len()) {
                                std::cmp::Ordering::Less => log::warn!(
                                    "Message partially sent: {nbytes}/{total} bytes sent",
                                    total = bytes.len()
                                ),
                                std::cmp::Ordering::Equal => {
                                    log::debug!("Successfully sent entire message")
                                }
                                std::cmp::Ordering::Greater => log::error!(
                                "More bytes sent than in the original message!?: {nbytes}/{total}",
                                total = bytes.len()
                            ),
                            }
                        }
                    }
                }
                Ok(())
            }
            _ => Err(anyhow!("Invalid message type for bradcasting")),
        }
    }

    // Filter messages from banned IPs. Returns is banned boolean.
    fn ban_filter(&mut self, message: &Message) -> bool {
        let addr = message.author_addr;
        log::info!("Checking Client {addr} ban status");
        if let Some(banned_at) = self.ban_list.get(&addr.ip()) {
            // Calculate ban time remaining
            let remaining_secs = (*banned_at + TOTAL_BAN_TIME)
                .signed_duration_since(Utc::now())
                .num_seconds();
            if remaining_secs > 0 {
                log::info!(
                    "Client {addr} is currently banned. Remaining time: {remaining_secs} seconds"
                );
                // Disconnect banned client if currently connected
                if let Some(client) = self.clients.remove(&addr) {
                    if let Some(stream) = client.stream {
                        let _ = stream.as_ref().write_all(
                            format!(
                            "You are currently banned\nRemaining time: {remaining_secs} seconds\n"
                        )
                            .as_bytes(),
                        );
                        let _ = stream.as_ref().shutdown(net::Shutdown::Both);
                    }
                };
                // Let client know they are banned and time remaining
                if let MessageContent::ConnectRequest(stream) = &message.content {
                    let _ = (*stream).as_ref().write_all(
                        format!(
                            "You are currently banned\nRemaining time: {remaining_secs} seconds\n"
                        )
                        .as_bytes(),
                    );
                    let _ = (*stream).as_ref().shutdown(net::Shutdown::Both);
                }
                // Client is still banned
                true
            } else {
                // Client no longer banned
                log::debug!("Client {addr} is no longer banned");
                let _ = self.ban_list.remove(&addr.ip());
                false
            }
        } else {
            // Client was not banned
            false
        }
    }

    fn ban_client(&mut self, addr: SocketAddr, reason: &str) {
        log::info!(
            "Banning Client {addr}. Reason: {reason}. Ban time: {ban_time} seconds",
            ban_time = TOTAL_BAN_TIME.num_seconds()
        );
        self.ban_list.insert(addr.ip(), Utc::now());
        // Disconnect client
        if let Some(client) = self.clients.remove(&addr) {
            if let Some(stream) = client.stream {
                let _ = stream.as_ref().write_all(
                    format!(
                        "You have been banned\nReason: {reason}\nBan time: {ban_time} seconds\n",
                        ban_time = TOTAL_BAN_TIME.num_seconds()
                    )
                    .as_bytes(),
                );
                let _ = stream.as_ref().shutdown(net::Shutdown::Both);
            }
        }
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

            // Get reference to client info
            let client = insert_or_get_mut(
                &mut self.clients,
                message.author_addr,
                Client {
                    stream: None,
                    auth_timestamp: None,
                    last_message_timestamp: Utc::now(),
                    strike_count: 0,
                },
            );

            // Message rate limit
            let message_timestamp = Utc::now();
            if message_timestamp.signed_duration_since(client.last_message_timestamp)
                < MESSAGE_COOLDOWN_TIME
            {
                client.strike_count += 1;
                log::info!(
                    "Client {addr}: Strike {n}/{total}",
                    addr = message.author_addr,
                    n = client.strike_count,
                    total = MAX_STRIKE_COUNT
                );
                if client.strike_count >= MAX_STRIKE_COUNT {
                    client.strike_count = 0;
                    // Ban offending client
                    self.ban_client(message.author_addr, "Spamming");
                    continue;
                }
            } else {
                client.strike_count = 0;
            }

            // Handle message
            match message.content {
                MessageContent::ConnectRequest(stream) => {
                    // TODO: Improve connection method
                    if let Err(err) = self.connect_client(message.author_addr, stream.clone()) {
                        log::error!(
                            "Unable to connect Client {addr}: {err}",
                            addr = message.author_addr
                        );
                        let _ = stream.shutdown(net::Shutdown::Both);
                        continue;
                    }
                }

                MessageContent::DisconnetRequest => {
                    if let Err(err) = self.disconnect_client(message.author_addr) {
                        log::error!(
                            "Unable to disconnect Client {addr}: {err}",
                            addr = message.author_addr
                        );
                    }
                }

                MessageContent::Bytes(bytes) => {
                    // Filter out escape codes
                    let bytes_safe: Vec<u8> = bytes.into_iter().filter(|c| *c >= 32).collect();
                    // Verify if message if valid UTF-8
                    let text = match str::from_utf8(&bytes_safe) {
                        Err(err) => {
                            log::error!("Text from message in not valid UTF-8: {err}");
                            continue;
                        }
                        Ok(string) => string,
                    };

                    // Token challenge for unauthenticated clients
                    if client.auth_timestamp.is_none() {
                        log::debug!(
                            "Attempting validation from Client {addr} with token: {text}",
                            addr = message.author_addr,
                        );
                        match self.access_token.validate_bytes(&bytes_safe) {
                            Err(err) => {
                                log::error!("Unable to validate token: {err}");
                            }
                            Ok(auth_timestamp) => {
                                log::info!("Token successfully authenticated");
                                client.auth_timestamp = Some(auth_timestamp);
                            }
                        }
                        continue;
                    }

                    log::debug!(
                        "Message from Client {addr} to {dest}: {text_clean}",
                        addr = message.author_addr,
                        dest = message.destination,
                        text_clean = text.trim_end()
                    );

                    let message_safe = Message {
                        author_addr: message.author_addr,
                        destination: message.destination,
                        timestamp: message.timestamp,
                        content: MessageContent::Bytes(bytes_safe),
                    };
                    match message_safe.destination {
                        Destination::Server => {
                            todo!("Handle messages sent to Server")
                        }
                        Destination::Client(_peer_addr) => {
                            todo!("Handle private messages")
                        }
                        Destination::AllClients => {
                            // Broadcast message to other clients
                            if let Err(err) = self.broadcast_message(message_safe) {
                                log::error!("Unable to brodcast message: {err}");
                            }
                        }
                    }
                }
            }
        }
    }
}
