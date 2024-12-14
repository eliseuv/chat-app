use std::{
    collections::HashMap,
    io::{self, Write},
    net::{self, IpAddr, SocketAddr},
    sync::mpsc::Receiver,
    time,
};

use crate::{
    client::Client,
    messages::{Author, Destination, Message, MessageContent},
};

const BAN_TIME: time::Duration = time::Duration::from_secs(5 * 60);

struct ClientInfo {
    last_message_timestamp: time::SystemTime,
    ban_strike_count: u32,
}

#[derive(Debug)]
pub struct Server {
    receiver: Receiver<Message>,
    clients: HashMap<SocketAddr, Client>,
    ban_list: HashMap<IpAddr, time::SystemTime>,
}

impl Server {
    // Create new empty server
    pub fn new(receiver: Receiver<Message>) -> Self {
        Self {
            receiver,
            clients: HashMap::new(),
            ban_list: HashMap::new(),
        }
    }

    pub(crate) fn send_message(&self, client_addr: SocketAddr, bytes: &[u8]) {
        log::debug!("Sending message from server to client {client_addr}");
        if let Err(err) = self.clients[&client_addr].sender.send(Message {
            author: Author::Server,
            destination: Destination::Client(client_addr),
            timestamp: time::SystemTime::now(),
            content: MessageContent::Bytes(bytes.into()),
        }) {
            log::error!("Server could not send message to client {client_addr}: {err}");
        } else {
            log::debug!("Successfully sent message from server to client {client_addr}");
        }
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
                            // Identify client author
                            let author = match self.clients.get(&author_addr) {
                                None => {
                                    log::error!(
                                        "Unable to find author {author_addr} in clients list"
                                    );
                                    continue;
                                }
                                Some(c) => c,
                            };

                            // Check author ban status
                            log::debug!("Checking client {author_addr} ban status");
                            if let Some(ban_timestamp) = self.ban_list.get(&author_addr.ip()) {
                                let ban_elapsed = ban_timestamp
                                    .elapsed()
                                    .expect("TODO: Handle clock going backwards");
                                if ban_elapsed < BAN_TIME {
                                    let remaining_secs = (BAN_TIME - ban_elapsed).as_secs();
                                    log::debug!("Client {author_addr} is currently banned. Remaining time: {remaining_secs} seconds");
                                    self.send_message(author_addr, "You are currently banned\nRemaining time: {remaining_secs} seconds\n".as_bytes());
                                    if let Err(err) = author.stream.shutdown(net::Shutdown::Both) {
                                        log::error!(
                                            "Unable to shutdown client {author_addr} stream: {err}"
                                        );
                                    }
                                    continue;
                                } else {
                                    // Ban time has expired
                                    log::debug!("Client {author_addr} no longer banned");
                                    self.ban_list
                                        .remove(&author_addr.ip())
                                        .expect("Client guaranteed to be in ban list");
                                }
                            }

                            // Handle message
                            match message.content {
                                MessageContent::ConnectRequest(client) => {
                                    log::info!(
                                        "Incoming connection request from client {author_addr}"
                                    );

                                    // Check if author is the same as client connecting
                                    let client_addr = client.addr;
                                    if client_addr != author_addr {
                                        log::error!("Client {author_addr} requesting connection for client {client_addr}");
                                        continue;
                                    }

                                    // Add client to connected clients list
                                    if let Some(_prev_client) =
                                        self.clients.insert(client_addr, client)
                                    {
                                        log::warn!("Replacing previoulsy connected client at {client_addr}");
                                    } else {
                                        log::debug!(
                                            "Successfully connected new client {client_addr}"
                                        );
                                    }

                                    // Send welcome message
                                    self.send_message(
                                        client_addr,
                                        "# Welcome to the epic Чат server #\n".as_bytes(),
                                    );
                                }

                                MessageContent::DisconnetRequest => {
                                    log::info!(
                                        "Incoming disconnection request from client {author_addr}"
                                    );
                                    match self.clients.remove(&author_addr) {
                                        None => log::error!(
                                            "Attempting to disconnect client {author_addr} unknown to server"
                                        ),
                                        Some(client) => {
                                            if let Err(err) = client.stream.shutdown(net::Shutdown::Both) {
                                                log::error!("Unable to shutdown stream while disconnecting client {author_addr}");
                                                return Err(err);
                                            }
                                        }
                                    }
                                }

                                MessageContent::Bytes(bytes) => {
                                    match message.destination {
                                        Destination::Server => todo!(),
                                        Destination::Client(_peer_addr) => todo!(),
                                        Destination::OtherClients => {
                                            // Broadcast message to other clients
                                            for (peer_addr, peer_client) in self.clients.iter() {
                                                if *peer_addr != author_addr {
                                                    log::debug!(
                                                        "Sending message from {author_addr} to {peer_addr}"
                                                    );
                                                    match peer_client.stream.as_ref().write(&bytes) {
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
