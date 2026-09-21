//! One worker process per conversation. It owns the axe session and the
//! transcript in the chat directory, and speaks the JSONL protocol with the
//! parent that spawned it.
//!
//! The turn logic is the same code the parent used to run in process; what
//! changes is that it now happens in a process of its own, with its own
//! working directory and its own children. The parent talks to the chat, the
//! worker talks to the model.

use crate::protocol::{Command, Event};
use crate::transport::{Msg, Session, Transport};
use crate::Config;
use std::io::{BufRead, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

pub fn run(args: Vec<String>) -> Result<(), String> {
    let chat = crate::flag(&args, "--chat").ok_or("worker necesita --chat")?;
    let config = Config::from_env()?;
    let agent = crate::build_agent(&config)?;
    let session = crate::session_from_key(&chat).ok_or("clave de chat inválida")?;
    let out = Out::default();

    crate::install_sigterm();
    out.emit(&Event::Ready)?;

    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else {
            break;
        };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(command) = serde_json::from_str::<Command>(&line) else {
            continue;
        };
        out.reset();
        let result = match command {
            Command::Shutdown => break,
            Command::Prompt { text, images } => agent.local_prompt(&out, &session, &text, images),
            Command::Resume => {
                let dir = agent.chat_dir(&session);
                agent.local_resume(&out, &session, &dir)
            }
        };
        if let Err(error) = result {
            if !out.emitted() {
                out.fail(&session, None, &format!("⚠️ {error}"));
            }
        }
    }
    Ok(())
}

#[derive(Default)]
struct Out {
    emitted: AtomicBool,
}

impl Out {
    fn reset(&self) {
        self.emitted.store(false, Ordering::SeqCst);
    }

    fn emitted(&self) -> bool {
        self.emitted.load(Ordering::SeqCst)
    }

    fn emit(&self, event: &Event) -> Result<(), String> {
        let mut line = serde_json::to_string(event).map_err(|e| e.to_string())?;
        line.push('\n');
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        out.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
        out.flush().map_err(|e| e.to_string())
    }

    fn terminal(&self, event: Event) {
        self.emitted.store(true, Ordering::SeqCst);
        let _ = self.emit(&event);
    }
}

impl Transport for Out {
    fn parse_target(&self, key: &str) -> Result<Session, String> {
        crate::session_from_key(key).ok_or_else(|| "clave de chat inválida".into())
    }

    fn progress(&self, _: &Session) -> Option<Msg> {
        None
    }

    fn answer(&self, _: &Session, _: Option<Msg>, markdown: &str) {
        self.terminal(Event::Answer {
            text: markdown.to_string(),
        });
    }

    fn note(&self, _: &Session, _: &str) {}

    fn fail(&self, _: &Session, _: Option<Msg>, text: &str) {
        self.terminal(Event::Failed {
            message: text.to_string(),
        });
    }

    fn download(&self, _: &str) -> Result<(String, Vec<u8>), String> {
        Err("el worker no baja archivos".into())
    }

    fn send_media(&self, _: &Session, _: &Path, _: Option<&str>) -> Result<Msg, String> {
        Err("el worker no manda archivos".into())
    }
}
