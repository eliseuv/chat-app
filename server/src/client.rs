use std::{
    fmt::Display,
    io::Read,
    net::{SocketAddr, TcpStream},
    sync::{
        mpsc::{SendError, Sender},
        Arc,
    },
};

use anyhow::{anyhow, Result};
use chrono::{DateTime, TimeDelta, Utc};
use log::debug;

use crate::local_messages::{Destination, LocalMessage, MessageContent};

const MESSAGE_COOLDOWN_TIME: TimeDelta = TimeDelta::milliseconds(300);
const MAX_STRIKE_COUNT: u32 = 5;

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
        let addr = stream.peer_addr()?;

        Ok(Self {
            addr,
            stream: Arc::new(stream),
            sender,
            last_message_time: Utc::now(),
            strike_count: 0,
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    // Send a message from this client
    pub(crate) fn send_message(
        &self,
        destination: Destination,
        content: MessageContent,
    ) -> Result<(), SendError<LocalMessage>> {
        let message = LocalMessage {
            author_addr: self.addr,
            destination,
            timestamp: chrono::Utc::now(),
            content,
        };

        debug!("Client {addr} sending {message}", addr = self.addr);
        self.sender.send(message)
    }

    // Send connection request to server
    fn request_connect(&self) -> Result<()> {
        self.send_message(
            Destination::Server,
            MessageContent::ConnectRequest(self.stream.clone()),
        )
        .map_err(|err| anyhow!("{self} unable to send Connect Request to Server: {err}"))
    }

    // Send disconnection request to server
    fn request_disconnect(&self) -> Result<()> {
        self.send_message(Destination::Server, MessageContent::DisconnetRequest)
            .map_err(|err| anyhow!("{self} unable to send Disconnect Request to Server: {err}"))
    }

    pub fn shutdown(&self) -> Result<()> {
        self.request_disconnect()?;
        self.stream.as_ref().shutdown(std::net::Shutdown::Both)?;
        Ok(())
    }

    fn rate_limiter(&mut self) {
        let message_time = Utc::now();
        if message_time.signed_duration_since(self.last_message_time) < MESSAGE_COOLDOWN_TIME {
            self.strike_count += 1;
            log::info!(
                "Client {addr}: Strike {n}/{total}",
                addr = self.addr,
                n = self.strike_count,
                total = MAX_STRIKE_COUNT
            );
            if self.strike_count >= MAX_STRIKE_COUNT {
                self.strike_count = 0;
                // Ban offending client
                self.send_message(Destination::Server, MessageContent::BanMe);
            }
        } else {
            // Reset strikes
            self.strike_count = 0;
        }
        self.last_message_time = message_time;
    }

    // Run client
    pub fn run(&mut self) -> Result<()> {
        log::info!("Spawned thread for {self}");

        // Message rate limit
        // FIXME: A new client gets a strike on first Connect Request
        self.rate_limiter();

        // Send Connect Request to Server
        if let Err(err) = self.request_connect() {
            let _ = self.shutdown();
            return Err(err);
        }

        // Chat loop
        let mut buffer = [0; 64];
        loop {
            match self.stream.as_ref().read(&mut buffer) {
                Err(err) => {
                    let _ = self.shutdown();
                    return Err(err.into());
                }
                Ok(nbytes) => {
                    if nbytes > 0 {
                        log::debug!("{self} read {nbytes} bytes into buffer");
                        let bytes = buffer[0..nbytes].to_owned();
                        if let Err(err) =
                            self.send_message(Destination::AllClients, MessageContent::Bytes(bytes))
                        {
                            log::error!("{self} could not send message: {err}");
                        }
                    } else {
                        log::debug!("{self} reached EOF");
                        return self.shutdown();
                    }
                }
            }
        }
    }
}
