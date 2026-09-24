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

/// Reenvía el pedido al preview, sin el prefijo, y devuelve la respuesta
/// reescrita: el proyecto se sirve en `/` y no sabe que vive bajo
/// `/preview/<nombre>/`, así que todo lo que sale apuntando a la raíz vuelve
/// prefijado.
///
/// Sólo se reescribe lo que es texto de la página (HTML, CSS y JavaScript). Un
/// cuerpo con streaming, un binario o un `Upgrade` se copian crudos.
pub fn forward(stream: &mut TcpStream, head: &Head, name: &str, port: u16) -> std::io::Result<()> {
    let prefix = format!("{PREFIX}{name}/");
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
    let base = format!("{PREFIX}{name}");
    let upgrading = head.header("upgrade").is_some();
    let target = match head.target.strip_prefix(&base) {
        Some("") => "/",
        Some(rest) => rest,
        None => "/",
    };
    let mut out = format!("{} {target} HTTP/1.1\r\n", head.method);
    for (name, value) in &head.headers {
        if name.eq_ignore_ascii_case("host") || name.eq_ignore_ascii_case("accept-encoding") {
            continue;
        }
        if !upgrading && name.eq_ignore_ascii_case("connection") {
            continue;
        }
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    out.push_str(&format!("Host: 127.0.0.1:{port}\r\n"));
    out.push_str(&format!("X-Forwarded-Host: {host}\r\n"));
    if !upgrading {
        out.push_str("Connection: close\r\n");
    }
    out.push_str("\r\n");
    upstream.write_all(out.as_bytes())?;
    upstream.flush()?;

    let request_body = head
        .header("content-length")
        .and_then(|length| length.parse::<usize>().ok())
        .unwrap_or(0);
    if request_body > 0 {
        let mut body = vec![0u8; request_body];
        stream.read_exact(&mut body)?;
        upstream.write_all(&body)?;
        upstream.flush()?;
    }

    let Some((status, headers)) = read_response_head(&mut upstream)? else {
        return Ok(());
    };
    let headers = with_location(&headers, &prefix);
    let ctype = header_value(&headers, "content-type").unwrap_or("");

    if !rewritable(ctype) || status == 101 || status == 304 {
        write!(stream, "HTTP/1.1 {status} {}\r\n", reason(status))?;
        for (name, value) in &headers {
            if status != 101 && name.eq_ignore_ascii_case("connection") {
                continue;
            }
            write!(stream, "{name}: {value}\r\n")?;
        }
        if status != 101 {
            stream.write_all(b"Connection: close\r\n")?;
        }
        stream.write_all(b"\r\n")?;
        stream.flush()?;
        let mut from_client = stream.try_clone()?;
        let mut from_upstream = upstream.try_clone()?;
        let mut to_upstream = upstream.try_clone()?;
        std::thread::scope(|scope| {
            scope.spawn(move || {
                let _ = std::io::copy(&mut from_client, &mut to_upstream);
                let _ = to_upstream.shutdown(std::net::Shutdown::Write);
            });
            let _ = std::io::copy(&mut from_upstream, stream);
            let _ = stream.shutdown(std::net::Shutdown::Read);
        });
        return Ok(());
    }

    let body = read_body(&mut upstream, &headers)?;
    let body = rewrite(ctype, &body, &prefix);
    write!(stream, "HTTP/1.1 {status} {}\r\n", reason(status))?;
    for (name, value) in &headers {
        if !networking_header(name) {
            write!(stream, "{name}: {value}\r\n")?;
        }
    }
    write!(
        stream,
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(&body)?;
    stream.flush()
}

fn networking_header(name: &str) -> bool {
    ["content-length", "transfer-encoding", "connection"]
        .iter()
        .any(|skip| name.eq_ignore_ascii_case(skip))
}
fn rewritable(ctype: &str) -> bool {
    ["text/html", "text/css", "javascript"]
        .iter()
        .any(|kind| ctype.contains(kind))
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "OK",
    }
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

/// Un redirect que sale apuntando a la raíz tiene que volver al preview, o el
/// navegador se va a jimmy.
fn with_location(headers: &[(String, String)], prefix: &str) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            if !name.eq_ignore_ascii_case("location") {
                return (name.clone(), value.clone());
            }
            if !value.starts_with('/') || value.starts_with("//") || value.starts_with(prefix) {
                return (name.clone(), value.clone());
            }
            (name.clone(), format!("{prefix}{}", &value[1..]))
        })
        .collect()
}

