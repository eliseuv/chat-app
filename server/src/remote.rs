use chrono::Utc;
use serde::{Deserialize, Serialize};

/// Message to be sent to remote client
#[derive(Debug, Serialize, Deserialize)]
pub struct Message {
    pub timestamp: i64,
    pub author: String,
    pub text: String,
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
