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
        /// Quién lo escribió, para que el modelo sepa a quién le contesta.
        #[serde(default)]
        author: String,
    },
    Resume,
    /// Compacta el contexto ahora, sin esperar al umbral.
    Compact,
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
    /// Quién está conectado, mire lo que mire. También lo dice el bus.
    Online {
        users: Vec<String>,
    },
    /// Alguien está escribiendo en esta conversación. No se guarda.
    Typing {
        user: String,
    },
    /// Alguien cortó el turno que estaba corriendo.
    Stopped {
        author: String,
    },
    /// What the user sent. The parent writes this one, nobody else.
    User {
        text: String,
        #[serde(default)]
        author: String,
        /// Los adjuntos que subió, como nombres de archivo en el `uploads/` de
        /// la conversación. El contenido no va acá: lo sirve `/api/file`.
        #[serde(default)]
        images: Vec<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_author_is_optional_in_the_log() {
        let old: Event = serde_json::from_str(r#"{"event":"user","text":"hola"}"#).unwrap();
        let Event::User { author, .. } = old else {
            panic!("esperaba un mensaje")
        };
        assert!(author.is_empty());
        let new: Event =
            serde_json::from_str(r#"{"event":"user","text":"hola","author":"berti"}"#).unwrap();
        let Event::User { author, .. } = new else {
            panic!("esperaba un mensaje")
        };
        assert_eq!(author, "berti");
    }
}
