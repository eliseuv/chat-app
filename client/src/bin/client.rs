use core::str;
use std::{
    io::{self, Read, Write},
    net::TcpStream,
    thread,
    time::Duration,
};

use anyhow::{bail, Result};
use crossterm::{cursor, event, terminal, tty::IsTty, QueueableCommand};

// TODO: Wrap lines
// TODO: Persistent prompt content on resize

const RENDER_FPS: u64 = 60;
const FRAME_TIME: Duration = Duration::from_nanos(1_000_000_000 / RENDER_FPS);
const BUFFER_SIZE: usize = 64;

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

#[derive(Debug)]
struct Prompt {
    content: String,
    is_full: bool,
    content_max_width: u16,
}

impl Prompt {
    fn new(width: u16) -> Self {
        Self {
            content: String::new(),
            is_full: false,
            content_max_width: width - 3,
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
        self.content_max_width = width - 3;
        self.content.truncate(self.content_max_width as usize);
    }

    fn clear(&mut self) {
        self.content.clear()
    }

    fn push(&mut self, ch: char) {
        if self.content.len() < self.content_max_width as usize {
            self.content.push(ch);
        } else {
            self.is_full = true;
        }
    }

    fn pop(&mut self) -> Option<char> {
        let ch = self.content.pop();
        if self.content.len() < self.content_max_width as usize {
            self.is_full = false;
        }
        ch
    }

    fn push_str(&mut self, string: &str) {
        self.content.push_str(string)
    }
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
        Ok(self.output.flush()?)
    }

    fn write_on_center(&mut self, text: &str) -> Result<()> {
        self.output.queue(cursor::MoveTo(
            (self.width - text.len() as u16) / 2,
            self.height / 2,
        ))?;
        let _ = self.output.write(text.as_bytes())?;
        Ok(())
    }

    fn draw_prompt(&mut self) -> Result<()> {
        self.output.queue(cursor::MoveTo(0, self.height - 2))?;
        let _ = self
            .output
            .write("━".repeat(self.width as usize).as_bytes())?;
        self.output.queue(cursor::MoveTo(0, self.height - 1))?;
        let _ = self.output.write(b" > ")?;
        let _ = self.output.write(self.prompt.content.as_bytes())?;
        Ok(())
    }

    fn draw_cover(&mut self) -> Result<()> {
        self.output
            .queue(terminal::Clear(terminal::ClearType::All))?;
        self.write_on_center("chat app")?;
        self.flush()?;
        Ok(())
    }

    fn draw_main(&mut self) -> Result<()> {
        // Cleanup
        self.output
            .queue(terminal::Clear(terminal::ClearType::All))?
            .queue(cursor::Show)?;
        // Chat
        for (row, line) in self
            .chat
            .iter()
            .skip(self.chat.len().saturating_sub(self.height as usize - 2))
            .enumerate()
        {
            self.output.queue(cursor::MoveTo(0, row as u16))?;
            let bytes = line.as_bytes();
            let _ = self
                .output
                .write(bytes.get(0..self.width as usize).unwrap_or(bytes))?;
        }
        // Prompt
        self.draw_prompt()?;

        self.flush()?;
        Ok(())
    }

    fn handle_event(&mut self) -> Result<()> {
        let new_event = event::read()?;
        log::debug!("Handling event: {new_event:?}");

        match new_event {
            crossterm::event::Event::Resize(width, height) => {
                self.resize(width, height);
                self.draw_main()?;
            }
            crossterm::event::Event::Paste(data) => {
                self.prompt.push_str(&data);
            }
            crossterm::event::Event::Key(key_event) => match key_event.code {
                crossterm::event::KeyCode::Char(c) => {
                    if c == 'd' && key_event.modifiers.contains(event::KeyModifiers::CONTROL) {
                        self.quit = true;
                        return Ok(());
                    }
                    if !self.prompt.is_full() {
                        self.prompt.push(c);
                    }
                }
                crossterm::event::KeyCode::Backspace => {
                    let _ = self.prompt.pop();
                }
                crossterm::event::KeyCode::Enter => {
                    if !self.prompt.is_empty() {
                        let _ = self.stream.write(self.prompt.text().as_bytes());
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
        let text = str::from_utf8(&self.buffer[0..n])?;
        self.chat.push("anon: ".to_string() + text);
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
            thread::sleep(FRAME_TIME);
        }

        terminal::disable_raw_mode()?;
        Ok(())
    }
}

fn main() -> Result<()> {
    env_logger::init();

    let stream = TcpStream::connect("127.0.0.1:6969")?;
    stream.set_nonblocking(true)?;

    if let Err(err) = ClientInterface::new(io::stdout(), stream)?.run() {
        terminal::disable_raw_mode()?;
        log::error!("{err}");
        return Err(err);
    }

    Ok(())
}
