//! One worker process per turn, spawned when the turn starts and gone when it
//! ends.
//!
//! The pool owns the children and the JSONL pipes; a turn is a command in and
//! events out until one of them is terminal. Nothing survives a turn, so the
//! pool is a record of what is running right now and never a pile of idle
//! processes.
//!
//! Cuando la org tiene sandbox, el worker no es un proceso de acá: es uno
//! adentro del sandbox, con su volumen montado, y el mismo protocolo viaja por
//! la API de procesos. El turno cuenta igual —el comando entra y los eventos
//! salen—, solo que del otro lado los archivos son los suyos.

use crate::conversations::Conversation;
use crate::protocol::{Command, Event};
use crate::sandbox::{OnEvent, Sandbox, Turn};
use crate::tensorlake::{SandboxInfo, Tensorlake, MOUNT};
use crate::transport::Session;
use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command as Process, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const WORKER_SLOTS: usize = 64;
static WORKER_PIDS: [AtomicI32; WORKER_SLOTS] = [const { AtomicI32::new(0) }; WORKER_SLOTS];

/// El otro lado del pipe: un proceso de acá o uno del sandbox de la org.
enum Destino {
    Local {
        stdin: Mutex<ChildStdin>,
        child: Mutex<Child>,
    },
    Remoto {
        cliente: Arc<Tensorlake>,
        sandbox: String,
        pid: i64,
    },
}

struct Worker {
    key: String,
    /// El pid de acá, que es el que vigila el reaper. Del otro lado es 0.
    pid: i32,
    destino: Destino,
    events: Mutex<Receiver<Event>>,
}

pub struct Pool {
    workers: Mutex<HashMap<String, Arc<Worker>>>,
    exe: Option<PathBuf>,
}

impl Pool {
    pub fn new(exe: Option<PathBuf>) -> Arc<Pool> {
        Arc::new(Pool {
            workers: Mutex::new(HashMap::new()),
            exe,
        })
    }

    pub fn turn(
        &self,
        session: &Session,
        conversation: &Conversation,
        env: &[(String, String)],
        command: Command,
        on_event: OnEvent,
        sandbox: Option<&SandboxInfo>,
    ) -> Result<Turn, String> {
        let worker = self.spawn(session, conversation, env, sandbox)?;
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
        match &worker.destino {
            Destino::Local { .. } => {
                unsafe { libc::kill(worker.pid, libc::SIGTERM) };
            }
            Destino::Remoto {
                cliente,
                sandbox,
                pid,
            } => {
                let _ = cliente.kill(sandbox, *pid);
            }
        }
        wait(&worker);
    }

    pub fn running(&self, key: &str) -> bool {
        self.workers.lock().unwrap().contains_key(key)
    }

    /// Un proceso nuevo por turno: arranca en milisegundos y rearma su contexto
    /// desde el transcript, así que no hay nada que guardar entre turnos.
    fn spawn(
        &self,
        session: &Session,
        conversation: &Conversation,
        env: &[(String, String)],
        sandbox: Option<&SandboxInfo>,
    ) -> Result<Arc<Worker>, String> {
        match sandbox {
            Some(sandbox) => self.spawn_remoto(session, conversation, env, sandbox),
            None => self.spawn_local(session, conversation, env),
        }
    }

    fn spawn_local(
        &self,
        session: &Session,
        conversation: &Conversation,
        env: &[(String, String)],
    ) -> Result<Arc<Worker>, String> {
        let key = session.key();
        let mut process = Process::new(self.exe()?);
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
        for (name, value) in env {
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
            destino: Destino::Local {
                stdin: Mutex::new(stdin),
                child: Mutex::new(child),
            },
            events: Mutex::new(receiver),
        });
        self.workers.lock().unwrap().insert(key, worker.clone());

