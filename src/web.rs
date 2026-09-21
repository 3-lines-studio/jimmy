//! The web frontend: the same conversations the transports have, served to a
//! browser over HTTP and SSE.
//!
//! Read-only for now: the conversations that arrive through a transport are
//! watched, not written.

use crate::bus::Bus;
use crate::conversations;
use crate::http::{self, Request};
use crate::log::Log;
use crate::users::{self, Sessions};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::RecvTimeoutError;
use std::sync::Arc;
use std::time::Duration;

/// The end of the backlog: everything before it is already rendered.
const SYNCED: &str = r#"{"type":"synced"}"#;

pub struct Web {
    root: PathBuf,
    workspace: PathBuf,
    bus: Arc<Bus>,
    sessions: Sessions,
}

impl Web {
    pub fn new(root: PathBuf, workspace: PathBuf, bus: Arc<Bus>) -> Arc<Web> {
        let sessions = Sessions::load(&root);
        Arc::new(Web {
            root,
            workspace,
            bus,
            sessions,
        })
    }

    fn user(&self, request: &Request) -> Option<String> {
        self.sessions.user(&request.cookie(users::COOKIE)?)
    }
}

pub fn listen(port: u16) -> Result<TcpListener, String> {
    TcpListener::bind(("0.0.0.0", port))
        .map_err(|e| format!("no pude escuchar en el puerto {port}: {e}"))
}

pub fn serve(web: Arc<Web>, listener: TcpListener) {
    for stream in listener.incoming().flatten() {
        let web = web.clone();
        std::thread::spawn(move || {
            let mut stream = stream;
            if let Err(e) = handle(&web, &mut stream) {
                eprintln!("jimmy web: {e}");
            }
        });
    }
}

fn handle(web: &Arc<Web>, stream: &mut TcpStream) -> std::io::Result<()> {
    let Some(request) = http::read(stream)? else {
        return Ok(());
    };
    match (request.method.as_str(), request.path.as_str()) {
        ("POST", "/api/login") => login(web, &request, stream),
        ("POST", "/api/logout") => logout(web, &request, stream),
        ("GET", "/api/state") => state(web, &request, stream),
        ("GET", "/api/stream") => events(web, &request, stream),
        _ => http::send_text(stream, 404, "text/plain", "no está"),
    }
}

fn login(web: &Arc<Web>, request: &Request, stream: &mut TcpStream) -> std::io::Result<()> {
    let name = request.field("user").unwrap_or_default();
    let password = request.field("password").unwrap_or_default();
    if !users::verify(&web.root, &name, &password) {
        return http::send_error(stream, 401, "usuario o contraseña mal");
    }
    let token = web.sessions.open(&name);
    let cookie = format!("{}={token}; Path=/; HttpOnly; SameSite=Lax", users::COOKIE);
    let body = serde_json::json!({ "user": name }).to_string();
    http::respond(
        stream,
        200,
        "application/json",
        &[("Set-Cookie", &cookie)],
        body.as_bytes(),
    )
}

fn logout(web: &Arc<Web>, request: &Request, stream: &mut TcpStream) -> std::io::Result<()> {
    if let Some(token) = request.cookie(users::COOKIE) {
        web.sessions.close(&token);
    }
    let cookie = format!("{}=; Path=/; Max-Age=0", users::COOKIE);
    http::respond(
        stream,
        200,
        "application/json",
        &[("Set-Cookie", &cookie)],
        b"{}",
    )
}

fn state(web: &Arc<Web>, request: &Request, stream: &mut TcpStream) -> std::io::Result<()> {
    let Some(user) = web.user(request) else {
        return http::send_error(stream, 401, "no estás adentro");
    };
    let projects: Vec<serde_json::Value> = conversations::projects(&web.root, &web.workspace)
        .into_iter()
        .map(|project| {
            let conversations: Vec<serde_json::Value> = project
                .conversations
                .iter()
                .map(|conversation| {
                    serde_json::json!({
                        "key": conversation.key,
                        "title": conversation.title,
                        "read_only": conversation.read_only,
                    })
                })
                .collect();
            serde_json::json!({
                "name": project.name,
                "path": project.path.display().to_string(),
                "conversations": conversations,
            })
        })
        .collect();
    http::send_json(
        stream,
        200,
        &serde_json::json!({ "user": user, "projects": projects }),
    )
}

