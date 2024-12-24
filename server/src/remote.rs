use std::{fmt::Display, io::Write};

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

/// Size in bytes of the buffer to store serialized data
pub const BUFFER_SIZE: usize = 64 * 1024; // 64kb

/// Message to be sent to remote client
#[derive(Debug, Serialize, Deserialize)]
pub struct Message {
    pub timestamp: i64,
    pub author: Author,
    pub text: String,
}

impl Message {
    pub fn new(author: Author, text: String) -> Self {
        Self {
            timestamp: Utc::now().timestamp(),
            author,
            text,
        }
    }

    pub fn write_to<W>(&self, writer: W) -> Result<()>
    where
        W: Write,
    {
        ciborium::into_writer(self, writer).context("Unable to serialize message")
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Author {
    Server,
    Client(usize),
}

impl Author {
    pub fn id(&self) -> usize {
        match self {
            Author::Server => 0,
            Author::Client(id) => *id,
        }
    }
}

impl Display for Author {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let repr = match self {
            Author::Server => "Server".to_owned(),
            Author::Client(id) => format!("User {id}"),
        };
        write!(f, "{repr}")
    }
}
