//! The web frontend: the same conversations the transports have, served to a
//! browser over HTTP and SSE.
//!
//! What arrives through a transport is watched, not written; the conversations
//! the web creates are its own and can be written from here.

use crate::agent::Agent;
use crate::bus::Bus;
use crate::conversations;
use crate::http::{self, Request};
use crate::log::Log;
use crate::transport::{Msg, Session, Transport};
use crate::users::{self, Sessions};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::Arc;
use std::time::Duration;

/// The end of the backlog: everything before it is already rendered.
const SYNCED: &str = r#"{"event":"synced"}"#;

const INDEX: &str = include_str!("../web/index.html");
const LOGIN: &str = include_str!("../web/login.html");
const STYLE: &str = include_str!("../web/style.css");
const APP: &str = include_str!("../web/app.js");

pub struct Web {
    root: PathBuf,
    workspace: PathBuf,
    bus: Arc<Bus>,
    sessions: Sessions,
    agent: Agent,
}

impl Web {
    pub fn new(root: PathBuf, workspace: PathBuf, bus: Arc<Bus>, agent: Agent) -> Arc<Web> {
        let sessions = Sessions::load(&root);
        Arc::new(Web {
            root,
            workspace,
            bus,
            sessions,
            agent,
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
        ("GET", "/") => app_page(web, &request, stream),
        ("GET", "/login") => http::send_text(stream, 200, HTML, LOGIN),
        ("GET", "/style.css") => http::send_text(stream, 200, "text/css; charset=utf-8", STYLE),
        ("GET", "/app.js") => http::send_text(stream, 200, "text/javascript; charset=utf-8", APP),
        ("POST", "/api/login") => login(web, &request, stream),
        ("POST", "/api/logout") => logout(web, &request, stream),
        ("GET", "/api/state") => state(web, &request, stream),
        ("GET", "/api/stream") => events(web, &request, stream),
        ("POST", "/api/conversations") => create(web, &request, stream),
        ("POST", "/api/projects") => create_project(web, &request, stream),
        ("POST", "/api/rename") => rename(web, &request, stream),
        ("POST", "/api/send") => send(web, &request, stream),
        _ => http::send_text(stream, 404, "text/plain", "no está"),
    }
}

const HTML: &str = "text/html; charset=utf-8";

fn app_page(web: &Arc<Web>, request: &Request, stream: &mut TcpStream) -> std::io::Result<()> {
    if web.user(request).is_none() {
        return http::respond(stream, 303, "text/plain", &[("Location", "/login")], b"");
    }
    http::send_text(stream, 200, HTML, INDEX)
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
                        "running": web.agent.running(&conversation.key),
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

fn create(web: &Arc<Web>, request: &Request, stream: &mut TcpStream) -> std::io::Result<()> {
    if web.user(request).is_none() {
        return http::send_error(stream, 401, "no estás adentro");
    }
    let project = request.field("project").unwrap_or_default();
    let title = request
        .field("title")
        .unwrap_or_else(|| conversations::NEW_TITLE.to_string());
    match conversations::create(&web.root, &web.workspace, &project, &title) {
        Ok(key) => http::send_json(stream, 200, &serde_json::json!({ "key": key })),
        Err(error) => http::send_error(stream, 400, &error),
    }
}

fn create_project(
    web: &Arc<Web>,
    request: &Request,
    stream: &mut TcpStream,
) -> std::io::Result<()> {
    if web.user(request).is_none() {
        return http::send_error(stream, 401, "no estás adentro");
    }
    let name = request.field("name").unwrap_or_default();
    let name = name.trim();
    if name.is_empty()
        || name == conversations::GENERAL
        || name.contains('/')
        || name.starts_with('.')
    {
        return http::send_error(stream, 400, "ese nombre no sirve para un proyecto");
    }
    let dir = web.workspace.join("projects").join(name);
    if dir.exists() {
        return http::send_error(stream, 400, "ese proyecto ya existe");
    }
    if let Err(error) = std::fs::create_dir_all(&dir) {
        return http::send_error(stream, 500, &error.to_string());
    }
    http::send_json(stream, 200, &serde_json::json!({ "name": name }))
}

fn rename(web: &Arc<Web>, request: &Request, stream: &mut TcpStream) -> std::io::Result<()> {
    if web.user(request).is_none() {
        return http::send_error(stream, 401, "no estás adentro");
    }
    let key = request.field("conversation").unwrap_or_default();
    let title = request.field("title").unwrap_or_default();
    if writable(web, &key).is_err() {
        return http::send_error(stream, 400, "esa conversación no se escribe desde acá");
    }
    match conversations::rename(&web.root, &key, &title) {
        Ok(()) => http::send_json(
            stream,
            200,
            &serde_json::json!({ "key": key, "title": title }),
        ),
        Err(error) => http::send_error(stream, 400, &error),
    }
}

fn send(web: &Arc<Web>, request: &Request, stream: &mut TcpStream) -> std::io::Result<()> {
    if web.user(request).is_none() {
        return http::send_error(stream, 401, "no estás adentro");
    }
    let key = request.field("conversation").unwrap_or_default();
    let text = request.field("text").unwrap_or_default();
    if text.trim().is_empty() {
        return http::send_error(stream, 400, "el mensaje está vacío");
    }
    let Ok(conversation) = writable(web, &key) else {
        return http::send_error(stream, 400, "esa conversación no se escribe desde acá");
    };
    if conversation.title.is_none()
        || conversation.title.as_deref() == Some(conversations::NEW_TITLE)
    {
        let _ = conversations::rename(&web.root, &key, &title_from(&text));
    }
    let Some(session) = crate::session_from_key(&key) else {
        return http::send_error(stream, 400, "clave de conversación inválida");
    };

    let web = web.clone();
    std::thread::spawn(move || {
        let lock = crate::chat_lock(&session.key());
        let _guard = lock.lock().unwrap();
        if let Err(error) = web.agent.respond(&Silent, &session, &text, Vec::new()) {
            eprintln!("jimmy web: {error}");
        }
    });
    http::send_json(stream, 202, &serde_json::json!({ "started": true }))
}

/// The answer of a web conversation is its own log, which the browser is
/// already watching, so there is nobody to hand it to here.
struct Silent;

impl Transport for Silent {
    fn parse_target(&self, key: &str) -> Result<Session, String> {
        crate::session_from_key(key).ok_or_else(|| "clave de conversación inválida".into())
    }

    fn progress(&self, _: &Session) -> Option<Msg> {
        None
    }

    fn answer(&self, _: &Session, _: Option<Msg>, _: &str) {}

    fn note(&self, _: &Session, _: &str) {}

    fn fail(&self, _: &Session, _: Option<Msg>, _: &str) {}

    fn download(&self, _: &str) -> Result<(String, Vec<u8>), String> {
        Err("la web no baja archivos".into())
    }

    fn send_media(&self, _: &Session, _: &Path, _: Option<&str>) -> Result<Msg, String> {
        Err("la web no manda archivos".into())
    }
}

/// Una conversación sin nombre se llama como su primer mensaje, que es lo que
/// va a buscar el ojo en la lista.
fn title_from(text: &str) -> String {
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();
    let mut title: String = line.chars().take(60).collect();
    if line.chars().count() > 60 {
        title.push('…');
    }
    title
}

fn writable(web: &Arc<Web>, key: &str) -> Result<conversations::Conversation, ()> {
    let conversation = conversations::get(&web.root, &web.workspace, key);
    if conversation.read_only || !conversation.dir.is_dir() {
        return Err(());
    }
    Ok(conversation)
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
        use std::os::unix::fs::PermissionsExt;

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

        let script = base.join("worker.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\necho '{\"event\":\"ready\"}'
while read -r line; do
  case \"$line\" in *shutdown*) exit 0 ;; esac
  echo '{\"event\":\"done\",\"text\":\"eco\"}'
done
",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let mut agent = Agent::new(
            "http://localhost".into(),
            "model".into(),
            "key".into(),
            Some(1_000_000),
            root.clone(),
            workspace.display().to_string(),
            String::new(),
        );
        agent.use_worker_exe(script);

        let bus = Bus::new();
        let web = Web::new(root.clone(), workspace, bus.clone(), agent);
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
        post_with(port, path, body, None)
    }

    fn post_with(port: u16, path: &str, body: &str, cookie: Option<&str>) -> String {
        let mut stream = connect(port);
        let cookie = cookie
            .map(|c| format!("Cookie: {c}\r\n"))
            .unwrap_or_default();
        write!(
            stream,
            "POST {path} HTTP/1.1\r\nHost: jimmy\r\n{cookie}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
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
    fn the_page_is_served_only_with_a_session() {
        let server = start("assets");
        let redirect = get(server.port, "/", None);
        assert!(redirect.starts_with("HTTP/1.1 303"), "{redirect}");
        assert!(redirect.contains("Location: /login"), "{redirect}");

        let login = get(server.port, "/login", None);
        assert!(login.starts_with("HTTP/1.1 200"), "{login}");
        assert!(login.contains("text/html"), "{login}");
        assert!(login.contains("<title>Jimmy</title>"), "{login}");

        let app = get(server.port, "/app.js", None);
        assert!(app.starts_with("HTTP/1.1 200"), "{app}");
        assert!(app.contains("javascript"), "{app}");
        assert!(app.contains("EventSource"), "{app}");

        let style = get(server.port, "/style.css", None);
        assert!(style.contains("text/css"), "{style}");

        let login = post(
            server.port,
            "/api/login",
            r#"{"user":"berti","password":"secreto"}"#,
        );
        let cookie = format!("jimmy_session={}", token(&login));
        let page = get(server.port, "/", Some(&cookie));
        assert!(page.starts_with("HTTP/1.1 200"), "{page}");
        assert!(page.contains("<div id=\"panes\">"), "{page}");
        std::fs::remove_dir_all(server.root.parent().unwrap()).unwrap();
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
    fn the_web_creates_and_writes_its_own_conversations() {
        let server = start("write");
        let login = post(
            server.port,
            "/api/login",
            r#"{"user":"berti","password":"secreto"}"#,
        );
        let cookie = format!("jimmy_session={}", token(&login));

        let created = post_with(
            server.port,
            "/api/conversations",
            r#"{"project":"ken","title":"una prueba"}"#,
            Some(&cookie),
        );
        assert!(created.starts_with("HTTP/1.1 200"), "{created}");
        let body: serde_json::Value =
            serde_json::from_str(created.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        let key = body["key"].as_str().unwrap().to_string();

        let state = get(server.port, "/api/state", Some(&cookie));
        assert!(state.contains(&key), "{state}");
        assert!(state.contains("una prueba"), "{state}");

        let renamed = post_with(
            server.port,
            "/api/rename",
            &format!(r#"{{"conversation":"{key}","title":"otro título"}}"#),
            Some(&cookie),
        );
        assert!(renamed.starts_with("HTTP/1.1 200"), "{renamed}");

        let sent = post_with(
            server.port,
            "/api/send",
            &format!(r#"{{"conversation":"{key}","text":"hola"}}"#),
            Some(&cookie),
        );
        assert!(sent.starts_with("HTTP/1.1 202"), "{sent}");

        let log = server
            .root
            .join("chats")
            .join(&key)
            .join("conversation.jsonl");
        let mut text = String::new();
        for _ in 0..200 {
            text = std::fs::read_to_string(&log).unwrap_or_default();
            if text.contains("\"done\"") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(text.contains("\"user\"") && text.contains("hola"), "{text}");
        assert!(text.contains("\"done\""), "{text}");

        let fresh = post_with(
            server.port,
            "/api/conversations",
            r#"{"project":"ken"}"#,
            Some(&cookie),
        );
        let body: serde_json::Value =
            serde_json::from_str(fresh.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        let fresh = body["key"].as_str().unwrap().to_string();
        post_with(
            server.port,
            "/api/send",
            &format!(r#"{{"conversation":"{fresh}","text":"decime los proyectos"}}"#),
            Some(&cookie),
        );
        let state = get(server.port, "/api/state", Some(&cookie));
        assert!(
            state.contains("\"title\":\"decime los proyectos\""),
            "el primer mensaje le pone nombre: {state}"
        );
        assert!(
            state.contains("\"title\":\"otro título\""),
            "y un nombre puesto a mano se respeta: {state}"
        );
        std::fs::remove_dir_all(server.root.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_conversation_that_comes_from_a_transport_is_not_written_from_the_web() {
        let server = start("readonly");
        let login = post(
            server.port,
            "/api/login",
            r#"{"user":"berti","password":"secreto"}"#,
        );

        let cookie = format!("jimmy_session={}", token(&login));
        let sent = post_with(
            server.port,
            "/api/send",
            r#"{"conversation":"7469057930","text":"hola"}"#,
            Some(&cookie),
        );
        assert!(sent.starts_with("HTTP/1.1 400"), "{sent}");
        let renamed = post_with(
            server.port,
            "/api/rename",
            r#"{"conversation":"7469057930","title":"mío"}"#,
            Some(&cookie),
        );
        assert!(renamed.starts_with("HTTP/1.1 400"), "{renamed}");
        assert!(!server.root.join("chats/7469057930/meta.json").exists());

        let created = post_with(
            server.port,
            "/api/conversations",
            r#"{"project":"nada"}"#,
            Some(&cookie),
        );
        assert!(created.starts_with("HTTP/1.1 400"), "{created}");
        let unauthenticated = post(server.port, "/api/conversations", r#"{"project":"ken"}"#);
        assert!(
            unauthenticated.starts_with("HTTP/1.1 401"),
            "{unauthenticated}"
        );
        assert!(login.starts_with("HTTP/1.1 200"));
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
