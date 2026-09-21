//! JSONL protocol between jimmy and the worker processes it spawns.

use axe::Image;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "lowercase")]
pub enum Command {
    Prompt {
        text: String,
        #[serde(default)]
        images: Vec<Image>,
    },
    Resume,
    Shutdown,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "event", rename_all = "lowercase")]
pub enum Event {
    Ready,
    Answer { text: String },
    Failed { message: String },
}
