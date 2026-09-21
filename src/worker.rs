//! One worker process per conversation. It owns the axe session and the
//! transcript in the chat directory, and speaks the JSONL protocol with the
//! parent that spawned it: a prompt or a resume in, events out.
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
use std::sync::mpsc::Sender;
use std::sync::Arc;

pub fn run(args: Vec<String>) -> Result<(), String> {
    let chat = crate::flag(&args, "--chat").ok_or("worker necesita --chat")?;
    let config = Config::from_env()?;
    let mut agent = crate::build_agent(&config)?;
    let session = crate::session_from_key(&chat).ok_or("clave de chat inválida")?;
    if let Some(cwd) = crate::flag(&args, "--cwd") {
        agent.set_cwd(&cwd);
    }
    let pipe = Pipe::new();
    agent.set_pipe(pipe.clone());

    // El turno corre en este hilo, así que alguien tiene que seguir leyendo la
    // entrada: es lo que deja cancelar sin esperar a que termine.
    let cancel = Arc::new(AtomicBool::new(false));
    agent.set_cancel(cancel.clone());
    let (commands, incoming) = std::sync::mpsc::channel();
    read_commands(commands, cancel.clone());

    crate::install_sigterm();
    pipe.emit(&Event::Ready)?;

    for command in incoming {
        pipe.begin_turn();
        cancel.store(false, Ordering::SeqCst);
        let result = match command {
            Command::Cancel => continue,
            Command::Shutdown => break,
            Command::Prompt { text, images } => {
                agent.local_prompt(pipe.as_ref(), &session, &text, images)
            }
            Command::Resume => {
                let dir = agent.conversation(&session).dir;
                agent.local_resume(pipe.as_ref(), &session, &dir)
            }
        };
        if let Err(error) = result {
            if !pipe.answered() {
                pipe.fail(&session, None, &format!("⚠️ {error}"));
            }
        }
    }
    Ok(())
}

/// Una cancelación se atiende acá mismo; el resto va para el hilo del turno.
fn read_commands(commands: Sender<Command>, cancel: Arc<AtomicBool>) {
    std::thread::spawn(move || {
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
            if matches!(command, Command::Cancel) {
                cancel.store(true, Ordering::SeqCst);
                continue;
            }
            if commands.send(command).is_err() {
                break;
            }
        }
    });
}

/// The worker's end of the pipe: every event it tells the parent goes through
/// here, whether it comes from the turn or from the session machinery.
#[derive(Default)]
pub struct Pipe {
    answered: AtomicBool,
}

impl Pipe {
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self::default())
    }

    fn begin_turn(&self) {
        self.answered.store(false, Ordering::SeqCst);
    }

    fn answered(&self) -> bool {
        self.answered.load(Ordering::SeqCst)
    }

    pub fn emit(&self, event: &Event) -> Result<(), String> {
        let mut line = serde_json::to_string(event).map_err(|e| e.to_string())?;
        line.push('\n');
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        out.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
        out.flush().map_err(|e| e.to_string())
    }

    fn terminal(&self, event: Event) {
        self.answered.store(true, Ordering::SeqCst);
        let _ = self.emit(&event);
    }
}

impl Transport for Pipe {
    fn parse_target(&self, key: &str) -> Result<Session, String> {
        crate::session_from_key(key).ok_or_else(|| "clave de chat inválida".into())
    }

    fn progress(&self, _: &Session) -> Option<Msg> {
        None
    }

    fn answer(&self, _: &Session, _: Option<Msg>, markdown: &str) {
        self.terminal(Event::Done {
            text: markdown.to_string(),
        });
    }

    fn note(&self, _: &Session, _: &str) {}

    fn fail(&self, _: &Session, _: Option<Msg>, text: &str) {
        self.terminal(Event::Error {
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