type ResponseHead = (u16, Vec<(String, String)>);

fn read_response_head(stream: &mut TcpStream) -> std::io::Result<Option<ResponseHead>> {
    let mut raw = Vec::with_capacity(512);
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
    let status = lines
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or(200);
    let headers = lines
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_string(), value.trim().to_string()))
        })
        .collect();
    Ok(Some((status, headers)))
}

fn read_body(stream: &mut TcpStream, headers: &[(String, String)]) -> std::io::Result<Vec<u8>> {
    let chunked = header_value(headers, "transfer-encoding")
        .map(|value| value.eq_ignore_ascii_case("chunked"))
        .unwrap_or(false);
    if chunked {
        let mut body = Vec::new();
        loop {
            let size = usize::from_str_radix(read_line(stream)?.trim(), 16).unwrap_or(0);
            if size == 0 {
                read_line(stream)?;
                return Ok(body);
            }
            let mut chunk = vec![0u8; size];
            stream.read_exact(&mut chunk)?;
            body.extend_from_slice(&chunk);
            read_line(stream)?;
        }
    }
    if let Some(length) = header_value(headers, "content-length").and_then(|v| v.parse().ok()) {
        let mut body = vec![0u8; length];
        stream.read_exact(&mut body)?;
        return Ok(body);
    }
    let mut body = Vec::new();
    stream.read_to_end(&mut body)?;
    Ok(body)
}

fn read_line(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        if stream.read(&mut byte)? == 0 || byte[0] == b'\n' {
            return Ok(String::from_utf8_lossy(&line).to_string());
        }
        line.push(byte[0]);
    }
}

/// El prefijo va sobre los paths que quedaron apuntando a la raíz. Un path que
/// ya empieza con `//` (un host prestado) se deja como está.
fn rewrite(ctype: &str, body: &[u8], prefix: &str) -> Vec<u8> {
    let text = String::from_utf8_lossy(body).to_string();
    let text = if ctype.contains("text/html") {
        ["href=\"", "src=\"", "action=\"", "srcset=\""]
            .iter()
            .fold(text, |text, needle| prefix_after(&text, needle, prefix))
    } else if ctype.contains("text/css") {
        prefix_after(&text, "url(", prefix)
    } else if ctype.contains("javascript") {
        prefix_after(&prefix_after(&text, "\"", prefix), "'", prefix)
    } else {
        text
    };
    text.into_bytes()
}

