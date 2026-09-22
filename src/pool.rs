//! One worker process per conversation, spawned on demand and kept alive
//! between turns.
//!
//! The pool owns the children and the JSONL pipes; a turn is a command in and
//! events out until one of them is terminal. A worker that dies is forgotten,
//! so the next turn spawns a fresh one.

use crate::conversations::Conversation;
use crate::protocol::{Command, Event};
use crate::transport::Session;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command as Process, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

const WORKER_SLOTS: usize = 64;
static WORKER_PIDS: [AtomicI32; WORKER_SLOTS] = [const { AtomicI32::new(0) }; WORKER_SLOTS];

pub enum Turn {
    Answer(String),
    Failed(String),
}

/// What the caller wants to do with every event on the way to the end of the
/// turn: jimmy writes them down as the conversation's log.
pub type OnEvent<'a> = &'a mut dyn FnMut(&Event);

struct Worker {
    pid: i32,
    busy: AtomicBool,
    stdin: Mutex<ChildStdin>,
    child: Mutex<Child>,
    events: Mutex<Receiver<Event>>,
}

pub struct Pool {
    workers: Mutex<HashMap<String, Arc<Worker>>>,
    env: Vec<(String, String)>,
    exe: Option<PathBuf>,
}

impl Pool {
    pub fn new(env: Vec<(String, String)>, exe: Option<PathBuf>) -> Arc<Pool> {
        Arc::new(Pool {
            workers: Mutex::new(HashMap::new()),
            env,
            exe,
        })
    }

    pub fn turn(
        self: &Arc<Self>,
        session: &Session,
        conversation: &Conversation,
        command: Command,
        on_event: OnEvent,
    ) -> Result<Turn, String> {
        let worker = self.ensure(session, conversation)?;
        worker.send(&command)?;
        worker.busy.store(true, Ordering::SeqCst);
        let result = self.run_turn(&worker, on_event);
        worker.busy.store(false, Ordering::SeqCst);
        result
    }

    fn run_turn(&self, worker: &Arc<Worker>, on_event: OnEvent) -> Result<Turn, String> {
        loop {
            let event = worker.receive()?;
            if !matches!(event, Event::Ready) {
                on_event(&event);
            }
            match event {
                Event::Ready => continue,
                Event::Done { text } => {
                    worker.busy.store(false, Ordering::SeqCst);
                    return Ok(Turn::Answer(text));
                }
                Event::Error { message } => {
                    worker.busy.store(false, Ordering::SeqCst);
                    return Ok(Turn::Failed(message));
                }
                _ => continue,
            }
        }
    }

    /// Interrumpe el turno de esa conversación, si hay uno corriendo. Va por el
    /// mismo pipe que todo lo demás, así que el worker decide cuándo mirarlo.
    pub fn cancel(&self, key: &str) {
        let worker = self.workers.lock().unwrap().get(key).cloned();
        if let Some(worker) = worker {
            let _ = worker.send(&Command::Cancel);
        }
    }

    /// Baja el worker de esa conversación y espera a que muera, para que no
    /// siga escribiendo en una carpeta que estamos por borrar.
    pub fn kill(&self, key: &str) {
        let worker = self.workers.lock().unwrap().get(key).cloned();
        let Some(worker) = worker else {
            return;
        };
        unsafe { libc::kill(worker.pid, libc::SIGTERM) };
        if let Ok(mut child) = worker.child.lock() {
            let _ = child.wait();
        }
        self.forget(key, worker.pid);
    }

    pub fn running(&self, key: &str) -> bool {
        self.workers
            .lock()
            .unwrap()
            .get(key)
            .is_some_and(|worker| worker.busy.load(Ordering::SeqCst))
    }

    fn ensure(
        self: &Arc<Self>,
        session: &Session,
        conversation: &Conversation,
    ) -> Result<Arc<Worker>, String> {
        let key = session.key();
        let mut workers = self.workers.lock().unwrap();
        if let Some(worker) = workers.get(&key) {
            return Ok(worker.clone());
        }

        let exe = match &self.exe {
            Some(exe) => exe.clone(),
            None => std::env::current_exe().map_err(|e| e.to_string())?,
        };
        let mut process = Process::new(exe);
        process
            .arg("worker")
            .arg("--chat")
            .arg(&key)
            .arg("--cwd")
            .arg(&conversation.cwd);
        process
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        for (name, value) in &self.env {
            process.env(name, value);
        }
        let mut child = process
            .spawn()
            .map_err(|e| format!("no pude lanzar el worker: {e}"))?;
        let stdin = child.stdin.take().ok_or("el worker no tiene stdin")?;
        let stdout = child.stdout.take().ok_or("el worker no tiene stdout")?;
        let pid = child.id() as i32;
        register(pid);
        let (sender, receiver) = mpsc::channel();
        let worker = Arc::new(Worker {
            pid,
            busy: AtomicBool::new(false),
            stdin: Mutex::new(stdin),
            child: Mutex::new(child),
            events: Mutex::new(receiver),
        });
        workers.insert(key.clone(), worker.clone());
        drop(workers);

        spawn_reader(self.clone(), key, pid, stdout, sender);
        Ok(worker)
    }

