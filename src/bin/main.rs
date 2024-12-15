use std::{
    io::{self},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener},
    sync::mpsc::channel,
    thread,
};

use chat_app::{client::Client, messages::Message, server::Server};

// TODO: Better async. Look `tokio` lib
// TODO: Handle errors propeyly. Look `anyhow` lib

const PORT: u16 = 6969;

fn main() -> io::Result<()> {
    env_logger::init();
    println!("# Epic Чат server #");

    // Bind TCP listener to address
    let server_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), PORT);
    let tcp_listener = TcpListener::bind(server_addr)?;
    log::info!("Listening to address {server_addr}");

    // Create main messages channel
    let (message_sender, message_receiver) = channel::<Message>();

    // Launch server
    let server = Server::new(message_receiver);
    thread::spawn(move || server.run());

    // Listen to incoming TCP connections
    for incoming_stream in tcp_listener.incoming() {
        // Handle TCP connections
        match incoming_stream {
            Err(err) => log::error!("Could not handle incoming TCP connection: {err}"),
            Ok(stream) => {
                // Spawn client thread
                match Client::new(stream, message_sender.clone()) {
                    Err(err) => log::error!("Unable to create client: {err}"),
                    Ok(client) => {
                        log::info!("Incoming connection from {addr}", addr = client.addr());
                        thread::spawn(move || client.run());
                    }
                }
            }
        }
    }

    Ok(())
}
