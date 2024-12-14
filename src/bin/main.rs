use std::{
    io::{self},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener},
    sync::mpsc::channel,
    thread,
};

use chat_app::{client::Client, messages::Message, server::Server};

// TODO: Better async. Look `tokio` lib
// TODO: Handle errors propeyly. Look `anyhow` lib
// TODO: Fix vulnerability for `slow loris reader`

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
                // Identify client
                let client_addr = match stream.peer_addr() {
                    Err(err) => {
                        log::error!("Could not retrieve client address: {err}");
                        continue;
                    }
                    Ok(addr) => addr,
                };
                log::info!("Incoming connection from {client_addr}");

                // Spawn client thread
                let client = Client::new(client_addr, stream, &message_sender);
                thread::spawn(move || client.run());
            }
        }
    }

    Ok(())
}
