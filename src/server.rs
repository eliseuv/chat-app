use core::str;
use std::{
    collections::HashMap,
    fmt::Display,
    io::Write,
    net::{self, IpAddr, SocketAddr, TcpStream},
    sync::{mpsc::Receiver, Arc},
};

use anyhow::{anyhow, bail, Result};
use getrandom::getrandom;

use crate::messages::{Destination, Message, MessageContent};

// TODO: Fix vulnerability to `slow loris reader`
// TODO: Proper authentication
// TODO: Use `chrono` lib
// TODO: Proper authentication
// TODO: Move more load to the client thread

const TOTAL_BAN_TIME: chrono::Duration = chrono::Duration::seconds(5 * 60);
const MESSAGE_COOLDOWN_TIME: chrono::Duration = chrono::Duration::milliseconds(300);
const MAX_STRIKE_COUNT: u32 = 5;
const WELCOME_MESSAGE: &str = "# Welcome to the epic Чат server #\n";
pub const TOKEN_LENGTH: usize = 8;

// Access token
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Token(pub(crate) [u8; TOKEN_LENGTH]);

impl Token {
    // New empty buffer to store token
    pub(crate) const fn buffer() -> [u8; TOKEN_LENGTH] {
        [0; TOKEN_LENGTH]
    }

    // Generate new random access token
    fn generate() -> Result<Token> {
        let mut buffer = Token::buffer();
        if let Err(err) = getrandom(&mut buffer) {
            bail!("Unable to generate random token: {err}");
        }
        Ok(Token(buffer))
    }

    pub(crate) fn from_str(s: &str) -> Result<Self> {
        log::debug!("Token string: {s}");
        let slen = s.len();
        if slen % (2 * TOKEN_LENGTH) != 0 {
            Err(anyhow!("Invalid token string length: {slen}"))
        } else {
            let mut buffer = Token::buffer();
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
pub struct Server {
    receiver: Receiver<Message>,
    connected_clients: HashMap<SocketAddr, Arc<TcpStream>>,
    clients_db: HashMap<IpAddr, ClientInfo>,
    access_token: Token,
}

#[derive(Debug)]
struct ClientInfo {
    last_message_timestamp: chrono::DateTime<chrono::Utc>,
    ban_strike_count: u32,
    ban_timestamp: Option<chrono::DateTime<chrono::Utc>>,
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
            connected_clients: HashMap::new(),
            clients_db: HashMap::new(),
            access_token,
        })
    }

    // Insert new client entry to database
    fn add_new_client(&mut self, client_ip: IpAddr) {
        log::debug!("Inserting Client {client_ip} to database");
        if let Some(_prev_client) = self.clients_db.insert(
            client_ip,
            ClientInfo {
                last_message_timestamp: chrono::Utc::now(),
                ban_strike_count: 0,
                ban_timestamp: None,
            },
        ) {
            log::warn!("Replacing previous client")
        }
    }

    // Connect client to server
    fn connect_client(&mut self, author_addr: SocketAddr, stream: Arc<TcpStream>) -> Result<()> {
        let client_addr = stream.as_ref().peer_addr()?;

        // Check if author is the same as client connecting
        if client_addr != author_addr {
            bail!("Client {author_addr} requesting connection for different Client {client_addr}",);
        }

        // Add client to connected clients list
        if let Some(_prev_client) = self.connected_clients.insert(client_addr, stream.clone()) {
            log::warn!("Replacing previoulsy connected Client at {client_addr}");
        } else {
            log::debug!("Successfully connected new Client {client_addr}");
        }

        // Send welcome message
        stream.as_ref().write_all(WELCOME_MESSAGE.as_bytes())?;

        Ok(())
    }

