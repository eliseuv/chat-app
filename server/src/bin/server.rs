use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener},
    sync::mpsc,
    thread,
};

use anyhow::{Context, Result};

use server::{client::Client, requests::ClientRequest, server::Server};

// TODO: Better async. Look `tokio` lib

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
    let tcp_listener = TcpListener::bind(server_addr).context("Unable to bind TCP listener")?;
    log::info!("Listening to address {server_addr}");

    // Requests channel
    let (request_sender, request_receiver) = mpsc::channel::<ClientRequest>();

    // Launch server
    let server = Server::new(request_receiver).context("Unable to create new Server")?;
    let access_token = server.access_token();
    let _server_handle = thread::spawn(move || server.run());

    // Listen to incoming TCP connections
    for incoming_stream in tcp_listener.incoming() {
        // Handle TCP connections
        match incoming_stream {
            Err(e) => log::error!("Could not handle incoming TCP connection: {e}"),
            Ok(stream) => {
                // Spawn client thread
                match Client::new(stream, request_sender.clone()) {
                    Err(e) => log::error!("Unable to create new Client: {e}"),
                    Ok(mut client) => {
                        let _ = thread::spawn(move || {
                            if let Err(e) = client.run(access_token) {
                                log::error!("Error in {client} thread: {e}",);
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