        spawn_reader(stdout, sender);
        Ok(worker)
    }

    /// El worker de una org, adentro de su sandbox: el binario sale de su
    /// volumen —publicado una vez por versión— y el proceso arranca con el
    /// volumen montado, así que los archivos que toca son los suyos.
    fn spawn_remoto(
        &self,
        session: &Session,
        conversation: &Conversation,
        env: &[(String, String)],
        listo: &SandboxInfo,
    ) -> Result<Arc<Worker>, String> {
        let key = session.key();
        let sandbox = listo.name.as_str();
        let cliente =
            Arc::new(Tensorlake::from_env().ok_or("esta org necesita TENSORLAKE_API_KEY")?);
        let binario = crate::remote::publicar(&cliente, listo, &self.exe()?)?;
        let mut entorno: BTreeMap<String, String> = env.iter().cloned().collect();
        // El PATH de la imagen del entorno, con los shims de mise: sin esto el
        // modelo no encuentra ni node, ni bun, ni cargo.
        entorno.entry("PATH".into()).or_insert(
            "/root/.local/share/mise/shims:/root/.cargo/bin:/usr/local/bin:/usr/bin:/bin".into(),
        );
        entorno.entry("HOME".into()).or_insert("/root".into());
        let pid = cliente.start(
            sandbox,
            &binario,
            &[
                "worker".to_string(),
                "--chat".to_string(),
                key.clone(),
                "--cwd".to_string(),
                conversation.cwd.display().to_string(),
            ],
            &entorno,
            MOUNT,
        )?;
        let (sender, receiver) = mpsc::channel();
        let worker = Arc::new(Worker {
            key: key.clone(),
            pid: 0,
            destino: Destino::Remoto {
                cliente: cliente.clone(),
                sandbox: sandbox.to_string(),
                pid,
            },
            events: Mutex::new(receiver),
        });
        self.workers.lock().unwrap().insert(key, worker.clone());

        spawn_follower(cliente, sandbox.to_string(), pid, sender);
        Ok(worker)
    }

    fn exe(&self) -> Result<PathBuf, String> {
        match &self.exe {
            Some(exe) => Ok(exe.clone()),
            None => std::env::current_exe().map_err(|e| e.to_string()),
        }
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
        env: &[(String, String)],
        command: Command,
        on_event: OnEvent,
        sandbox: Option<&SandboxInfo>,
    ) -> Result<Turn, String> {
        Pool::turn(self, session, conversation, env, command, on_event, sandbox)
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
        match &self.destino {
            Destino::Local { stdin, .. } => {
                let mut stdin = stdin.lock().unwrap();
                stdin
                    .write_all(line.as_bytes())
                    .map_err(|e| format!("el worker terminó sin responder: {e}"))?;
                stdin
                    .flush()
                    .map_err(|e| format!("el worker terminó sin responder: {e}"))
            }
            Destino::Remoto {
                cliente,
                sandbox,
                pid,
            } => cliente.write_stdin(sandbox, *pid, line.as_bytes()),
        }
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
    match &worker.destino {
        Destino::Local { child, .. } => {
            if let Ok(mut child) = child.lock() {
                let _ = child.wait();
            }
        }
        // Del otro lado el proceso se va con el shutdown; si se quedó colgado
        // no se espera para siempre.
        Destino::Remoto { .. } => {
            let _ = worker
                .events
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5));
        }
    }
}

fn spawn_reader(stdout: ChildStdout, sender: Sender<Event>) {
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else {
                break;
            };
            if !send_line(&line, &sender) {
                break;
            }
        }
    });
}

/// Del otro lado la salida llega por SSE, línea por línea, hasta que el proceso
/// termina.
fn spawn_follower(cliente: Arc<Tensorlake>, sandbox: String, pid: i64, sender: Sender<Event>) {
    std::thread::spawn(move || {
        let _ = cliente.follow(&sandbox, pid, &mut |line| {
            send_line(line, &sender);
        });
    });
}

