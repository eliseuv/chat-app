use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener},
    sync::mpsc::channel,
    thread,
};

use anyhow::{Context, Result};

use server::{client::Client, client_requests::ClientRequest, server::Server};

// TODO: Better async. Look `tokio` lib
// TODO: Use `anyhow` lib to better compose errors

const PORT: u16 = 6969;

fn main() -> Result<()> {
    simple_logger::SimpleLogger::new()
        .env()
        .with_colors(true)
        .with_local_timestamps()
        .init()
        .context("Unable to initialize logger")?;

    // Bind TCP listener to address
    let server_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), PORT);
    let tcp_listener = TcpListener::bind(server_addr)?;
    log::info!("Listening to address {server_addr}");

    // Create main messages channel
    let (message_sender, message_receiver) = channel::<ClientRequest>();

    // Launch server
    let server = Server::new(message_receiver).expect("Unable to create new Server");
    let access_token = server.access_token();
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
                    Ok(mut client) => {
                        let _ = thread::spawn(move || {
                            if let Err(err) = client.run(access_token) {
                                log::error!("Error in {client} thread: {err}",);
                                let _ = client.shutdown();
                            }
                        });
                    }
                }
            }
        }
    }

    Ok(())
}
