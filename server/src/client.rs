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
use log::debug;

use crate::messages::{Destination, Message, MessageContent};

#[derive(Debug, Clone)]
pub struct Client {
    addr: SocketAddr,
    stream: Arc<TcpStream>,
    sender: Sender<Message>,
}

impl Display for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Client {addr}", addr = self.addr)
    }
}

impl Client {
    pub fn new(stream: TcpStream, sender: Sender<Message>) -> Result<Self> {
        let addr = stream.peer_addr()?;

        Ok(Self {
            addr,
            stream: Arc::new(stream),
            sender,
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
    ) -> Result<(), SendError<Message>> {
        let message = Message {
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

    // Run client
    pub fn run(&self) -> Result<()> {
        log::info!("Spawned thread for {self}");

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
