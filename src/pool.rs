//! One worker process per turn, spawned when the turn starts and gone when it
//! ends.
//!
//! The pool owns the children and the JSONL pipes; a turn is a command in and
//! events out until one of them is terminal. Nothing survives a turn, so the
//! pool is a record of what is running right now and never a pile of idle
//! processes.

use crate::conversations::Conversation;
use crate::protocol::{Command, Event};
use crate::sandbox::{OnEvent, Sandbox, Turn};
use crate::transport::Session;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command as Process, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

const WORKER_SLOTS: usize = 64;
static WORKER_PIDS: [AtomicI32; WORKER_SLOTS] = [const { AtomicI32::new(0) }; WORKER_SLOTS];

struct Worker {
    key: String,
    pid: i32,
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
        &self,
        session: &Session,
        conversation: &Conversation,
        command: Command,
        on_event: OnEvent,
    ) -> Result<Turn, String> {
        let worker = self.spawn(session, conversation)?;
        let result = worker
            .send(&command)
            .and_then(|_| self.pump(&worker, on_event));
        self.retire(&worker);
        result
    }

    fn pump(&self, worker: &Arc<Worker>, on_event: OnEvent) -> Result<Turn, String> {
        loop {
            let event = worker.receive()?;
            if !matches!(event, Event::Ready) {
                on_event(&event);
            }
            match event {
                Event::Ready => continue,
                Event::Done { text } => return Ok(Turn::Answer(text)),
                Event::Error { message } => return Ok(Turn::Failed(message)),
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

    /// Baja el worker de esa conversación a la fuerza y espera a que muera,
    /// para que no siga escribiendo en una carpeta que estamos por borrar.
    pub fn kill(&self, key: &str) {
        let worker = self.workers.lock().unwrap().get(key).cloned();
        let Some(worker) = worker else {
            return;
        };
        if !self.forget(&worker) {
            return;
        }
        unsafe { libc::kill(worker.pid, libc::SIGTERM) };
        wait(&worker);
    }

    pub fn running(&self, key: &str) -> bool {
        self.workers.lock().unwrap().contains_key(key)
    }

    /// Un proceso nuevo por turno: arranca en milisegundos y rearma su contexto
    /// desde el transcript, así que no hay nada que guardar entre turnos.
    fn spawn(&self, session: &Session, conversation: &Conversation) -> Result<Arc<Worker>, String> {
        let key = session.key();
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
            key: key.clone(),
            pid,
            stdin: Mutex::new(stdin),
            child: Mutex::new(child),
            events: Mutex::new(receiver),
        });
        self.workers.lock().unwrap().insert(key, worker.clone());

        spawn_reader(stdout, sender);
        Ok(worker)
    }

    /// Lo que terminó su turno se saca del registro y se le pide que se vaya:
    /// muere por su cuenta, con el marcador del turno ya cerrado. El que ya se
    /// había ido antes no se toca dos veces.
    fn retire(&self, worker: &Arc<Worker>) {
        if !self.forget(worker) {
            return;
        }
        let _ = worker.send(&Command::Shutdown);
        wait(worker);
    }

    fn forget(&self, worker: &Arc<Worker>) -> bool {
        let mut workers = self.workers.lock().unwrap();
        if !workers
            .get(&worker.key)
            .is_some_and(|it| it.pid == worker.pid)
        {
            return false;
        }
        workers.remove(&worker.key);
        unregister(worker.pid);
        true
    }
}

impl Sandbox for Pool {
    /// En local el sandbox es este proceso: no hay nada que preparar.
    fn ensure(&self, _org: &str) -> Result<(), String> {
        Ok(())
    }

    fn turn(
        &self,
        session: &Session,
        conversation: &Conversation,
        command: Command,
        on_event: OnEvent,
    ) -> Result<Turn, String> {
        Pool::turn(self, session, conversation, command, on_event)
    }

    fn cancel(&self, key: &str) {
        Pool::cancel(self, key);
    }

    fn kill(&self, key: &str) {
        Pool::kill(self, key);
    }

    fn running(&self, key: &str) -> bool {
        Pool::running(self, key)
    }

    /// Nada que dormir: los procesos viven lo que dura el turno.
    fn suspend(&self, _org: &str) -> Result<(), String> {
        Ok(())
    }

    /// Nada que bajar: cuando el proceso muere no queda nada suyo.
    fn destroy(&self, _org: &str) -> Result<(), String> {
        Ok(())
    }
}

impl Worker {
    fn send(&self, command: &Command) -> Result<(), String> {
        let mut line = serde_json::to_string(command).map_err(|e| e.to_string())?;
        line.push('\n');
        let mut stdin = self.stdin.lock().unwrap();
        stdin
            .write_all(line.as_bytes())
            .map_err(|e| format!("el worker terminó sin responder: {e}"))?;
        stdin
            .flush()
            .map_err(|e| format!("el worker terminó sin responder: {e}"))
    }

