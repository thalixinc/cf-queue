//! Structured error type + exit-code mapping (AXI convention: 0/1/2).

use std::fmt;

#[derive(Debug)]
pub struct QueueError {
    pub message: String,
    pub code: &'static str,
    pub suggestions: Vec<String>,
}

impl QueueError {
    pub fn operational(message: impl Into<String>, code: &'static str) -> Self {
        QueueError {
            message: message.into(),
            code,
            suggestions: Vec::new(),
        }
    }

    pub fn usage(message: impl Into<String>) -> Self {
        QueueError {
            message: message.into(),
            code: "VALIDATION_ERROR",
            suggestions: Vec::new(),
        }
    }

    pub fn with_suggestions(mut self, suggestions: Vec<String>) -> Self {
        self.suggestions = suggestions;
        self
    }

    pub fn exit_code(&self) -> i32 {
        if self.code == "VALIDATION_ERROR" {
            2
        } else {
            1
        }
    }
}

impl fmt::Display for QueueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for QueueError {}

pub type Result<T> = std::result::Result<T, QueueError>;
