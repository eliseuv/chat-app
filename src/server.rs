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

const TOTAL_BAN_TIME: time::Duration = time::Duration::from_secs(5 * 60);
const MESSAGE_COOLDOWN_TIME: time::Duration = time::Duration::from_millis(300);
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

    // Insert new client entry to database
    fn add_new_client(&mut self, client_ip: IpAddr) {
        log::debug!("Inserting Client {client_ip} to database");
        if let Some(_prev_client) = self.clients_db.insert(
            client_ip,
            ClientInfo {
                last_message_timestamp: time::SystemTime::now(),
                ban_strike_count: 0,
                ban_timestamp: None,
            },
        ) {
            log::warn!("Replacing previous client")
        }
    }

    // Connect client to server
    fn connect_client(
        &mut self,
        author_addr: SocketAddr,
        stream: Arc<TcpStream>,
    ) -> io::Result<()> {
        let client_addr = stream.as_ref().peer_addr()?;

        // Check if author is the same as client connecting
        if client_addr != author_addr {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "Client {author_addr} requesting connection for Client {client_addr}",
            ));
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
    fn broadcast_message(&self, message: Message) -> io::Result<()> {
        match message.author {
            Author::Server => {
                match message.content {
                    MessageContent::Bytes(bytes) => {
                        for (client_addr, client_stream) in self.connected_clients.iter() {
                            log::debug!("Sending message from Server to Client {client_addr}");
                            let nbytes = client_stream.as_ref().write(&bytes)?;
                            match nbytes.cmp(&bytes.len()) {
                                std::cmp::Ordering::Less => log::warn!(
                                    "Message partially sent: {nbytes}/{total} bytes sent",
                                    total = bytes.len()
                                ),
                                std::cmp::Ordering::Equal => {
                                    log::debug!("Successfully sent entire message")
                                }
                                std::cmp::Ordering::Greater => log::error!("More bytes sent than in the original message!?: {nbytes}/{total}", total=bytes.len()),
                            }
                        }
                        Ok(())
                    }
                    _ => Err(io::Error::new(
                        io::ErrorKind::Other,
                        "Invalid message type for bradcasting",
                    )),
                }
            }
            Author::Client(author_addr) => match message.content {
                MessageContent::Bytes(bytes) => {
                    for (client_addr, client_stream) in self.connected_clients.iter() {
                        if *client_addr != author_addr {
                            log::debug!(
                                "Sending message from {author_addr} to Client {client_addr}"
                            );
                            let nbytes = client_stream.as_ref().write(&bytes)?;
                            match nbytes.cmp(&bytes.len()) {
                                std::cmp::Ordering::Less => log::warn!(
                                    "Message partially sent: {nbytes}/{total} bytes sent",
                                    total = bytes.len()
                                ),
                                std::cmp::Ordering::Equal => {
                                    log::debug!("Successfully sent entire message")
                                }
                                std::cmp::Ordering::Greater => log::error!("More bytes sent than in the original message!?: {nbytes}/{total}", total=bytes.len()),
                            }
                        }
                    }
                    Ok(())
                }
                _ => Err(io::Error::new(
                    io::ErrorKind::Other,
                    "Invalid message type for bradcasting",
                )),
            },
        }
    }

    // Run server
    pub fn run(mut self) -> io::Result<()> {
        println!("# Epic Чат server #");

        loop {
            // Messages receive by the server
            match self.receiver.recv() {
                Err(err) => {
                    log::error!("Server could not receive message: {err}");
                    continue;
                }
                Ok(message) => {
                    log::debug!("Incoming message: {message}");
                    let message_timestamp = time::SystemTime::now();
                    // Identify message author
                    match message.author {
                        Author::Server => todo!("Handle messages from Server"),
                        Author::Client(author_addr) => {
                            // Check if client is known to server
                            let author_ip = author_addr.ip();
                            match self.clients_db.get_mut(&author_ip) {
                                None => {
                                    // Client is unknown to server
                                    log::info!("Client {author_ip} is unknown");
                                    match message.content {
                                        // Only message valid for unknown client is connection request
                                        MessageContent::ConnectRequest(stream) => {
                                            // Perform first time connection
                                            // TODO: Authentication
                                            if let Err(err) =
                                                self.connect_client(author_addr, stream)
                                            {
                                                log::error!(
                                                    "Unable perform first time connection to Client {author_addr}: {err}"
                                                );
                                                continue;
                                            }
                                            self.add_new_client(author_addr.ip());
                                        }
                                        _ => {
                                            log::warn!(
                                                "Invalid message from unknown Client {author_addr}"
                                            );
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
                                        let ban_elapsed = banned_at
                                            .elapsed()
                                            .expect("TODO: Handle clock going backwards");
                                        if ban_elapsed < TOTAL_BAN_TIME {
                                            // Client is still banned
                                            let remaining_secs =
                                                (TOTAL_BAN_TIME - ban_elapsed).as_secs();
                                            log::debug!("Client {author_ip} is currently banned. Remaining time: {remaining_secs} seconds");
                                            // Let client know they are banned and time remaining
                                            if let MessageContent::ConnectRequest(stream) =
                                                message.content
                                            {
                                                let _ = stream.as_ref().write_all(format!("You are currently banned\nRemaining time: {remaining_secs} seconds\n").as_bytes());
                                                if let Err(err) =
                                                    stream.as_ref().shutdown(net::Shutdown::Both)
                                                {
                                                    log::error!("Unable to shutdown Client {author_addr} stream: {err}");
                                                }
                                            }
                                            // Disconnect banned client if currently connected
                                            if let Some(stream) =
                                                self.connected_clients.remove(&author_addr)
                                            {
                                                if let Err(err) =
                                                    stream.as_ref().shutdown(net::Shutdown::Both)
                                                {
                                                    log::error!("Unable to shutdown Client {author_addr} stream: {err}");
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
                                    if message_timestamp
                                        .duration_since(client_info.last_message_timestamp)
                                        .expect("TODO: Handle clock going backwards")
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
                                            if let Some(stream) =
                                                self.connected_clients.remove(&author_addr)
                                            {
                                                let _ = stream.as_ref().write_all(format!("You have been banned\nReason: {ban_reason}\nBan time: {ban_time} seconds\n", ban_time=TOTAL_BAN_TIME.as_secs()).as_bytes());
                                                if let Err(err) =
                                                    stream.as_ref().shutdown(net::Shutdown::Both)
                                                {
                                                    log::error!("Unable to shutdown Client {author_addr} stream: {err}");
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
                                        MessageContent::ConnectRequest(stream) => {
                                            if let Err(err) =
                                                self.connect_client(author_addr, stream)
                                            {
                                                log::error!(
                                                    "Unable to connect Client {author_addr}: {err}"
                                                );
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

                                        MessageContent::Bytes(ref bytes) => {
                                            // Verify if message if valid UTF-8
                                            let text = match str::from_utf8(bytes) {
                                                Err(err) => {
                                                    log::error!("Text from message in not valid UTF-8: {err}");
                                                    continue;
                                                }
                                                Ok(string) => string,
                                            };
                                            log::debug!("Message from Client {author_addr} to {dest}: {text}", dest = message.destination);
                                            match message.destination {
                                                Destination::Server => {
                                                    todo!("Handle messages sent to Server")
                                                }
                                                Destination::Client(_peer_addr) => {
                                                    todo!("Handle private messages")
                                                }
                                                Destination::AllClients => {
                                                    // Broadcast message to other clients
                                                    if let Err(err) =
                                                        self.broadcast_message(message)
                                                    {
                                                        log::error!(
                                                            "Unable to brodcast message: {err}"
                                                        );
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
