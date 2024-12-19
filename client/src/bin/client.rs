use std::{
    io::{self, stdout, Write},
    thread,
    time::Duration,
};

use anyhow::{bail, Result};
use crossterm::{
    cursor::MoveTo,
    event::{poll, read, KeyModifiers},
    terminal::{self, Clear, ClearType},
    tty::IsTty,
    QueueableCommand,
};

const RENDER_FPS: u64 = 60;
const FRAME_TIME: Duration = Duration::from_nanos(1_000_000_000 / RENDER_FPS);
const BAR_CHAR: &str = "━";
const CHAT_PROMPT: &[u8; 3] = b" > ";

#[derive(Debug)]
struct ClientInterface {
    size: (u16, u16),
    prompt: String,
    quit: bool,
    chat: Vec<String>,
}

impl ClientInterface {
    fn new() -> Result<Self> {
        let size = terminal::size()?;
        Ok(Self {
            size,
            prompt: String::new(),
            quit: false,
            chat: Vec::new(),
        })
    }

    pub fn width(&self) -> u16 {
        self.size.0
    }

    pub fn height(&self) -> u16 {
        self.size.1
    }

    fn resize(&mut self, size: (u16, u16)) {
        self.size = size
    }

    pub(crate) fn draw<Q>(&self, output: &mut Q) -> Result<()>
    where
        Q: QueueableCommand + io::Write,
    {
        // Cleanup
        output.queue(Clear(ClearType::All))?;
        // Chat
        let start_idx = self.chat.len().saturating_sub(self.height() as usize - 2);
        for (row, line) in self.chat[start_idx..].iter().enumerate() {
            output.queue(MoveTo(0, row as u16))?;
            output.write_all(format!("{row:>3}: ").as_bytes())?;
            output.write_all(line.as_bytes())?;
        }
        // Division
        output.queue(MoveTo(0, self.height() - 2))?;
        output.write_all(BAR_CHAR.repeat(self.width() as usize).as_bytes())?;
        // Prompt symbol
        output.queue(MoveTo(0, self.height() - 1))?;
        output.write_all(CHAT_PROMPT)?;
        // Prompt content
        output.write_all(self.prompt.as_bytes())?;

        Ok(())
    }
}

fn main() -> Result<()> {
    let mut stdout = stdout();
    if !stdout.is_tty() {
        bail!("Standard output is not tty")
    }
    terminal::enable_raw_mode()?;

    let mut ui = ClientInterface::new()?;

    while !ui.quit {
        while poll(Duration::ZERO)? {
            match read()? {
                crossterm::event::Event::Resize(new_width, new_height) => {
                    ui.resize((new_width, new_height))
                }
                crossterm::event::Event::Key(key_event) => match key_event.code {
                    crossterm::event::KeyCode::Char(c) => {
                        if c == 'd' && key_event.modifiers.contains(KeyModifiers::CONTROL) {
                            ui.quit = true;
                            continue;
                        }
                        ui.prompt.push(c);
                    }
                    crossterm::event::KeyCode::Backspace => {
                        let _ = ui.prompt.pop();
                    }
                    crossterm::event::KeyCode::Enter => {
                        if !ui.prompt.is_empty() {
                            ui.chat.push(ui.prompt.clone());
                            ui.prompt.clear();
                        }
                    }
                    crossterm::event::KeyCode::Left => todo!(),
                    crossterm::event::KeyCode::Right => todo!(),
                    crossterm::event::KeyCode::Tab => todo!(),
                    crossterm::event::KeyCode::Delete => todo!(),
                    crossterm::event::KeyCode::Esc => todo!(),
                    _ => {}
                },
                _ => {}
            }
        }

        ui.draw(&mut stdout)?;
        stdout.flush()?;

        thread::sleep(FRAME_TIME);
    }

    terminal::disable_raw_mode()?;

    Ok(())
}