fn send_line(line: &str, sender: &Sender<Event>) -> bool {
    let Ok(event) = serde_json::from_str::<Event>(line) else {
        return true;
    };
    sender.send(event).is_ok()
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
        let pool = Pool::new(None);
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
        let pool = Pool::new(Some(exe));
        let session = Session::channel("test");
        let conversation = conversation("../workspace");
        for _ in 0..2 {
            match pool
                .turn(
                    &session,
                    &conversation,
                    &[],
                    Command::Resume,
                    &mut |_| {},
                    None,
                )
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
        let pool = Pool::new(Some(exe));
        let session = Session::channel("test");
        let conversation = conversation("../workspace");
        let mut pids = Vec::new();
        for _ in 0..2 {
            match pool
                .turn(
                    &session,
                    &conversation,
                    &[],
                    Command::Resume,
                    &mut |_| {},
                    None,
                )
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
        let pool = Pool::new(Some(exe));
        let session = Session::channel("test");
        let conversation = conversation("../workspace");
        pool.turn(
            &session,
            &conversation,
            &[],
            Command::Resume,
            &mut |_| {},
            None,
        )
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
        let pool = Pool::new(Some(exe));
        let session = Session::channel("test");
        let conversation = conversation("../workspace");
        let mut seen = false;
        pool.turn(
            &session,
            &conversation,
            &[],
            Command::Resume,
            &mut |_| {
                seen = pool.running("test");
            },
            None,
        )
        .unwrap();
        assert!(seen);
        assert!(!pool.running("test"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_worker_that_dies_leaves_no_trace() {
        let dir = scratch("dead");
        let exe = script(&dir, "worker.sh", "exit 0\n");
        let pool = Pool::new(Some(exe));
        let session = Session::channel("test");
        let conversation = conversation("../workspace");
        assert!(pool
            .turn(
                &session,
                &conversation,
                &[],
                Command::Resume,
                &mut |_| {},
                None
            )
            .is_err());
        assert!(pool.workers.lock().unwrap().is_empty());
        assert!(pool
            .turn(
                &session,
                &conversation,
                &[],
                Command::Resume,
                &mut |_| {},
                None
            )
            .is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
mod remoto {
    use super::*;
    use crate::store::Store;
    use crate::workspace::place;
    use crate::Agent;
    use std::path::Path;

    /// Baja el sandbox aunque la prueba falle a mitad: el plan de prueba deja
    /// uno solo por vez.
    struct Guardado(Arc<Tensorlake>, String);

    impl Drop for Guardado {
        fn drop(&mut self) {
            let _ = self.0.terminate(&self.1);
        }
    }

    /// El binario que va al sandbox: el release es el que se publica en serio,
    /// así que la prueba lo prefiere si se lo señalan.
    fn exe() -> PathBuf {
        match crate::env("JIMMY_TEST_EXE") {
            Some(path) => PathBuf::from(path),
            None => std::env::current_exe().unwrap(),
        }
    }

    /// La compuerta del turno adentro: el worker arranca en el sandbox de una
    /// org, escribe su transcript y sus archivos en el volumen, y los eventos
    /// vuelven. Habla con el modelo de verdad y crea recursos, así que corre a
    /// mano:
    ///
    ///     cargo build --release
    ///     heimdall run -p jimmy -c dev -- env JIMMY_TEST_EXE=/tmp/cargo-target/release/jimmy \
    ///       cargo test --bin jimmy -- --ignored el_turno_corre_adentro --nocapture
    #[test]
    #[ignore]
    fn el_turno_corre_adentro_del_sandbox() {
        let base = std::env::temp_dir().join(format!("jimmy-adentro-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("root");
        let workspace = root.join("workspace");
        std::fs::create_dir_all(workspace.join("projects/ken")).unwrap();

        let store = Arc::new(Store::open(&root.join("jimmy.db")).unwrap());
        let (_, org) = store.register("don@ejemplo.com").unwrap();
        store
            .set_machine(&org.id, "tensorlake", "turno-adentro", "jimmy-org")
            .unwrap();

        let mut agent = Agent::new(
            crate::env("AXE_BASE").unwrap_or_else(|| "https://api.deepseek.com".into()),
            crate::env("AXE_MODEL").unwrap_or_else(|| "deepseek-flash".into()),
            crate::env("OPENAI_API_KEY").expect("esta prueba habla con el modelo"),
            crate::env("AXE_CONTEXT_WINDOW").and_then(|w| w.parse().ok()),
            root.clone(),
            workspace.display().to_string(),
            String::new(),
        );
        agent.set_store(store.clone());
        agent.use_worker_exe(exe());
        let agent = agent.at(&place(&root, &workspace, &org));

        let session = Session::channel("adentro-del-sandbox");
        let chat = root.join("chats/adentro-del-sandbox");
        let primero = std::time::Instant::now();
        let respuesta = agent
            .run_task(
                &crate::transport::Null,
                &session,
                "Escribí el archivo hola.txt con la palabra hola en el directorio actual \
                 (con una ruta relativa) y contestá listo.",
                true,
            )
            .expect("el turno adentro del sandbox");
        let frio = primero.elapsed();
        assert!(
            !respuesta.trim().is_empty(),
            "no hubo respuesta: {respuesta:?}"
        );

        let cliente = Arc::new(Tensorlake::from_env().unwrap());
        let sandbox = cliente
            .find("turno-adentro")
            .unwrap()
            .expect("el sandbox quedó de la corrida");
        let _guardado = Guardado(cliente.clone(), sandbox.id.clone());

        // El agente adentro es el mismo de acá, y en los mismos paths: si no,
        // lo que el modelo corre por bash no es el agente.
        let donde = cliente
            .run(
                "turno-adentro",
                MOUNT,
                "command -v jimmy; command -v browse; jimmy skill list | head -3",
                60,
                &mut |_| {},
            )
            .unwrap();
        assert!(donde.contains("/usr/local/bin/jimmy"), "{donde}");
        assert!(donde.contains("/usr/local/bin/browse"), "{donde}");
        assert!(donde.contains("browse"), "el CLI no ve las skills: {donde}");

        // El entorno: lo que el agente usa para trabajar en los proyectos y lo
        // que necesita su herramienta de navegar.
        let entorno = cliente
            .run(
                "turno-adentro",
                MOUNT,
                "for c in node bun cargo go fd jq rg chromium ffmpeg python3; do \
                   command -v $c > /dev/null || echo falta $c; done; \
                 /opt/browse-venv/bin/python -c 'import playwright' && echo playwright ok; \
                 browse goto https://example.com | head -3",
                180,
                &mut |_| {},
            )
            .unwrap();
        assert!(
            !entorno.contains("falta "),
            "el entorno no está completo: {entorno}"
        );
        assert!(entorno.contains("playwright ok"), "{entorno}");
        assert!(
            entorno.to_lowercase().contains("example"),
            "browse no trajo la página: {entorno}"
        );
        let instalado = cliente
            .read_file("turno-adentro", "/usr/local/bin/jimmy")
            .unwrap();
        assert_eq!(
            instalado,
            std::fs::read(exe()).unwrap(),
            "el jimmy del sandbox no es el mismo binario"
        );
        let prompt = cliente
            .read_file(
                "turno-adentro",
                "/usr/local/share/jimmy/prompts/identidad.md",
            )
            .unwrap();
        assert_eq!(
            prompt,
            std::fs::read(Path::new(crate::prompt::BUILTIN).join("identidad.md")).unwrap(),
            "los prompts del sandbox no son los de acá"
        );

        // El segundo turno aprovecha el sandbox despierto y el binario ya
        // publicado: es el tiempo que importa, y de paso comprueba que el
        // transcript del volumen es el que el worker lee para seguir.
        let segundo = std::time::Instant::now();
        let otra = agent
            .run_task(
                &crate::transport::Null,
                &session,
                "¿Qué archivo escribiste recién? Contestá con el nombre y nada más.",
                true,
            )
            .expect("el segundo turno");
        let caliente = segundo.elapsed();
        eprintln!("turno frío: {frio:?} · turno caliente: {caliente:?} · dijo: {otra:?}");
        assert!(otra.contains("hola.txt"), "no siguió el hilo: {otra:?}");

        let transcript = cliente
            .read_file(
                "turno-adentro",
                &format!("{MOUNT}/chats/adentro-del-sandbox/transcript.jsonl"),
            )
            .expect("el transcript está en el volumen");
        assert!(
            String::from_utf8_lossy(&transcript).contains("hola.txt"),
            "el transcript no es de este turno: {}",
            String::from_utf8_lossy(&transcript)
        );
        let escrito = cliente
            .read_file("turno-adentro", &format!("{MOUNT}/workspace/hola.txt"))
            .expect("el archivo que escribió el modelo está en el volumen");
        assert!(
            String::from_utf8_lossy(&escrito).contains("hola"),
            "{escrito:?}"
        );

        assert!(
            !chat.join("transcript.jsonl").exists(),
            "el transcript quedó de este lado"
        );
        assert!(
            !workspace.join("hola.txt").exists(),
            "el archivo del modelo quedó de este lado"
        );

        let _ = std::fs::remove_dir_all(&base);
    }
}
