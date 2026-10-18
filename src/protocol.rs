use axe::Image;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    Prompt {
        text: String,
        #[serde(default)]
        images: Vec<Image>,
        #[serde(default)]
        author: String,
    },
    Resume,
    Compact,
    Cancel,
    Shutdown,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Ready,
    Presence {
        users: Vec<String>,
    },
    Online {
        users: Vec<String>,
    },
    Typing {
        user: String,
    },
    Stopped {
        author: String,
    },
    User {
        text: String,
        #[serde(default)]
        author: String,
        #[serde(default)]
        images: Vec<String>,
    },
    Image {
        name: String,
        #[serde(default)]
        caption: String,
    },
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
            serde_json::from_str(r#"{"event":"user","text":"hola","author":"bob"}"#).unwrap();
        let Event::User { author, .. } = new else {
            panic!("esperaba un mensaje")
        };
        assert_eq!(author, "bob");
    }
}
