use core::str;
use std::{
    collections::HashMap,
    fmt::Display,
    net::{self, IpAddr, SocketAddr, TcpStream},
    sync::{mpsc::Receiver, Arc},
};

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, TimeDelta, Utc};
use getrandom::getrandom;

use crate::{
    client_requests::{BanReason, ClientRequest, Request},
    remote::{Author, Message},
};

// TODO: Authentication
// TODO: Move more load to the client thread
// TODO: Fix vulnerability to `slow loris reader`

/// Total a client remains banned
const TOTAL_BAN_TIME: TimeDelta = TimeDelta::seconds(5 * 60);

/// Server access token length in bytes
pub const TOKEN_LENGTH: usize = 8;

/// Server Access Token
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token([u8; TOKEN_LENGTH]);

impl Token {
    /// Generate new random access token
    fn generate() -> Result<Token> {
        let mut buffer = [0; TOKEN_LENGTH];
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

        let mut buffer = [0; TOKEN_LENGTH];
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

fn write_to(stream: &TcpStream, text: String) -> Result<()> {
    Message::new(Author::Server, text)
        .write_to(stream)
        .context("Unable to write message to client stream")
}

#[derive(Debug)]
struct Client {
    id: usize,
    stream: Arc<TcpStream>,
}

#[derive(Debug)]
pub struct Server {
    receiver: Receiver<ClientRequest>,
    access_token: Token,
    ban_list: HashMap<IpAddr, DateTime<Utc>>,
    clients: HashMap<SocketAddr, Client>,
}

impl Server {
    /// Create new empty Server
    pub fn new(receiver: Receiver<ClientRequest>) -> Result<Self> {
        log::trace!("Creating new Server");

        // Generate access token
        let access_token = Token::generate()?;
        log::info!("Access token: {access_token}");

        Ok(Self {
            receiver,
            access_token,
            ban_list: HashMap::new(),
            clients: HashMap::new(),
        })
    }

    pub fn access_token(&self) -> Token {
        self.access_token
    }

    /// Filter messages from banned IPs. Returns is banned boolean.
    fn ban_filter(&mut self, request: &ClientRequest) -> bool {
        let addr = request.addr;
        let ip_addr = addr.ip();
        log::trace!("Checking IP {ip_addr} ban status");
        if let Some(banned_at) = self.ban_list.get(&ip_addr) {
            // Calculate ban time remaining
            let remaining_secs = (*banned_at + TOTAL_BAN_TIME)
                .signed_duration_since(Utc::now())
                .num_seconds();
            if remaining_secs > 0 {
                log::debug!(
                    "IP {ip_addr} is currently banned. Remaining time: {remaining_secs} seconds"
                );
                // Disconnect banned client if currently connected
                if let Some(client) = self.clients.remove(&addr) {
                    let _ = write_to(
                        client.stream.as_ref(),
                        format!(
                            "You are currently banned\nRemaining time: {remaining_secs} seconds\n"
                        ),
                    );
                } else {
                    // Refuse Connect Request
                    if let Request::Connect(stream) = &request.request {
                        let _ = write_to(
                            stream.as_ref(),
                            format!(
                            "You are currently banned\nRemaining time: {remaining_secs} seconds\n"
                        ),
                        );
                        let _ = (*stream).as_ref().shutdown(net::Shutdown::Both);
                    }
                }
                // Client is still banned
                true
            } else {
                // Client no longer banned
                log::info!("Client {ip_addr} has been unbanned");
                let _ = self.ban_list.remove(&ip_addr);
                false
            }
        } else {
            // Client was not banned
            false
        }
    }

    /// Connect client to server
    fn connect_client(&mut self, addr: SocketAddr, stream: Arc<TcpStream>) -> Result<()>{
        let id = self.clients.len() + 1;

        if let Some(prev_client) = self.clients.insert(addr, Client{ id, stream }){
            self.clients.insert(addr, prev_client);
            bail!("Client {addr} already connected");
        }


        Ok(())

    }

    /// Disconnect client from server
    fn disconnect_client(&mut self, addr: SocketAddr) -> Result<()> {
        log::info!("Disconneting Client {addr}");
        match self.clients.remove(&addr) {
            None => bail!("Attempting to disconnect already disconnected Client {addr}"),
            Some(client) => {
                client
                    .stream
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

    fn send_to_client(&self, client: &Client, message: &Message) ->Result<()> {
        ciborium::into_writer(message, client.stream.as_ref()).context("Unable to send message")
    }

    fn broadcast(&self, author_addr: SocketAddr, text: &str) -> Result<()> {
        log::trace!("Broadcasting message from client {author_addr}");
        let id = self.get_client_id(author_addr)?;
        let author = Author::Client(id);
        let message = Message::new(author, text.to_owned());
        log::debug!("Message: {message:?}");
        self.clients.iter().filter(|(peer_addr, _)| **peer_addr != author_addr ).for_each(|(peer_addr, peer_client)| 
            {
                log::debug!("Sending message from Client {author_addr} to Client {peer_addr}");
                if let Err(err) = self.send_to_client(peer_client, &message) {
                    log::error!(
                        "Unable to broadcast message from Client {author_addr} to Client {peer_addr}: {err}"
                    );
                }

            });
        Ok(())
    }

    // Shutdown client, optionally sending a final message
    fn shutdown_client(&mut self, addr: SocketAddr, text: Option<&str>) {
        log::info!("Shutting down Client {addr}");
        if let Some(client) = self.clients.remove(&addr) {
            if let Some(text) = text {
                let _ = write_to(client.stream.as_ref(), text.to_owned());
            }
            let _ = client.stream.as_ref().shutdown(net::Shutdown::Both);
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


    /// Run server
    pub fn run(mut self) -> Result<()> {
        log::trace!("Launching chat server");

        // Main server loop
        loop {
            // Try to receive a request from a client thread
            let request = match self.receiver.recv() {
                Err(err) => {
                    log::error!("Server could not receive message: {err}");
                    continue;
                }
                Ok(request) => request,
            };
            log::debug!("Server received message: {request}");

            // Ban filter
            if self.ban_filter(&request) {
                continue;
            }

            // Address of the client that made the request
            let addr = request.addr;

            // Handle client request
            match request.request {
                Request::Connect(stream) => {
                    if let Err(err) = self.connect_client(addr, stream.clone()) {
                        log::error!("Unable to connect Client {addr}: {err}");
                        let _ = stream.shutdown(net::Shutdown::Both);
                    }
                }

                Request::Disconnet => {
                    if let Err(err) = self.disconnect_client(addr) {
                        log::error!("Unable to disconnect Client {addr}: {err}");
                    }
                }

                Request::Ban(reason) => {
                    self.ban_client(addr, reason);
                }

                Request::Broadcast(text) => {
                    log::info!("Client {addr} says: {text}");
                    if let Err(err) = self.broadcast(addr, &text) {
                        log::error!("Unable to broadcast message: {err}");
                    }
                }
            }
        }
    }
}
