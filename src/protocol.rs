//! JSONL protocol between jimmy and the worker processes it spawns.

use axe::Image;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    Prompt {
        text: String,
        #[serde(default)]
        images: Vec<Image>,
    },
    Resume,
    /// Interrumpe el turno que esté corriendo, si hay alguno.
    Cancel,
    Shutdown,
}

/// What the worker tells the parent. It is also what the parent writes down as
/// the conversation's log, so a client can replay the whole thing.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// The worker is up and reading commands.
    Ready,
    /// Quién está mirando esta conversación. Lo dice el bus, no el worker.
    Presence {
        users: Vec<String>,
    },
    /// What the user sent. The parent writes this one, nobody else.
    User {
        text: String,
    },
    /// Texto que va apareciendo mientras el modelo escribe. No se guarda: el
    /// mensaje terminado es el que queda en el log.
    Delta {
        text: String,
    },
    Assistant {
        text: String,
    },
    ToolStart {
        id: String,
        name: String,
        args: String,
    },
    ToolDelta {
        id: String,
        text: String,
    },
    ToolResult {
        id: String,
        text: String,
        ms: u64,
        failed: bool,
    },
    /// The turn ended with this answer for the chat.
    Done {
        text: String,
    },
    Error {
        message: String,
    },
}