    fn receive(&self) -> Result<Event, String> {
        self.events
            .lock()
            .unwrap()
            .recv()
            .map_err(|_| "el worker terminó sin responder".to_string())
    }
}

fn wait(worker: &Worker) {
    if let Ok(mut child) = worker.child.lock() {
        let _ = child.wait();
    }
}

fn spawn_reader(stdout: ChildStdout, sender: Sender<Event>) {
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

/// Si hay algún turno en curso: la tabla la escriben los que spawnean, así que
/// también la puede leer el vigilante de la memoria.
pub fn busy() -> bool {
    WORKER_PIDS
        .iter()
        .any(|slot| slot.load(Ordering::Acquire) != 0)
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
    use std::path::Path;

    /// El archivo lo escribe un proceso hijo: si lo escribiera éste, el fork de
    /// cualquier otro test heredaría su descriptor y el exec daría «Text file
    /// busy».
    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        let mut writer = Process::new("sh")
            .arg("-c")
            .arg(format!("cat > {p} && chmod 0755 {p}", p = path.display()))
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        writer
            .stdin
            .as_mut()
            .unwrap()
            .write_all(format!("#!/bin/sh\n{body}").as_bytes())
            .unwrap();
        assert!(writer.wait().unwrap().success());
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

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jimmy-pool-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn en_local_no_hay_nada_que_preparar_ni_dormir() {
        let pool = Pool::new(Vec::new(), None);
        for org in ["una-org", ""] {
            assert!(pool.ensure(org).is_ok(), "preparar no hace nada");
            assert!(pool.suspend(org).is_ok(), "dormir no hace nada");
            assert!(pool.destroy(org).is_ok(), "bajar no hace nada");
        }
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
        assert!(pool.workers.lock().unwrap().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn every_turn_gets_its_own_worker() {
        let dir = scratch("fresh");
        let exe = script(
            &dir,
            "worker.sh",
            "echo '{\"event\":\"ready\"}'
while read -r line; do
  case \"$line\" in *shutdown*) exit 0 ;; esac
  printf '{\"event\":\"done\",\"text\":\"%s\"}\\n' \"$$\"
done
",
        );
        let pool = Pool::new(Vec::new(), Some(exe));
        let session = Session::channel("test");
        let conversation = conversation("../workspace");
        let mut pids = Vec::new();
        for _ in 0..2 {
            match pool
                .turn(&session, &conversation, Command::Resume, &mut |_| {})
                .unwrap()
            {
                Turn::Answer(text) => pids.push(text),
                Turn::Failed(message) => panic!("esperaba respuesta, no {message}"),
            }
        }
        assert_ne!(pids[0], pids[1]);
        assert!(pool.workers.lock().unwrap().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Al worker se le pide que se vaya en vez de matarlo: así alcanza a cerrar
    /// su marcador de turno antes de morir.
    #[test]
    fn a_turn_ends_by_asking_the_worker_to_leave() {
        let dir = scratch("leave");
        let left = dir.join("left");
        let exe = script(
            &dir,
            "worker.sh",
            &format!(
                "echo '{{\"event\":\"ready\"}}'
while read -r line; do
  case \"$line\" in *shutdown*) echo chau > {}; exit 0 ;; esac
  printf '{{\"event\":\"done\",\"text\":\"listo\"}}\\n'
done
",
                left.display()
            ),
        );
        let pool = Pool::new(Vec::new(), Some(exe));
        let session = Session::channel("test");
        let conversation = conversation("../workspace");
        pool.turn(&session, &conversation, Command::Resume, &mut |_| {})
            .unwrap();
        assert!(left.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_worker_is_running_only_during_its_turn() {
        let dir = scratch("alive");
        let exe = script(
            &dir,
            "worker.sh",
            "echo '{\"event\":\"ready\"}'
while read -r line; do
  case \"$line\" in *shutdown*) exit 0 ;; esac
  printf '{\"event\":\"tool_start\",\"id\":\"1\",\"name\":\"bash\",\"args\":\"{}\"}\\n'
  printf '{\"event\":\"done\",\"text\":\"listo\"}\\n'
done
",
        );
        let pool = Pool::new(Vec::new(), Some(exe));
        let session = Session::channel("test");
        let conversation = conversation("../workspace");
        let mut seen = false;
        pool.turn(&session, &conversation, Command::Resume, &mut |_| {
            seen = pool.running("test");
        })
        .unwrap();
        assert!(seen);
        assert!(!pool.running("test"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_worker_that_dies_leaves_no_trace() {
        let dir = scratch("dead");
        let exe = script(&dir, "worker.sh", "exit 0\n");
        let pool = Pool::new(Vec::new(), Some(exe));
        let session = Session::channel("test");
        let conversation = conversation("../workspace");
        assert!(pool
            .turn(&session, &conversation, Command::Resume, &mut |_| {})
            .is_err());
        assert!(pool.workers.lock().unwrap().is_empty());
        assert!(pool
            .turn(&session, &conversation, Command::Resume, &mut |_| {})
            .is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
