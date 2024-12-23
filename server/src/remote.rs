use std::fmt::Display;

use chrono::Utc;
use serde::{Deserialize, Serialize};

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
