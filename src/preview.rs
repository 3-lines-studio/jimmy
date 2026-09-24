//! Previews: a project running in a child process of this one, served under
//! /preview/<name>/ by the web frontend.
//!
//! The registry lives in memory because the owner is this process: a preview
//! that answered an order over a socket is never an orphan, which is what keeps
//! the reaper's hands off it. The orders come in over a unix socket, so the CLI
//! is a thin client and the process that spawns is always the same one.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const PREFIX: &str = "/preview/";
const SOCKET: &str = "state/preview.sock";
const LOGS: &str = "state/previews";
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_HEAD: usize = 32 * 1024;

#[derive(Serialize, Deserialize, Default)]
pub struct Order {
    pub op: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Answer {
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub previews: Vec<Summary>,
    #[serde(default)]
    pub output: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Summary {
    pub name: String,
    pub port: u16,
    pub pid: u32,
    pub seconds: u64,
}

struct Running {
    port: u16,
    child: Child,
    started: Instant,
}

pub struct Previews {
    workspace: PathBuf,
    running: Mutex<HashMap<String, Running>>,
}

impl Previews {
    pub fn new(workspace: &Path) -> Arc<Previews> {
        Arc::new(Previews {
            workspace: workspace.to_path_buf(),
            running: Mutex::new(HashMap::new()),
        })
    }

    /// El puerto de un preview vivo, o `None` si no está o se murió.
    pub fn port(&self, name: &str) -> Option<u16> {
        let mut running = self.running.lock().ok()?;
        let entry = running.get_mut(name)?;
        match entry.child.try_wait() {
            Ok(None) => Some(entry.port),
            _ => {
                running.remove(name);
                None
            }
        }
    }

    /// El final del log del preview, para saber por qué no arrancó.
    pub fn output(&self, name: &str) -> String {
        let path = self.workspace.join(LOGS).join(format!("{name}.log"));
        let Ok(text) = std::fs::read_to_string(path) else {
            return String::new();
        };
        let lines: Vec<&str> = text.lines().collect();
        let tail = lines[lines.len().saturating_sub(20)..].join("\n");
        tail.trim().to_string()
    }

    fn start(&self, order: &Order) -> Result<Summary, String> {
        let name = valid_name(&order.name)?;
        let command = order
            .command
            .as_deref()
            .map(str::trim)
            .filter(|command| !command.is_empty())
            .ok_or("falta el comando")?;
        let cwd = match &order.cwd {
            Some(cwd) => PathBuf::from(cwd),
            None => self.workspace.clone(),
        };
        if !cwd.is_dir() {
            return Err(format!("{} no es un directorio", cwd.display()));
        }
        self.stop(&name).ok();

        let port = free_port()?;
        let base = format!("{PREFIX}{name}/");
        let logs = self.workspace.join(LOGS);
        std::fs::create_dir_all(&logs).map_err(|e| e.to_string())?;
        let log = logs.join(format!("{name}.log"));
        let out = std::fs::File::create(&log).map_err(|e| e.to_string())?;
        let err = out.try_clone().map_err(|e| e.to_string())?;

        let child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(&cwd)
            .env("PORT", port.to_string())
            .env("PREVIEW_PORT", port.to_string())
            .env("PREVIEW_NAME", &name)
            .env("PREVIEW_BASE", &base)
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err)
            .process_group(0)
            .spawn()
            .map_err(|e| format!("no pude lanzar {command}: {e}"))?;

        let started = Instant::now();
        let pid = child.id();
        self.running
            .lock()
            .map_err(|_| "el registro está trabado".to_string())?
            .insert(
                name.clone(),
                Running {
                    port,
                    child,
                    started,
                },
            );
        Ok(Summary {
            name,
            port,
            pid,
            seconds: started.elapsed().as_secs(),
        })
    }

    fn stop(&self, name: &str) -> Result<Summary, String> {
        let name = valid_name(name)?;
        let mut running = self
            .running
            .lock()
            .map_err(|_| "el registro está trabado".to_string())?;
        let Some(mut entry) = running.remove(&name) else {
            return Err(format!("{name} no está corriendo"));
        };
        let pid = entry.child.id();
        let summary = Summary {
            name,
            port: entry.port,
            pid,
            seconds: entry.started.elapsed().as_secs(),
        };
        kill_group(pid as i32, libc::SIGTERM);
        if wait_gone(&mut entry.child, STOP_TIMEOUT).is_err() {
            kill_group(pid as i32, libc::SIGKILL);
            entry.child.wait().ok();
        }
        Ok(summary)
    }

    fn list(&self) -> Vec<Summary> {
        let Ok(mut running) = self.running.lock() else {
            return Vec::new();
        };
        running.retain(|_, entry| matches!(entry.child.try_wait(), Ok(None)));
        running
            .iter()
            .map(|(name, entry)| Summary {
                name: name.clone(),
                port: entry.port,
                pid: entry.child.id(),
                seconds: entry.started.elapsed().as_secs(),
            })
            .collect()
    }

    fn apply(&self, order: &Order) -> Answer {
        match order.op.as_str() {
            "start" => match self.start(order) {
                Ok(summary) => Answer {
                    ok: true,
                    port: Some(summary.port),
                    output: Some(format!(
                        "{} en el puerto {} (pid {})",
                        summary.name, summary.port, summary.pid
                    )),
                    ..Answer::default()
                },
                Err(error) => Answer {
                    ok: false,
                    error: Some(error),
                    ..Answer::default()
                },
            },
            "stop" => match self.stop(&order.name) {
                Ok(summary) => Answer {
                    ok: true,
                    output: Some(format!("{} parado (pid {})", summary.name, summary.pid)),
                    ..Answer::default()
                },
                Err(error) => Answer {
                    ok: false,
                    error: Some(error),
                    ..Answer::default()
                },
            },
            "list" => Answer {
                ok: true,
                previews: self.list(),
                ..Answer::default()
            },
            other => Answer {
                ok: false,
                error: Some(format!("no conozco la orden {other}")),
                ..Answer::default()
            },
        }
    }
}

/// El socket sobre el que llegan las órdenes. Un preview viejo de un arranque
/// anterior no puede quedar vivo, así que el socket se rehace siempre.
pub fn listen(previews: Arc<Previews>) {
    let path = previews.workspace.join(SOCKET);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    std::fs::remove_file(&path).ok();
    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("jimmy: no pude escuchar en {}: {e}", path.display());
            return;
        }
    };
    for stream in listener.incoming().flatten() {
        let previews = previews.clone();
        std::thread::spawn(move || {
            if let Err(e) = answer(&previews, stream) {
                eprintln!("jimmy preview: {e}");
            }
        });
    }
}

