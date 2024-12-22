use chrono::Utc;
use serde::{Deserialize, Serialize};

/// Message to be sent to remote client
#[derive(Debug, Serialize, Deserialize)]
pub struct Message {
    timestamp: i64,
    author: String,
    text: String,
}

impl Message {
    pub fn new(author: String, text: String) -> Self {
        Self {
            timestamp: Utc::now().timestamp(),
            author,
            text,
        }
    }
}