    fn forget(&self, key: &str, pid: i32) -> Option<Arc<Worker>> {
        let mut workers = self.workers.lock().unwrap();
        if !workers.get(key).is_some_and(|worker| worker.pid == pid) {
            return None;
        }
        let worker = workers.remove(key);
        unregister(pid);
        worker
    }
}

impl Worker {
    fn send(&self, command: &Command) -> Result<(), String> {
        let mut line = serde_json::to_string(command).map_err(|e| e.to_string())?;
        line.push('\n');
        let mut stdin = self.stdin.lock().unwrap();
        stdin
            .write_all(line.as_bytes())
            .map_err(|e| e.to_string())?;
        stdin.flush().map_err(|e| e.to_string())
    }

    fn receive(&self) -> Result<Event, String> {
        self.events
            .lock()
            .unwrap()
            .recv()
            .map_err(|_| "el worker terminó sin responder".to_string())
    }
}

fn spawn_reader(
    pool: Arc<Pool>,
    key: String,
    pid: i32,
    stdout: ChildStdout,
    sender: Sender<Event>,
) {
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else {
                break;
            };
            let Ok(event) = serde_json::from_str::<Event>(&line) else {
                continue;
            };
            if sender.send(event).is_err() {
                break;
            }
        }
        if let Some(worker) = pool.forget(&key, pid) {
            if let Ok(mut child) = worker.child.lock() {
                let _ = child.wait();
            }
        }
    });
}

fn register(pid: i32) {
    for slot in &WORKER_PIDS {
        if slot
            .compare_exchange(0, pid, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            return;
        }
    }
}

fn unregister(pid: i32) {
    for slot in &WORKER_PIDS {
        let _ = slot.compare_exchange(pid, 0, Ordering::AcqRel, Ordering::Relaxed);
    }
}

/// Signal every worker to go. Called from the signal handler, so it only
/// touches the lock-free pid array.
pub fn kill_all() {
    for slot in &WORKER_PIDS {
        let pid = slot.swap(0, Ordering::AcqRel);
        if pid > 0 {
            unsafe { libc::kill(pid, libc::SIGTERM) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Command;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn conversation(cwd: &str) -> Conversation {
        Conversation {
            key: "test".into(),
            dir: PathBuf::from("."),
            cwd: PathBuf::from(cwd),
            project: "general".into(),
            title: None,
            read_only: true,
        }
    }

    /// El olvido de un worker muerto pasa en el hilo que lee su salida, así que
    /// hay que esperarlo.
    fn eventually(mut ready: impl FnMut() -> bool) -> bool {
        for _ in 0..200 {
            if ready() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        ready()
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jimmy-pool-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_turn_gets_what_the_worker_answers() {
        let dir = scratch("answer");
        let exe = script(
            &dir,
            "worker.sh",
            "echo '{\"event\":\"ready\"}'
while read -r line; do
  case \"$line\" in *shutdown*) exit 0 ;; esac
  printf '{\"event\":\"done\",\"text\":\"%s\"}\n' \"$*\"
done
",
        );
        let pool = Pool::new(Vec::new(), Some(exe));
        let session = Session::channel("test");
        let conversation = conversation("../workspace");
        for _ in 0..2 {
            match pool
                .turn(&session, &conversation, Command::Resume, &mut |_| {})
                .unwrap()
            {
                Turn::Answer(text) => {
                    assert!(text.contains("--chat test"), "{text}");
                    assert!(text.contains("--cwd ../workspace"), "{text}");
                }
                Turn::Failed(message) => panic!("esperaba respuesta, no {message}"),
            }
        }
        assert_eq!(pool.workers.lock().unwrap().len(), 1);
        pool.kill("test");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_worker_that_dies_is_forgotten() {
        let dir = scratch("dead");
        let exe = script(&dir, "worker.sh", "exit 0\n");
        let pool = Pool::new(Vec::new(), Some(exe));
        let session = Session::channel("test");
        let conversation = conversation("../workspace");
        assert!(pool
            .turn(&session, &conversation, Command::Resume, &mut |_| {})
            .is_err());
        assert!(eventually(|| pool.workers.lock().unwrap().is_empty()));
        assert!(pool
            .turn(&session, &conversation, Command::Resume, &mut |_| {})
            .is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
