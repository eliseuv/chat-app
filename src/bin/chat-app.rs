use std::{
    io::{self},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener},
    sync::mpsc::channel,
    thread,
};

use chat_app::{client::Client, messages::Message, server::Server};

// TODO: Better async. Look `tokio` lib
// TODO: Use `anyhow` lib to better compose errors

const PORT: u16 = 6969;

fn main() -> io::Result<()> {
    env_logger::init();

    // Bind TCP listener to address
    let server_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), PORT);
    let tcp_listener = TcpListener::bind(server_addr)?;
    log::info!("Listening to address {server_addr}");

    // Create main messages channel
    let (message_sender, message_receiver) = channel::<Message>();

    // Launch server
    let server = Server::new(message_receiver).expect("Unable to create new Server");
    let _server_handle = thread::spawn(move || server.run());

    // Listen to incoming TCP connections
    for incoming_stream in tcp_listener.incoming() {
        // Handle TCP connections
        match incoming_stream {
            Err(err) => log::error!("Could not handle incoming TCP connection: {err}"),
            Ok(stream) => {
                // Spawn client thread
                match Client::new(stream, message_sender.clone()) {
                    Err(err) => log::error!("Unable to create new Client: {err}"),
                    Ok(client) => {
                        let _client_handle = thread::spawn(move || {
                            if let Err(err) = client.run() {
                                log::error!("Error in {client} thread: {err}",);
                                return Err(err);
                            }
                            Ok(())
                        });
                    }
                }
            }
        }
    }

    Ok(())
}