fn answer(previews: &Arc<Previews>, mut stream: UnixStream) -> std::io::Result<()> {
    let mut line = String::new();
    BufReader::new(stream.try_clone()?).read_line(&mut line)?;
    let answer = match serde_json::from_str::<Order>(line.trim()) {
        Ok(order) => previews.apply(&order),
        Err(e) => Answer {
            ok: false,
            error: Some(format!("orden ilegible: {e}")),
            ..Answer::default()
        },
    };
    let body = serde_json::to_string(&answer).unwrap_or_default();
    stream.write_all(body.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()
}

/// El cliente del socket: el proceso que pide no es el que lanza.
pub fn call(workspace: &Path, order: &Order) -> Result<Answer, String> {
    let path = workspace.join(SOCKET);
    let mut stream =
        UnixStream::connect(&path).map_err(|e| format!("no pude hablar con jimmy: {e}"))?;
    stream
        .write_all(serde_json::to_string(order).unwrap_or_default().as_bytes())
        .and_then(|()| stream.write_all(b"\n"))
        .map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    serde_json::from_str(line.trim()).map_err(|e| format!("respuesta ilegible: {e}"))
}

impl Drop for Previews {
    fn drop(&mut self) {
        let Ok(running) = self.running.lock() else {
            return;
        };
        for entry in running.values() {
            kill_group(entry.child.id() as i32, libc::SIGTERM);
        }
    }
}

pub fn valid_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("falta el nombre".into());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(format!("{name}: sólo minúsculas, números y guiones"));
    }
    Ok(name.to_string())
}

/// El nombre del preview al que apunta un path, si es un path de preview.
pub fn name_from_path(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix(PREFIX)?;
    match rest.split_once('/') {
        Some((name, tail)) => Some((name, tail)),
        None => Some((rest, "")),
    }
}