fn events(web: &Arc<Web>, request: &Request, stream: &mut TcpStream) -> std::io::Result<()> {
    if web.user(request).is_none() {
        return http::send_error(stream, 401, "no estás adentro");
    }
    let Some(key) = request.param("conversation") else {
        return http::send_error(stream, 400, "falta conversation");
    };
    let conversation = conversations::get(&web.root, &web.workspace, key);
    if !conversation.dir.is_dir() {
        return http::send_error(stream, 404, "esa conversación no existe");
    }
    let log = Log::in_dir(&conversation.dir);
    let (backlog, live) = web.bus.attach(key, &log);

    http::sse_open(stream)?;
    for event in &backlog {
        if let Ok(json) = serde_json::to_string(event) {
            http::sse_data(stream, &json)?;
        }
    }
    http::sse_data(stream, SYNCED)?;
    loop {
        match live.recv_timeout(Duration::from_secs(5)) {
            Ok(event) => {
                if let Ok(json) = serde_json::to_string(&event) {
                    http::sse_data(stream, &json)?;
                }
            }
            Err(RecvTimeoutError::Timeout) => http::sse_ping(stream)?,
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Event;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpStream;

    struct Server {
        root: PathBuf,
        port: u16,
        bus: Arc<Bus>,
    }

    fn start(tag: &str) -> Server {
        let base = std::env::temp_dir().join(format!("jimmy-web-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("root");
        let workspace = base.join("workspace");
        std::fs::create_dir_all(root.join("chats/7469057930")).unwrap();
        std::fs::create_dir_all(workspace.join("projects/ken")).unwrap();
        std::fs::write(
            root.join("chats/7469057930/conversation.jsonl"),
            "{\"event\":\"user\",\"text\":\"hola\"}\n{\"event\":\"done\",\"text\":\"listo\"}\n",
        )
        .unwrap();
        users::add(&root, "berti", "secreto").unwrap();

        let bus = Bus::new();
        let web = Web::new(root.clone(), workspace, bus.clone());
        let listener = listen(0).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || serve(web, listener));
        Server { root, port, bus }
    }

    fn connect(port: u16) -> TcpStream {
        TcpStream::connect(("127.0.0.1", port)).unwrap()
    }

    fn whole(mut stream: TcpStream) -> String {
        let mut out = String::new();
        stream.read_to_string(&mut out).unwrap();
        out
    }

    fn get(port: u16, path: &str, cookie: Option<&str>) -> String {
        let mut stream = connect(port);
        let cookie = cookie
            .map(|c| format!("Cookie: {c}\r\n"))
            .unwrap_or_default();
        write!(stream, "GET {path} HTTP/1.1\r\nHost: jimmy\r\n{cookie}\r\n").unwrap();
        whole(stream)
    }

    fn post(port: u16, path: &str, body: &str) -> String {
        let mut stream = connect(port);
        write!(
            stream,
            "POST {path} HTTP/1.1\r\nHost: jimmy\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        whole(stream)
    }

    fn token(response: &str) -> String {
        let line = response
            .lines()
            .find(|line| line.starts_with("Set-Cookie"))
            .expect("esperaba la cookie");
        line.split_once("jimmy_session=")
            .expect("esperaba el token")
            .1
            .split(';')
            .next()
            .unwrap()
            .to_string()
    }

    #[test]
    fn everything_but_logging_in_needs_a_session() {
        let server = start("auth");
        assert!(get(server.port, "/api/state", None).starts_with("HTTP/1.1 401"));
        assert!(post(
            server.port,
            "/api/login",
            r#"{"user":"berti","password":"mal"}"#
        )
        .starts_with("HTTP/1.1 401"));
        assert!(
            get(server.port, "/api/state", Some("jimmy_session=nada")).starts_with("HTTP/1.1 401")
        );
        assert!(get(server.port, "/nada", None).starts_with("HTTP/1.1 404"));
        std::fs::remove_dir_all(server.root.parent().unwrap()).unwrap();
    }

    #[test]
    fn the_state_lists_projects_and_conversations() {
        let server = start("state");
        let login = post(
            server.port,
            "/api/login",
            r#"{"user":"berti","password":"secreto"}"#,
        );
        assert!(login.starts_with("HTTP/1.1 200"), "{login}");
        let cookie = format!("jimmy_session={}", token(&login));

        let state = get(server.port, "/api/state", Some(&cookie));
        assert!(state.contains("\"general\""), "{state}");
        assert!(state.contains("\"7469057930\""), "{state}");
        assert!(state.contains("\"read_only\":true"), "{state}");
        assert!(state.contains("\"ken\""), "{state}");
        assert!(state.contains("\"berti\""), "{state}");

        let logout = post(server.port, "/api/logout", "{}");
        assert!(logout.contains("Max-Age=0"), "{logout}");
        std::fs::remove_dir_all(server.root.parent().unwrap()).unwrap();
    }

    #[test]
    fn the_stream_replays_the_backlog_and_then_follows() {
        let server = start("stream");
        let login = post(
            server.port,
            "/api/login",
            r#"{"user":"berti","password":"secreto"}"#,
        );
        let cookie = format!("jimmy_session={}", token(&login));

        let mut stream = connect(server.port);
        write!(
            stream,
            "GET /api/stream?conversation=7469057930 HTTP/1.1\r\nHost: jimmy\r\nCookie: {cookie}\r\n\r\n"
        )
        .unwrap();
        let mut lines = BufReader::new(stream.try_clone().unwrap()).lines();

        let mut seen = Vec::new();
        while seen.len() < 3 {
            let line = lines.next().unwrap().unwrap();
            if let Some(data) = line.strip_prefix("data: ") {
                seen.push(data.to_string());
            }
        }
        assert!(seen[0].contains("\"hola\""), "{seen:?}");
        assert!(seen[1].contains("\"listo\""), "{seen:?}");
        assert!(seen[2].contains("synced"), "{seen:?}");

        let log = Log::in_dir(&server.root.join("chats/7469057930"));
        server.bus.publish(
            "7469057930",
            &log,
            &Event::Assistant {
                text: "en vivo".into(),
            },
        );
        let mut live = String::new();
        while !live.contains("en vivo") {
            live = lines.next().unwrap().unwrap();
        }
        assert!(live.starts_with("data: "), "{live}");
        std::fs::remove_dir_all(server.root.parent().unwrap()).unwrap();
    }
}
