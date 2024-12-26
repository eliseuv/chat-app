use core::str;
use std::{
    io::{self, Write},
    net::{SocketAddr, TcpStream},
    thread, time,
};

use anyhow::{bail, Context, Result};
use chrono::{DateTime, TimeZone};
use clap::Parser;
use crossterm::{
    cursor::{self, MoveTo},
    event::{self, Event, KeyCode, KeyModifiers},
    style::Print,
    terminal::{self, Clear, ClearType},
    tty::IsTty,
    QueueableCommand,
};

use server::messages::{self, MessageToClient, PeerMessage};

// TODO: Read message struct directly from stream, without buffer
// TODO: Separate read message from stream and process it
// TODO: Send serialized message struct to server
// TODO: Better authentication step
// TODO: UI: Wrap lines
// TODO: UI: Persistent prompt content on resize

/// Get local datetime from message timestamp
fn datetime(timestamp: i64) -> Result<DateTime<chrono::Local>> {
    chrono::Local
        .timestamp_opt(timestamp, 0)
        .single()
        .context("Unable to convert timestamp to local timezone")
}

#[derive(Debug)]
struct Rect {
    x: u16,
    y: u16,
    w: u16,
    h: u16,
}

#[derive(Debug)]
struct Prompt {
    content: String,
    is_full: bool,
    max_width: u16,
}

impl Prompt {
    fn new(width: u16) -> Self {
        Self {
            content: String::new(),
            is_full: false,
            max_width: width - 3,
        }
    }

    fn is_full(&self) -> bool {
        self.is_full
    }

    fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    fn text(&self) -> &str {
        &self.content
    }

    fn resize(&mut self, width: u16) {
        self.max_width = width - 3;
        self.content.truncate(self.max_width as usize);
    }

    fn clear(&mut self) {
        self.content.clear()
    }

    fn push(&mut self, ch: char) {
        if self.content.len() < self.max_width as usize {
            self.content.push(ch);
        } else {
            self.is_full = true;
        }
    }

    fn pop(&mut self) -> Option<char> {
        let ch = self.content.pop();
        if self.content.len() < self.max_width as usize {
            self.is_full = false;
        }
        ch
    }

    fn push_str(&mut self, string: &str) {
        if self.content.len() + string.len() < self.max_width as usize {
            self.content.push_str(string)
        }
    }
}

/// Messages stored in the client
#[derive(Debug)]
enum Message {
    /// Message received
    Received(MessageToClient),
    /// Message sent from this client
    Sent {
        timestamp: i64,
        content: PeerMessage,
    },
}

impl Message {
    fn string(&self) -> Result<String> {
        match self {
            Message::Received(message) => match message.author {
                messages::MessageAuthor::Server(ref content) => match content {
                    messages::ServerMessage::Ban(reason) => {
                        Ok(format!("You have been banned. Reason: {reason}"))
                    }
                    messages::ServerMessage::Text(text) => Ok(format!(
                        "[{dt}] Server: {text}",
                        dt = datetime(message.timestamp)?.format("%d/%m/%Y %H:%M:%S")
                    )),
                },
                messages::MessageAuthor::Peer { id, ref content } => match content {
                    messages::PeerMessage::Text(text) => Ok(format!(
                        "[{dt}] User {id}: {text}",
                        dt = datetime(message.timestamp)?.format("%d/%m/%Y %H:%M:%S")
                    )),
                },
            },
            Message::Sent { timestamp, content } => match content {
                PeerMessage::Text(text) => Ok(format!(
                    "[{dt}] You: {text}",
                    dt = datetime(*timestamp)?.format("%d/%m/%Y %H:%M:%S")
                )),
            },
        }
    }
}

/// Application state
#[derive(Debug, PartialEq, Eq)]
enum State {
    Default,
    Quit,
}

#[derive(Debug)]
struct ClientInterface<T>
where
    T: io::Write + QueueableCommand + IsTty,
{
    output: T,
    width: u16,
    height: u16,
    prompt: Prompt,
    chat: Vec<Message>,
    stream: TcpStream,
    state: State,
}