/// Cabecera de un pedido, cruda: el proxy tiene que reenviarla tal como llegó.
/// Un pedido de preview no se parsea con `http::read` porque el cuerpo que
/// sigue puede ser el de un WebSocket, y ese se reenvía sin entenderlo.
pub struct Head {
    pub method: String,
    pub target: String,
    pub headers: Vec<(String, String)>,
}

impl Head {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub fn cookie(&self, name: &str) -> Option<String> {
        for part in self.header("cookie")?.split(';') {
            if let Some((key, value)) = part.trim().split_once('=') {
                if key == name {
                    return Some(value.to_string());
                }
            }
        }
        None
    }
}

/// Si el pedido en el socket es de un preview, sin consumir un solo byte: el
/// resto del ruteo sigue leyendo de un stream intacto.
pub fn wants_preview(stream: &mut TcpStream) -> std::io::Result<bool> {
    let mut buffer = [0u8; 1024];
    for _ in 0..40 {
        let read = stream.peek(&mut buffer)?;
        if read == 0 {
            return Ok(false);
        }
        let Some(end) = buffer[..read].windows(2).position(|w| w == b"\r\n") else {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        };
        let line = String::from_utf8_lossy(&buffer[..end]);
        let target = line.split(' ').nth(1).unwrap_or("");
        return Ok(target.starts_with(PREFIX));
    }
    Ok(false)
}

pub fn read_head(stream: &mut TcpStream) -> std::io::Result<Option<Head>> {
    let mut raw = Vec::with_capacity(1024);
    let mut byte = [0u8; 1];
    while !raw.ends_with(b"\r\n\r\n") {
        if raw.len() > MAX_HEAD {
            return Ok(None);
        }
        if stream.read(&mut byte)? == 0 {
            return Ok(None);
        }
        raw.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&raw).to_string();
    let mut lines = text.split("\r\n");
    let Some(request) = lines.next() else {
        return Ok(None);
    };
    let mut parts = request.split(' ');
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("/").to_string();
    let headers = lines
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_string(), value.trim().to_string()))
        })
        .collect();
    Ok(Some(Head {
        method,
        target,
        headers,
    }))
}

/// Reenvía el pedido al preview y devuelve la respuesta sin mirarla: lo que
/// venga después del encabezado se copia crudo, así el streaming y el
/// handshake de un WebSocket pasan igual.
pub fn forward(stream: &mut TcpStream, head: &Head, port: u16) -> std::io::Result<()> {
    let mut upstream = match TcpStream::connect(("127.0.0.1", port)) {
        Ok(upstream) => upstream,
        Err(e) => {
            let body = format!("el preview no contesta en el puerto {port}: {e}\n");
            return crate::http::respond(
                stream,
                502,
                "text/plain; charset=utf-8",
                &[],
                body.as_bytes(),
            );
        }
    };

    let host = head.header("host").unwrap_or("localhost").to_string();
    let mut out = format!("{} {} HTTP/1.1\r\n", head.method, head.target);
    for (name, value) in &head.headers {
        if name.eq_ignore_ascii_case("host") {
            continue;
        }
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    out.push_str(&format!("Host: 127.0.0.1:{port}\r\n"));
    out.push_str(&format!("X-Forwarded-Host: {host}\r\n"));
    out.push_str("\r\n");
    upstream.write_all(out.as_bytes())?;
    upstream.flush()?;

    let mut from_client = stream.try_clone()?;
    let mut to_upstream = upstream.try_clone()?;
    let mut from_upstream = upstream.try_clone()?;
    let mut to_client = stream.try_clone()?;
    std::thread::scope(|scope| {
        scope.spawn(move || {
            let _ = std::io::copy(&mut from_client, &mut to_upstream);
            let _ = to_upstream.shutdown(std::net::Shutdown::Write);
        });
        let _ = std::io::copy(&mut from_upstream, &mut to_client);
        let _ = to_client.shutdown(std::net::Shutdown::Write);
        let _ = stream.shutdown(std::net::Shutdown::Read);
    });
    Ok(())
}

fn free_port() -> Result<u16, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
    listener
        .local_addr()
        .map(|addr| addr.port())
        .map_err(|e| e.to_string())
}

