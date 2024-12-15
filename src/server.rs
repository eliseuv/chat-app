use core::str;
use std::{
    collections::HashMap,
    io::{self, Write},
    net::{self, IpAddr, SocketAddr, TcpStream},
    sync::{mpsc::Receiver, Arc},
    time,
};

use crate::messages::{Author, Destination, Message, MessageContent};

// TODO: Fix vulnerability to `slow loris reader`
// TODO: Proper authentication

const BAN_TIME: time::Duration = time::Duration::from_secs(5 * 60);
const COOLDOWN_TIME: time::Duration = time::Duration::from_millis(300);
const MAX_STRIKE_COUNT: u32 = 5;
const WELCOME_MESSAGE: &str = "# Welcome to the epic Чат server #\n";

#[derive(Debug)]
pub struct Server {
    receiver: Receiver<Message>,
    connected_clients: HashMap<SocketAddr, Arc<TcpStream>>,
    clients_db: HashMap<IpAddr, ClientInfo>,
}

#[derive(Debug)]
struct ClientInfo {
    last_message_timestamp: time::SystemTime,
    ban_strike_count: u32,
    ban_timestamp: Option<time::SystemTime>,
}

impl Server {
    // Create new empty server
    pub fn new(receiver: Receiver<Message>) -> Self {
        Self {
            receiver,
            connected_clients: HashMap::new(),
            clients_db: HashMap::new(),
        }
    }