    // Broadcast message to clients
    fn broadcast_message(&self, message: Message) -> Result<()> {
        let author_addr = message.author_addr;
        match message.content {
            MessageContent::Bytes(bytes) => {
                for (client_addr, client_stream) in self.connected_clients.iter() {
                    if *client_addr != message.author_addr {
                        log::debug!("Sending message from {author_addr} to Client {client_addr}");
                        let nbytes = client_stream.as_ref().write(&bytes)?;
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
                Ok(())
            }
            _ => Err(anyhow!("Invalid message type for bradcasting")),
        }
    }

    // Run server
    pub fn run(mut self) -> Result<()> {
        log::debug!("Launching chat server");
        println!("# Epic Чат server #");

        loop {
            // Try to receive a message
            let message = match self.receiver.recv() {
                Err(err) => {
                    log::error!("Server could not receive message: {err}");
                    continue;
                }
                Ok(message) => message,
            };

            // Generate server side message timestamp
            let message_timestamp = chrono::Utc::now();
            // Identify message author
            let author_addr = message.author_addr;
            log::debug!("Message from Client {author_addr} at {message_timestamp}");

            // Look for client in database
            let author_ip = author_addr.ip();
            match self.clients_db.get_mut(&author_ip) {
                None => {
                    // New Client
                    log::info!("Client {author_ip} is unknown");
                    match message.content {
                        // Only message valid for unknown client is connection request
                        MessageContent::ConnectRequest(stream, client_token) => {
                            // Perform first time connection
                            // Check token provided
                            if client_token != self.access_token {
                                let _ = stream.as_ref().write("Invalid token!\n".as_bytes());
                                let _ = stream.as_ref().shutdown(net::Shutdown::Both);
                                continue;
                            }
                            if let Err(err) = self.connect_client(author_addr, stream) {
                                log::error!(
                                                    "Unable perform first time connection to Client {author_addr}: {err}"
                                                );
                                continue;
                            }
                            self.add_new_client(author_addr.ip());
                        }
                        _ => {
                            log::warn!("Invalid message from unknown Client {author_addr}");
                            continue;
                        }
                    }
                }
                Some(client_info) => {
                    // Client is known to server
                    log::debug!("Client {author_ip} is known to Server");
                    // Check author ban status
                    log::debug!("Checking Client {author_ip} ban status");
                    if let Some(banned_at) = client_info.ban_timestamp {
                        let ban_end = banned_at + TOTAL_BAN_TIME;
                        if chrono::Utc::now() < ban_end {
                            // Client is still banned
                            let remaining_secs = ban_end
                                .signed_duration_since(chrono::Utc::now())
                                .num_seconds();
                            log::debug!("Client {author_ip} is currently banned. Remaining time: {remaining_secs} seconds");
                            // Let client know they are banned and time remaining
                            if let MessageContent::ConnectRequest(stream, _client_token) =
                                message.content
                            {
                                let _ = stream.as_ref().write_all(format!("You are currently banned\nRemaining time: {remaining_secs} seconds\n").as_bytes());
                                if let Err(err) = stream.as_ref().shutdown(net::Shutdown::Both) {
                                    log::error!(
                                        "Unable to shutdown Client {author_addr} stream: {err}"
                                    );
                                }
                            }
                            // Disconnect banned client if currently connected
                            if let Some(stream) = self.connected_clients.remove(&author_addr) {
                                if let Err(err) = stream.as_ref().shutdown(net::Shutdown::Both) {
                                    log::error!(
                                        "Unable to shutdown Client {author_addr} stream: {err}"
                                    );
                                }
                            };
                            continue;
                        } else {
                            // Ban time has expired
                            log::info!("Client {author_ip} is no longer banned");
                            client_info.ban_timestamp = None;
                        }
                    }

                    // Limit message rate
                    if message_timestamp.signed_duration_since(client_info.last_message_timestamp)
                        < MESSAGE_COOLDOWN_TIME
                    {
                        client_info.ban_strike_count += 1;
                        log::info!(
                            "Client {author_addr}: Strike {n}/{total}",
                            n = client_info.ban_strike_count,
                            total = MAX_STRIKE_COUNT
                        );
                        if client_info.ban_strike_count >= MAX_STRIKE_COUNT {
                            // Ban offending client
                            let ban_reason = "Spamming";
                            log::info!("Banned Client {author_addr}. Reason: {ban_reason}.");
                            client_info.ban_timestamp = Some(message_timestamp);
                            client_info.ban_strike_count = 0;
                            // Disconnect client
                            if let Some(stream) = self.connected_clients.remove(&author_addr) {
                                let _ = stream.as_ref().write_all(format!("You have been banned\nReason: {ban_reason}\nBan time: {ban_time} seconds\n", ban_time=TOTAL_BAN_TIME.num_seconds()).as_bytes());
                                if let Err(err) = stream.as_ref().shutdown(net::Shutdown::Both) {
                                    log::error!(
                                        "Unable to shutdown Client {author_addr} stream: {err}"
                                    );
                                }
                            }
                            continue;
                        }
                    } else {
                        client_info.ban_strike_count = 0;
                    }

                    // Handle message from known client
                    client_info.last_message_timestamp = message_timestamp;
                    match message.content {
                        MessageContent::ConnectRequest(stream, client_token) => {
                            // Check token provided
                            if client_token != self.access_token {
                                let _ = stream.as_ref().write("Invalid token!\n".as_bytes());
                                let _ = stream.as_ref().shutdown(net::Shutdown::Both);
                                continue;
                            }
                            if let Err(err) = self.connect_client(author_addr, stream) {
                                log::error!("Unable to connect Client {author_addr}: {err}");
                                continue;
                            }
                        }

                        MessageContent::DisconnetRequest => {
                            match self.connected_clients.remove(&author_addr) {
                                                None => log::error!(
                                                    "Attempting to disconnect Client {author_addr} unknown to Server"
                                                ),
                                                Some(stream) => {
                                                    if let Err(err) = stream.as_ref().shutdown(net::Shutdown::Both) {
                                                        log::error!("Unable to shutdown stream while disconnecting Client {author_addr}: {err}");
                                                        continue;
                                                    }
                                                    log::info!("Successfully disconnect Client {author_addr}");
                                                }
                                            }
                        }

                        MessageContent::Bytes(bytes) => {
                            // Filter out escape codes
                            let bytes_safe: Vec<u8> =
                                bytes.into_iter().filter(|c| *c >= 32).collect();
                            // Verify if message if valid UTF-8
                            let text = match str::from_utf8(&bytes_safe) {
                                Err(err) => {
                                    log::error!("Text from message in not valid UTF-8: {err}");
                                    continue;
                                }
                                Ok(string) => string,
                            };
                            log::debug!(
                                "Message from Client {author_addr} to {dest}: {text_clean}",
                                dest = message.destination,
                                text_clean = text.trim_end()
                            );
                            let message_safe = Message {
                                author_addr: message.author_addr,
                                destination: message.destination,
                                timestamp: message.timestamp,
                                content: MessageContent::Bytes(bytes_safe),
                            };
                            match message.destination {
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
    }
}