fn prefix_after(text: &str, needle: &str, prefix: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find(needle) {
        let end = index + needle.len();
        out.push_str(&rest[..end]);
        rest = &rest[end..];
        if !rest.starts_with('/') || rest.starts_with("//") {
            continue;
        }
        out.push_str(prefix);
        rest = &rest[1..];
    }
    out.push_str(rest);
    out
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
    fn forward_quita_el_prefijo_y_reescribe_lo_que_sale() {
        let upstream = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = upstream.local_addr().unwrap().port();
        let seen = std::thread::spawn(move || {
            let (mut conn, _) = upstream.accept().unwrap();
            conn.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let head = read_until_blank(&mut conn);
            let mut body = [0u8; 8];
            let read = conn.read(&mut body).unwrap_or(0);
            let page = "<a href=\"/app.js\">x</a><img src=\"/f.png\">";
            conn.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{page}",
                    page.len()
                )
                .as_bytes(),
            )
            .unwrap();
            (head, String::from_utf8_lossy(&body[..read]).to_string())
        });

        let (client, mut server_side) = pair();
        let head = head_with("/preview/x/", &[("Content-Length", "4")]);
        let proxying =
            std::thread::spawn(move || forward(&mut server_side, &head, "x", port).unwrap());

        let mut client = client;
        client.write_all(b"hola").unwrap();
        let mut answer = Vec::new();
        client.read_to_end(&mut answer).unwrap();
        proxying.join().unwrap();

        let answer = String::from_utf8_lossy(&answer).to_string();
        assert!(answer.starts_with("HTTP/1.1 200 OK"), "{answer}");
        assert!(answer.contains("href=\"/preview/x/app.js\""), "{answer}");
        assert!(answer.contains("src=\"/preview/x/f.png\""), "{answer}");
        let (head, body) = seen.join().unwrap();
        assert!(head.starts_with("GET / HTTP/1.1\r\n"), "{head}");
        assert!(head.contains(&format!("Host: 127.0.0.1:{port}")), "{head}");
        assert!(head.contains("X-Forwarded-Host: jimmy.berti.sh"), "{head}");
        assert_eq!(body, "hola");
    }

    #[test]
    fn forward_arma_el_cuerpo_fragmentado_y_saca_el_prefijo_del_redirect() {
        let upstream = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = upstream.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut conn, _) = upstream.accept().unwrap();
            read_until_blank(&mut conn);
            conn.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/css\r\nTransfer-Encoding: chunked\r\n\r\n",
            )
            .unwrap();
            let body = "body { background: url(/f.png) }";
            conn.write_all(format!("{:x}\r\n{body}\r\n0\r\n\r\n", body.len()).as_bytes())
                .unwrap();
        });

        let (client, mut server_side) = pair();
        let head = head_with("/preview/x/estilo.css", &[]);
        let proxying =
            std::thread::spawn(move || forward(&mut server_side, &head, "x", port).unwrap());
        let mut client = client;
        let mut answer = Vec::new();
        client.read_to_end(&mut answer).unwrap();
        proxying.join().unwrap();

        let answer = String::from_utf8_lossy(&answer).to_string();
        assert!(answer.contains("Content-Length:"), "{answer}");
        assert!(!answer.contains("chunked"), "{answer}");
        assert!(answer.contains("url(/preview/x/f.png)"), "{answer}");

        let upstream = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = upstream.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut conn, _) = upstream.accept().unwrap();
            read_until_blank(&mut conn);
            conn.write_all(b"HTTP/1.1 302 Found\r\nLocation: /panel\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
        });
        let (client, mut server_side) = pair();
        let head = head_with("/preview/x/viejo", &[]);
        let proxying =
            std::thread::spawn(move || forward(&mut server_side, &head, "x", port).unwrap());
        let mut client = client;
        let mut answer = Vec::new();
        client.read_to_end(&mut answer).unwrap();
        proxying.join().unwrap();
        let answer = String::from_utf8_lossy(&answer).to_string();
        assert!(answer.contains("Location: /preview/x/panel"), "{answer}");
    }

    #[test]
    fn forward_relaya_un_websocket_en_los_dos_sentidos() {
        let upstream = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = upstream.local_addr().unwrap().port();
        let echo = std::thread::spawn(move || {
            let (mut conn, _) = upstream.accept().unwrap();
            conn.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let head = read_until_blank(&mut conn);
            conn.write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\n")
                .unwrap();
            conn.write_all(b"frame-del-server").unwrap();
            let mut buf = [0u8; 32];
            let read = conn.read(&mut buf).unwrap_or(0);
            (head, String::from_utf8_lossy(&buf[..read]).to_string())
        });

        let (client, mut server_side) = pair();
        let head = head_with("/preview/x/ws?token=y", &[]);
        let proxying =
            std::thread::spawn(move || forward(&mut server_side, &head, "x", port).unwrap());

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

        let (head, ping) = echo.join().unwrap();
        assert!(head.starts_with("GET /ws?token=y HTTP/1.1\r\n"), "{head}");
        assert_eq!(ping, "ping-del-cliente");
        proxying.join().unwrap();
    }

    #[test]
    fn forward_le_dice_al_navegador_que_la_conexion_es_de_una_sola_vez() {
        let upstream = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = upstream.local_addr().unwrap().port();
        let seen = std::thread::spawn(move || {
            let (mut conn, _) = upstream.accept().unwrap();
            let head = read_until_blank(&mut conn);
            conn.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: keep-alive\r\nContent-Length: 2\r\n\r\n42")
                .unwrap();
            head
        });

        let (client, mut server_side) = pair();
        let head = head_with("/preview/x/api/dato", &[("Connection", "keep-alive")]);
        let proxying =
            std::thread::spawn(move || forward(&mut server_side, &head, "x", port).unwrap());
        let mut client = client;
        let mut answer = Vec::new();
        client.read_to_end(&mut answer).unwrap();
        proxying.join().unwrap();

        let answer = String::from_utf8_lossy(&answer).to_string();
        assert!(answer.contains("Connection: close"), "{answer}");
        assert!(!answer.contains("keep-alive"), "{answer}");
        assert!(answer.ends_with("42"), "{answer}");
        let head = seen.join().unwrap();
        assert!(head.contains("Connection: close"), "{head}");
    }

    #[test]
    fn rewrite_prefija_lo_que_apunta_a_la_raiz_y_no_toca_lo_prestado() {
        let prefix = "/preview/x/";
        let html = rewrite(
            "text/html",
            b"<a href=\"/a\">1</a><script src=\"/b.js\"></script><img src=\"//cdn.com/c.png\">",
            prefix,
        );
        let html = String::from_utf8(html).unwrap();
        assert!(html.contains("href=\"/preview/x/a\""), "{html}");
        assert!(html.contains("src=\"/preview/x/b.js\""), "{html}");
        assert!(html.contains("src=\"//cdn.com/c.png\""), "{html}");

        let css = rewrite("text/css", b"a { background: url(/f.png) }", prefix);
        assert_eq!(
            String::from_utf8(css).unwrap(),
            "a { background: url(/preview/x/f.png) }"
        );

        let js = rewrite(
            "application/javascript",
            b"fetch(\"/api/dato\"); const u = '/otro'; const r = a / b;",
            prefix,
        );
        let js = String::from_utf8(js).unwrap();
        assert!(js.contains("fetch(\"/preview/x/api/dato\")"), "{js}");
        assert!(js.contains("'/preview/x/otro'"), "{js}");
        assert!(js.contains("a / b"), "{js}");

        let otros = rewrite("application/json", b"{\"url\":\"/api/x\"}", prefix);
        assert_eq!(String::from_utf8(otros).unwrap(), "{\"url\":\"/api/x\"}");
    }

    #[test]
    fn with_location_deja_afuera_lo_que_no_es_del_preview() {
        let headers = vec![
            ("Location".to_string(), "/panel".to_string()),
            ("X-Otro".to_string(), "/no-tocar".to_string()),
        ];
        let out = with_location(&headers, "/preview/x/");
        assert_eq!(out[0].1, "/preview/x/panel");
        assert_eq!(out[1].1, "/no-tocar");

        let ya = vec![("Location".to_string(), "/preview/x/panel".to_string())];
        assert_eq!(with_location(&ya, "/preview/x/")[0].1, "/preview/x/panel");
        let afuera = vec![("Location".to_string(), "https://otro.com/x".to_string())];
        assert_eq!(
            with_location(&afuera, "/preview/x/")[0].1,
            "https://otro.com/x"
        );
    }

    fn head_with(target: &str, extra: &[(&str, &str)]) -> Head {
        let mut headers = vec![("Host".to_string(), "jimmy.berti.sh".to_string())];
        for (name, value) in extra {
            headers.push((name.to_string(), value.to_string()));
        }
        Head {
            method: "GET".into(),
            target: target.into(),
            headers,
        }
    }

    fn read_until_blank(conn: &mut TcpStream) -> String {
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            if conn.read(&mut byte).unwrap() == 0 {
                break;
            }
            head.push(byte[0]);
        }
        String::from_utf8_lossy(&head).to_string()
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
