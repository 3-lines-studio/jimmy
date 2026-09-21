//! The conversation as a client sees it: one JSON event per line, in the chat
//! directory, next to the transcript.
//!
//! The transcript is what the model gets back on the next turn; this is what a
//! human reads. Both describe the same turn, and the parent is the only writer:
//! it logs the events the worker sends it, plus the message the user sent.

use crate::protocol::Event;
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct Log {
    path: PathBuf,
}

impl Log {
    pub fn in_dir(dir: &Path) -> Self {
        Self {
            path: dir.join("conversation.jsonl"),
        }
    }

    pub fn append(&self, event: &Event) {
        let Ok(mut line) = serde_json::to_string(event) else {
            return;
        };
        line.push('\n');
        let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        else {
            return;
        };
        let _ = file.write_all(line.as_bytes());
    }
}