    fn connect_client(
        &mut self,
        author_addr: SocketAddr,
        stream: Arc<TcpStream>,
    ) -> io::Result<()> {
        let client_addr = stream.peer_addr()?;

        // Check if author is the same as client connecting
        if client_addr != author_addr {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "Client {author_addr} requesting connection for client {client_addr}",
            ));
        }

        // Add client to connected clients list
        if let Some(_prev_client) = self.connected_clients.insert(client_addr, stream.clone()) {
            log::warn!("Replacing previoulsy connected client at {client_addr}");
        } else {
            log::debug!("Successfully connected new client {client_addr}");
        }

        // Send welcome message
        stream.as_ref().write_all(WELCOME_MESSAGE.as_bytes())?;

        Ok(())
    }

    // Run server
    pub fn run(mut self) -> io::Result<()> {
        log::info!("Launching server");

        loop {
            match self.receiver.recv() {
                Err(err) => {
                    log::error!("Server could not receive message: {err}");
                    continue;
                }
                Ok(message) => {
                    // Identify author
                    match message.author {
                        Author::Server => todo!("Handle server messages"),
                        Author::Client(author_addr) => {
                            log::info!("Incoming message from {author_addr}");
                            // Check if client is known to server
                            match self.clients_db.get_mut(&author_addr.ip()) {
                                None => {
                                    // Client is unknown to server
                                    log::info!("Client {author_addr} unknown");
                                    match message.content {
                                        MessageContent::ConnectRequest(stream) => {
                                            // Perform first time connection
                                            if let Err(err) =
                                                self.connect_client(author_addr, stream)
                                            {
                                                log::error!(
                                                    "Unable to connect client {author_addr}: {err}"
                                                );
                                                continue;
                                            }

                                            // Add client info to db
                                            self.clients_db.insert(
                                                author_addr.ip(),
                                                ClientInfo {
                                                    last_message_timestamp: time::SystemTime::now(),
                                                    ban_strike_count: 0,
                                                    ban_timestamp: None,
                                                },
                                            );
                                        }
                                        _ => {
                                            log::warn!(
                                                "Invalid message from unknown client {author_addr}"
                                            );
                                        }
                                    }
                                }
                                Some(client_info) => {
                                    // Check author ban status
                                    log::debug!("Checking client {author_addr} ban status");
                                    if let Some(banned_at) = client_info.ban_timestamp {
                                        let ban_elapsed = banned_at
                                            .elapsed()
                                            .expect("TODO: Handle clock going backwards");
                                        if ban_elapsed < BAN_TIME {
                                            // Client is still banned
                                            let remaining_secs = (BAN_TIME - ban_elapsed).as_secs();
                                            log::debug!("Client {author_addr} is currently banned. Remaining time: {remaining_secs} seconds");
                                            if let Some(stream) =
                                                self.connected_clients.remove(&author_addr)
                                            {
                                                let _ = stream.as_ref().write_all(format!("You are currently banned\nRemaining time: {remaining_secs} seconds\n").as_bytes());
                                                if let Err(err) =
                                                    stream.as_ref().shutdown(net::Shutdown::Both)
                                                {
                                                    log::error!("Unable to shutdown client {author_addr} stream: {err}");
                                                }
                                            }
                                            continue;
                                        } else {
                                            // Ban time has expired
                                            log::debug!("Client {author_addr} no longer banned");
                                            client_info.ban_timestamp = None;
                                        }
                                    }

                                    // Limit message rate
                                    if client_info
                                        .last_message_timestamp
                                        .elapsed()
                                        .expect("TODO: Handle clock going backwards")
                                        < COOLDOWN_TIME
                                    {
                                        client_info.ban_strike_count += 1;
                                        log::info!(
                                            "Client {author_addr}: Strike {n}/{total}",
                                            n = client_info.ban_strike_count,
                                            total = MAX_STRIKE_COUNT
                                        );
                                        if client_info.ban_strike_count >= MAX_STRIKE_COUNT {
                                            client_info.ban_timestamp =
                                                Some(time::SystemTime::now());
                                            client_info.ban_strike_count = 0;
                                            let ban_reason = "Maximum strikes reached";
                                            log::info!("Banned client {author_addr}. Reason: {ban_reason}.");
                                            if let Some(stream) =
                                                self.connected_clients.remove(&author_addr)
                                            {
                                                let _ = stream.as_ref().write_all(format!("You have been banned\nReason: {ban_reason}\nBan time: {ban_time} seconds\n", ban_time=BAN_TIME.as_secs()).as_bytes());
                                                if let Err(err) =
                                                    stream.as_ref().shutdown(net::Shutdown::Both)
                                                {
                                                    log::error!("Unable to shutdown client {author_addr} stream: {err}");
                                                }
                                            }
                                            continue;
                                        }
                                    }
                                    client_info.last_message_timestamp = time::SystemTime::now();

                                    match message.content {
                                        MessageContent::ConnectRequest(stream) => {
                                            if let Err(err) =
                                                self.connect_client(author_addr, stream)
                                            {
                                                log::error!(
                                                    "Unable to connect client {author_addr}: {err}"
                                                );
                                                continue;
                                            }
                                        }

                                        MessageContent::DisconnetRequest => {
                                            log::info!(
                                                "Incoming disconnection request from client {author_addr}"
                                            );
                                            match self.connected_clients.remove(&author_addr) {
                                                None => log::error!(
                                                    "Attempting to disconnect client {author_addr} unknown to server"
                                                ),
                                                Some(stream) => {
                                                    if let Err(err) = stream.as_ref().shutdown(net::Shutdown::Both) {
                                                        log::error!("Unable to shutdown stream while disconnecting client {author_addr}: {err}");
                                                        continue;
                                                    }
                                                }
                                            }
                                        }

                                        MessageContent::Bytes(bytes) => {
                                            // Verify if message if valid UTF-8
                                            let text = match str::from_utf8(&bytes) {
                                                Err(err) => {
                                                    log::error!("Text from message in not valid UTF-8: {err}");
                                                    continue;
                                                }
                                                Ok(string) => string,
                                            };
                                            match message.destination {
                                                Destination::Server => todo!(),
                                                Destination::Client(_peer_addr) => todo!(),
                                                Destination::OtherClients => {
                                                    log::debug!(
                                                        "Client {author_addr} sent message to {dest:?}: {text}",
                                                        dest = message.destination
                                                    );
                                                    // Broadcast message to other clients
                                                    for (peer_addr, peer_stream) in
                                                        self.connected_clients.iter()
                                                    {
                                                        if *peer_addr != author_addr {
                                                            log::debug!(
                                                                "Sending message from {author_addr} to {peer_addr}"
                                                            );
                                                            match peer_stream.as_ref().write(&bytes) {
                                                                Err(err) => log::error!("Unable to send message from client {author_addr} to client {peer_addr}: {err}"),
                                                                Ok(nbytes) => if nbytes != bytes.len() {
                                                                    log::warn!("Message from {author_addr} partially sent to {peer_addr}: {nbytes}/{total_bytes} bytes sent", total_bytes=bytes.len());
                                                                } else {
                                                                    log::debug!("Successfully sent entire message")
                                                                },
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
                    }
                }
            }
        }
    }
}