fn kill_group(pid: i32, signal: i32) {
    unsafe { libc::kill(-pid, signal) };
}

fn wait_gone(child: &mut Child, timeout: Duration) -> Result<(), ()> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jimmy-preview-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(LOGS)).unwrap();
        dir
    }

    fn order(op: &str, name: &str, command: &str) -> Order {
        Order {
            op: op.into(),
            name: name.into(),
            command: Some(command.into()),
            cwd: None,
        }
    }

    fn registry() -> (Arc<Previews>, PathBuf) {
        let dir = workspace();
        let previews = Previews::new(&dir);
        (previews, dir)
    }

    fn alive(pid: u32) -> bool {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }

    #[test]
    fn a_preview_runs_and_stops() {
        let (previews, _dir) = registry();
        let started = previews.apply(&order("start", "lento", "exec sleep 30"));
        assert!(started.ok, "{:?}", started.error);
        let port = previews.port("lento").unwrap();
        assert!(port > 0);
        let pids: Vec<u32> = previews.list().iter().map(|s| s.pid).collect();
        assert_eq!(pids.len(), 1);
        assert!(alive(pids[0]));

        let stopped = previews.apply(&Order {
            op: "stop".into(),
            name: "lento".into(),
            ..Order::default()
        });
        assert!(stopped.ok, "{:?}", stopped.error);
        assert!(previews.port("lento").is_none());
        assert!(previews.list().is_empty());
        assert!(!alive(pids[0]));
    }

    #[test]
    fn restarting_a_preview_replaces_the_process() {
        let (previews, _dir) = registry();
        previews.apply(&order("start", "uno", "exec sleep 30"));
        let first = previews.list()[0].pid;
        let again = previews.apply(&order("start", "uno", "exec sleep 30"));
        assert!(again.ok, "{:?}", again.error);
        assert_eq!(previews.list().len(), 1);
        assert!(!alive(first));
    }

    #[test]
    fn a_dead_preview_is_forgotten() {
        let (previews, _dir) = registry();
        previews.apply(&order("start", "corto", "exec sleep 0.1"));
        std::thread::sleep(Duration::from_millis(400));
        assert!(previews.port("corto").is_none());
        assert!(previews.list().is_empty());
    }

    #[test]
    fn a_name_that_is_not_a_slug_is_rejected() {
        assert!(valid_name("bifrost-37").is_ok());
        assert!(valid_name("Bifrost").is_err());
        assert!(valid_name("dos/pisos").is_err());
        assert!(valid_name("  ").is_err());
        assert!(valid_name("a b").is_err());
    }

    #[test]
    fn the_log_keeps_what_the_command_printed() {
        let (previews, _dir) = registry();
        let answer = previews.apply(&order(
            "start",
            "hablador",
            "echo hola-preview; exec sleep 5",
        ));
        assert!(answer.ok, "{:?}", answer.error);
        std::thread::sleep(Duration::from_millis(300));
        assert!(previews.output("hablador").contains("hola-preview"));
    }

    #[test]
    fn the_path_names_the_preview() {
        assert_eq!(name_from_path("/preview/x/"), Some(("x", "")));
        assert_eq!(name_from_path("/preview/x/app.js"), Some(("x", "app.js")));
        assert_eq!(name_from_path("/preview/x"), Some(("x", "")));
        assert_eq!(name_from_path("/app.js"), None);
        assert_eq!(name_from_path("/preview/"), Some(("", "")));
    }

    #[test]
    fn forward_lleva_el_pedido_al_preview_y_trae_la_respuesta() {
        let upstream = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = upstream.local_addr().unwrap().port();
        let seen = std::thread::spawn(move || {
            let (mut conn, _) = upstream.accept().unwrap();
            conn.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                if conn.read(&mut byte).unwrap() == 0 {
                    break;
                }
                head.push(byte[0]);
            }
            let mut body = [0u8; 8];
            let read = conn.read(&mut body).unwrap_or(0);
            conn.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .unwrap();
            (
                String::from_utf8_lossy(&head).to_string(),
                String::from_utf8_lossy(&body[..read]).to_string(),
            )
        });

        let (client, mut server_side) = pair();
        let head = head("/preview/x/");
        let proxying = std::thread::spawn(move || forward(&mut server_side, &head, port).unwrap());

        let mut client = client;
        client.write_all(b"hola").unwrap();
        let mut answer = Vec::new();
        client.read_to_end(&mut answer).unwrap();
        proxying.join().unwrap();

        let answer = String::from_utf8_lossy(&answer).to_string();
        assert!(answer.starts_with("HTTP/1.1 200 OK"), "{answer}");
        assert!(answer.ends_with("ok"), "{answer}");
        let (head, body) = seen.join().unwrap();
        assert!(head.starts_with("GET /preview/x/ HTTP/1.1\r\n"), "{head}");
        assert!(head.contains(&format!("Host: 127.0.0.1:{port}")), "{head}");
        assert!(head.contains("X-Forwarded-Host: jimmy.berti.sh"), "{head}");
        assert_eq!(body, "hola");
    }

    #[test]
    fn forward_relaya_un_websocket_en_los_dos_sentidos() {
        let upstream = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = upstream.local_addr().unwrap().port();
        let echo = std::thread::spawn(move || {
            let (mut conn, _) = upstream.accept().unwrap();
            conn.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                if conn.read(&mut byte).unwrap() == 0 {
                    break;
                }
                head.push(byte[0]);
            }
            conn.write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\n")
                .unwrap();
            conn.write_all(b"frame-del-server").unwrap();
            let mut buf = [0u8; 32];
            let read = conn.read(&mut buf).unwrap_or(0);
            String::from_utf8_lossy(&buf[..read]).to_string()
        });

        let (client, mut server_side) = pair();
        let head = head("/preview/x/?token=x");
        let proxying = std::thread::spawn(move || forward(&mut server_side, &head, port).unwrap());

        let mut client = client;
        let mut handshake = Vec::new();
        let mut byte = [0u8; 1];
        while !handshake.ends_with(b"\r\n\r\n") {
            assert!(client.read(&mut byte).unwrap() > 0);
            handshake.push(byte[0]);
        }
        assert!(String::from_utf8_lossy(&handshake).starts_with("HTTP/1.1 101"));
        let mut frame = [0u8; 16];
        let read = client.read(&mut frame).unwrap();
        assert_eq!(&frame[..read], b"frame-del-server");
        client.write_all(b"ping-del-cliente").unwrap();
        client.flush().unwrap();

        assert_eq!(echo.join().unwrap(), "ping-del-cliente");
        proxying.join().unwrap();
    }

    fn head(target: &str) -> Head {
        Head {
            method: "GET".into(),
            target: target.into(),
            headers: vec![("Host".into(), "jimmy.berti.sh".into())],
        }
    }

    /// Un cliente y el otro extremo, como los ve el proxy.
    fn pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = std::thread::spawn(move || TcpStream::connect(("127.0.0.1", port)).unwrap());
        let (server_side, _) = listener.accept().unwrap();
        (client.join().unwrap(), server_side)
    }

    #[test]
    fn the_socket_answers_an_order_and_a_bad_op() {
        let (previews, dir) = registry();
        let listener = previews.clone();
        std::thread::spawn(move || listen(listener));
        std::thread::sleep(Duration::from_millis(200));

        let started = call(&dir, &order("start", "eco", "exec sleep 30")).unwrap();
        assert!(started.ok, "{:?}", started.error);
        assert!(started.port.unwrap() > 0);

        let listed = call(
            &dir,
            &Order {
                op: "list".into(),
                ..Order::default()
            },
        )
        .unwrap();
        assert!(listed.ok);
        assert_eq!(listed.previews.len(), 1);
        assert_eq!(listed.previews[0].name, "eco");

        let nonsense = call(
            &dir,
            &Order {
                op: "bailar".into(),
                ..Order::default()
            },
        )
        .unwrap();
        assert!(!nonsense.ok);

        let stopped = call(
            &dir,
            &Order {
                op: "stop".into(),
                name: "eco".into(),
                ..Order::default()
            },
        )
        .unwrap();
        assert!(stopped.ok, "{:?}", stopped.error);
        let empty = call(
            &dir,
            &Order {
                op: "list".into(),
                ..Order::default()
            },
        )
        .unwrap();
        assert!(empty.previews.is_empty());
    }
}
