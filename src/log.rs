//! The conversation as a client sees it: one JSON event per line, in the chat
//! directory, next to the transcript.
//!
//! The transcript is what the model gets back on the next turn; this is what a
//! human reads. Both describe the same turn, and the parent is the only writer:
//! it logs the events the worker sends it, plus the message the user sent.

use crate::protocol::Event;
use std::io::Write;
use std::path::{Path, PathBuf};

/// How much of the backlog a client gets when it attaches. Enough to open the
/// conversation in context, and bounded so a long one does not come down the
/// wire whole.
pub const REPLAY: usize = 2000;

pub struct Log {
    path: PathBuf,
}

impl Log {
    pub fn in_dir(dir: &Path) -> Self {
        Self {
            path: dir.join("conversation.jsonl"),
        }
    }

    pub fn events(&self) -> Vec<Event> {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return Vec::new();
        };
        let all: Vec<Event> = text
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        all[all.len().saturating_sub(REPLAY)..].to_vec()
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
