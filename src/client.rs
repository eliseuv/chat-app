use core::str;
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    sync::{
        mpsc::{SendError, Sender},
        Arc,
    },
};

use anyhow::{anyhow, bail, Result};
use log::debug;

use crate::{
    messages::{Destination, Message, MessageContent},
    server,
};

#[derive(Debug, Clone)]
pub struct Client {
    addr: SocketAddr,
    stream: Arc<TcpStream>,
    sender: Sender<Message>,
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

    pub fn shutdown(&self) -> Result<()> {
        self.request_disconnect()?;
        self.stream.as_ref().shutdown(std::net::Shutdown::Both)?;
        Ok(())
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
        let _ = write!(self.stream.as_ref(), "Token: ");

        let mut buffer = [0; 2 * server::TOKEN_LENGTH];
        let nbytes = self.stream.as_ref().read(&mut buffer)?;
        if nbytes != buffer.len() {
            let _ = self.stream.as_ref().write("Invalid token!\n".as_bytes())?;
            bail!("Invalid token length: {nbytes}");
        }
        let token_str = str::from_utf8(&buffer)?;
        let token = server::Token::from_str(token_str)?;

        log::debug!(
            "Client {addr} sending Connect Request to server with token {token}",
            addr = self.addr,
        );
        self.send_message(
            Destination::Server,
            MessageContent::ConnectRequest(self.stream.clone(), token),
        )
        .map_err(|err| anyhow!("Unable to send Connect Request to Server: {err}"))
    }

    // Send disconnection request to server
    fn request_disconnect(&self) -> Result<()> {
        self.send_message(Destination::Server, MessageContent::DisconnetRequest)
            .map_err(|err| anyhow!("Unable to send Disconnect Request to Server: {err}"))
    }

    // Run client
    pub fn run(&self) -> Result<()> {
        let addr = self.addr;
        log::info!("Spawned thread for Client {addr}");

        // Send Connect Request to Server
        if let Err(err) = self.request_connect() {
            let _ = self.shutdown();
            return Err(err);
        }

        // Chat loop
        let mut buffer = vec![0; 64];
        loop {
            match self.stream.as_ref().read(&mut buffer) {
                Err(err) => {
                    let _ = self.shutdown();
                    return Err(err.into());
                }
                Ok(nbytes) => {
                    if nbytes > 0 {
                        log::debug!("Client {addr} read {nbytes} bytes into buffer");
                        let bytes = buffer[0..nbytes].to_owned();
                        if let Err(err) =
                            self.send_message(Destination::AllClients, MessageContent::Bytes(bytes))
                        {
                            log::error!("Client {addr} could not send message: {err}");
                        }
                    } else {
                        log::debug!("Client {addr} reached EOF");
                        return self.shutdown();
                    }
                }
            }
        }
    }
}
