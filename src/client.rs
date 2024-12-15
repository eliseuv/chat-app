use std::{
    io::{self, Read},
    net::{self, SocketAddr, TcpStream},
    sync::{mpsc::Sender, Arc},
    time,
};

use crate::messages::{Author, Destination, Message, MessageContent};

#[derive(Debug, Clone)]
pub struct Client {
    pub(crate) stream: Arc<TcpStream>,
    pub(crate) sender: Sender<Message>,
    addr: SocketAddr,
}

impl Client {
    pub fn new(stream: TcpStream, sender: Sender<Message>) -> io::Result<Self> {
        let addr = stream.peer_addr()?;

        Ok(Self {
            stream: Arc::new(stream),
            sender,
            addr,
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
    ) -> io::Result<()> {
        let addr = self.addr;
        log::debug!("Client {addr} sending messege");
        let message = Message {
            author: Author::Client(addr),
            destination,
            timestamp: time::SystemTime::now(),
            content,
        };
        if let Err(err) = self.sender.send(message) {
            let mut error_message =
                format!("Client {addr} could not send connection request to server: {err}");
            if let Err(err) = self.stream.shutdown(net::Shutdown::Both) {
                error_message.push_str(&format!("\nFailed to shutdown stream :{err}"));
            }
            log::error!("{}", error_message);
            return Err(io::Error::new(io::ErrorKind::Other, error_message));
        }

        Ok(())
    }

    // Send connection request to server
    pub(crate) fn request_connect(&self) -> io::Result<()> {
        self.send_message(
            Destination::Server,
            MessageContent::ConnectRequest(self.stream.clone()),
        )
    }

    // Send disconnection request to server
    pub(crate) fn request_disconnect(&self) -> io::Result<()> {
        self.send_message(Destination::Server, MessageContent::DisconnetRequest)
    }

    // Run client
    pub fn run(&self) -> io::Result<()> {
        let addr = self.addr;
        log::info!("Spawning client thread for {addr}");
        self.request_connect()?;

        let mut buffer = vec![0; 64];
        loop {
            match self.stream.as_ref().read(&mut buffer) {
                Err(err) => {
                    log::error!("Client {addr} could not read message into buffer: {err}");
                    return self.request_disconnect();
                }
                Ok(nbytes) => {
                    if nbytes > 0 {
                        log::debug!("Client {addr} read {nbytes} bytes into buffer");
                        let bytes = buffer[0..nbytes].to_owned();
                        if let Err(err) = self
                            .send_message(Destination::OtherClients, MessageContent::Bytes(bytes))
                        {
                            log::error!("Client {addr} could not send message: {err}");
                        }
                    }
                }
            }
        }
    }
}
