use core::str;
use std::{
    env::{self},
    io::{self, Read, Write},
    net::TcpStream,
    thread,
    time::Duration,
};

use anyhow::{bail, Context, Result};
use crossterm::{
    cursor::MoveTo,
    event::{self, Event, KeyCode, KeyModifiers},
    style::Print,
    terminal::{self, Clear, ClearType},
    tty::IsTty,
    QueueableCommand,
};

// TODO: Wrap lines
// TODO: Persistent prompt content on resize

const BUFFER_SIZE: usize = 64;

#[derive(Debug)]
struct Rect {
    x: u16,
    y: u16,
    w: u16,
    h: u16,
}

#[derive(Debug)]
enum State {
    Default,
    Quit,
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

#[derive(Debug)]
struct ClientInterface<T>
where
    T: io::Write + QueueableCommand + IsTty,
{
    output: T,
    width: u16,
    height: u16,
    prompt: Prompt,
    quit: bool,
    chat: Vec<String>,
    buffer: [u8; BUFFER_SIZE],
    stream: TcpStream,
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
            quit: false,
            chat: Vec::new(),
            buffer: [0; BUFFER_SIZE],
            stream,
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

    fn write_on_center(&mut self, text: &str) -> Result<&mut T> {
        self.output
            .queue(MoveTo(
                (self.width - text.len() as u16) / 2,
                self.height / 2,
            ))?
            .queue(Print(text))
            .context("Unable to on the center of the screen")
    }

    fn draw_prompt(&mut self) -> Result<&mut T> {
        let bar = "━".repeat(self.width as usize);
        self.output
            .queue(MoveTo(0, self.height - 2))?
            .queue(Print(bar))?
            .queue(MoveTo(0, self.height - 1))?
            .queue(Print(" > "))?
            .queue(Print(self.prompt.text()))
            .context("Unable to draw prompt")
    }

    fn draw_cover(&mut self) -> Result<()> {
        self.output.queue(Clear(ClearType::All))?;
        self.write_on_center("chat app")?;
        self.flush()
    }

    fn draw_chat(&mut self, rect: Rect) -> Result<()> {
        todo!()
    }

    fn draw_main(&mut self) -> Result<()> {
        // Cleanup
        self.output.queue(Clear(ClearType::All))?;
        // Chat
        for (row, line) in self
            .chat
            .iter()
            .skip(self.chat.len().saturating_sub(self.height as usize - 2))
            .enumerate()
        {
            self.output
                .queue(MoveTo(0, row as u16))?
                .queue(Print(line.get(0..self.width as usize).unwrap_or(line)))?;
        }
        // Prompt
        self.draw_prompt()?;

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
                        self.quit = true;
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
                            Err(err) => log::error!("Unable to send data: {err}"),
                            Ok(n) => log::info!("Successfully sent {n} bytes"),
                        }
                        self.chat.push("you: ".to_string() + self.prompt.text());
                        self.prompt.clear();
                    }
                }
                _ => {}
            },
            _ => {}
        }
        Ok(())
    }

    fn read_stream(&mut self) -> Result<()> {
        let n = self.stream.read(&mut self.buffer)?;
        if n > 0 {
            let text = str::from_utf8(&self.buffer[0..n])?;
            self.chat.push(text.to_string());
        } else {
            self.quit = true;
        }
        Ok(())
    }

    fn run(&mut self) -> Result<()> {
        terminal::enable_raw_mode()?;

        self.draw_cover()?;

        // Main loop
        while !self.quit {
            // Poll for new event
            while event::poll(Duration::ZERO)? {
                if let Err(err) = self.handle_event() {
                    log::error!("Error handling event: {err}");
                }
            }

            if let Err(err) = self.read_stream() {
                log::error!("Error reading from stream: {err}");
            }

            self.draw_main()?;

            // 60 FPS
            thread::sleep(Duration::from_nanos(1_000_000_000 / 60));
        }

        terminal::disable_raw_mode()?;
        Ok(())
    }
}

fn main() -> Result<()> {
    env_logger::init();

    let mut args = env::args();
    let _program = args.next().expect("program name");
    let addr = args.next().expect("server address address");

    let stream = TcpStream::connect(format!("{addr}:6969"))?;
    stream.set_nonblocking(true)?;

    if let Err(err) = ClientInterface::new(io::stdout(), stream)?.run() {
        terminal::disable_raw_mode()?;
        log::error!("{err}");
        return Err(err);
    }

    Ok(())
}
