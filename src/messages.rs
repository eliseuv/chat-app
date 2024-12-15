use std::{
    net::{SocketAddr, TcpStream},
    sync::Arc,
    time,
};

#[derive(Debug)]
pub struct Message {
    pub(crate) author: Author,
    pub(crate) destination: Destination,
    pub(crate) timestamp: time::SystemTime,
    pub(crate) content: MessageContent,
}

#[derive(Debug)]
pub(crate) enum MessageContent {
    ConnectRequest(Arc<TcpStream>),
    DisconnetRequest,
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Author {
    Server,
    Client(SocketAddr),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Destination {
    Server,
    OtherClients,
    Client(SocketAddr),
}