impl<T> ClientInterface<T>
where
    T: io::Write + QueueableCommand + IsTty,
{
    fn new(output: T, stream: TcpStream) -> Result<Self> {
        if !output.is_tty() {
            bail!("Output is not tty")
        }

        let (width, height) = terminal::size()?;
        Ok(Self {
            output,
            width,
            height,
            prompt: Prompt::new(width),
            chat: Vec::new(),
            stream,
            state: State::Default,
        })
    }

    fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
        self.prompt.resize(width);
    }

    fn flush(&mut self) -> Result<()> {
        self.output.flush().context("Unable to flush output")
    }

    fn queue_write_on_center(&mut self, text: &str) -> Result<&mut T> {
        self.output
            .queue(MoveTo(
                (self.width - text.len() as u16) / 2,
                self.height / 2,
            ))?
            .queue(Print(text))
            .context("Unable to on the center of the screen")
    }

    fn draw_cover(&mut self) -> Result<()> {
        self.output.queue(Clear(ClearType::All))?;
        self.queue_write_on_center("chat app")?;
        self.output.flush().context("Unable to draw cover")
    }

    fn queue_draw_prompt(&mut self) -> Result<&mut T> {
        self.output
            .queue(MoveTo(0, self.height - 2))?
            .queue(Print("━".repeat(self.width as usize)))?
            .queue(MoveTo(0, self.height - 1))?
            .queue(Print(" > "))?
            .queue(Print(self.prompt.text()))
            .context("Unable to draw prompt")
    }

    fn queue_draw_chat(&mut self, rect: Rect) -> Result<()> {
        self.chat
            .iter()
            .skip(self.chat.len().saturating_sub(rect.h as usize))
            .filter_map(|message| message.string().ok())
            .enumerate()
            .try_fold(self.output.queue(cursor::Show)?, |cmd, (row, line)| {
                cmd.queue(MoveTo(rect.x, rect.y + row as u16))?
                    .queue(Print(line.get(0..rect.w as usize).unwrap_or(&line)))
            })
            .map(|_| ())
            .context("Unable to print chat")
    }

    fn draw_main(&mut self) -> Result<()> {
        // Cleanup
        self.output.queue(Clear(ClearType::All))?;
        // Draw Chat
        self.queue_draw_chat(Rect {
            x: 0,
            y: 0,
            w: self.width,
            h: self.height - 2,
        })?;
        // Prompt
        self.queue_draw_prompt()?;

        self.flush()
    }

    fn handle_event(&mut self) -> Result<()> {
        let new_event = event::read()?;
        log::debug!("Handling event: {new_event:?}");

        match new_event {
            Event::Resize(width, height) => {
                self.resize(width, height);
                self.draw_main()?;
            }
            Event::Paste(data) => {
                self.prompt.push_str(&data);
            }
            Event::Key(key_event) => match key_event.code {
                KeyCode::Char(c) => {
                    if c == 'd' && key_event.modifiers.contains(KeyModifiers::CONTROL) {
                        self.state = State::Quit;
                        return Ok(());
                    }
                    if !self.prompt.is_full() {
                        self.prompt.push(c);
                    }
                }
                KeyCode::Backspace => {
                    let _ = self.prompt.pop();
                }
                KeyCode::Enter => {
                    if !self.prompt.is_empty() {
                        match self.stream.write(self.prompt.text().as_bytes()) {
                            Err(e) => log::error!("Unable to send data: {e}"),
                            Ok(n) => log::info!("Successfully sent {n} bytes"),
                        }
                        self.chat.push(Message::Sent {
                            timestamp: chrono::Local::now().timestamp(),
                            content: PeerMessage::Text(self.prompt.text().to_string()),
                        });
                        self.prompt.clear();
                    }
                }
                _ => {}
            },
            _ => {}
        }
        Ok(())
    }

    /// Read incoming data from stream
    fn read_stream(&mut self) -> Result<()> {
        match MessageToClient::read_from(&self.stream) {
            Err(e) => {
                // Ignore `WouldBlock` errors
                if let ciborium::de::Error::Io(err) = e {
                    if err.kind() == io::ErrorKind::WouldBlock {
                        Ok(())
                    } else {
                        Err(err).context("Unable to read from stream due to IO error")
                    }
                } else {
                    Err(e).context("Unable to read from stream due to parsing error")
                }
            }
            Ok(message) => {
                self.chat.push(Message::Received(message));
                Ok(())
            }
        }
    }

    fn run(&mut self) -> Result<()> {
        terminal::enable_raw_mode()?;

        self.draw_cover()?;

        // Main loop
        loop {
            match self.state {
                State::Quit => {
                    terminal::disable_raw_mode()?;
                    return Ok(());
                }
                State::Default => {
                    // Poll for new event
                    while event::poll(time::Duration::ZERO)? {
                        if let Err(e) = self.handle_event() {
                            log::error!("Error handling event: {e}");
                        }
                    }

                    if let Err(e) = self.read_stream() {
                        log::error!("Error reading from stream: {e}");
                        continue;
                    };

                    self.draw_main()?;

                    // 60 FPS
                    thread::sleep(time::Duration::from_nanos(1_000_000_000 / 60));
                }
            }
        }
    }
}

/// Command line arguments
#[derive(Debug, Parser)]
#[command(version, about, long_about=None)]
struct Args {
    /// Address of the server
    #[arg(short, long)]
    addr: SocketAddr,
}

fn main() -> Result<()> {
    // Initialize logger
    log4rs::init_file("client-tui/log4rs.yml", Default::default())
        .context("Unable to initialize logger")?;

    // Parse arguments
    let args = Args::parse();

    let stream = TcpStream::connect(args.addr)?;
    stream.set_nonblocking(true)?;

    if let Err(e) = ClientInterface::new(io::stdout(), stream)?.run() {
        terminal::disable_raw_mode()?;
        log::error!("{e}");
        return Err(e);
    }

    Ok(())
}
