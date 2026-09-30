//! The web frontend: the same conversations the transports have, served to a
//! browser over HTTP and SSE.
//!
//! What arrives through a transport is watched, not written; the conversations
//! the web creates are its own and can be written from here.

use crate::agent::Agent;
use crate::auth::{self, Auth};
use crate::bus::Bus;
use crate::conversations;
use crate::files;
use crate::http::{self, Request};
use crate::log::{Log, Window};
use crate::machine;
use crate::media;
use crate::preview::{self, Previews};
use crate::protocol::Event;
use crate::schedule;
use crate::transport::Null;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;

/// El final del backlog: lo de antes ya está en pantalla. Lleva cuántos eventos
/// tiene el log, para que el que mira guarde el número y al volver a la
/// conversación pida desde ahí en vez de bajar todo de nuevo.
fn synced(total: usize, first: usize) -> String {
    format!(r#"{{"event":"synced","count":{total},"first":{first}}}"#)
}

const INDEX: &str = include_str!("../web/index.html");
const LOGIN: &str = include_str!("../web/login.html");
const THEME: &str = include_str!("../web/theme.css");
const STYLE: &str = include_str!("../web/style.css");
const APP: &str = include_str!("../web/app.js");
const FILES: &str = include_str!("../web/files.js");
const AGENDA: &str = include_str!("../web/agenda.js");
const PROJECT: &str = include_str!("../web/project.js");
const MARKDOWN: &str = include_str!("../web/markdown.js");
const MACHINE: &str = include_str!("../web/machine.js");
const TOOL: &str = include_str!("../web/tool.js");
const AVATAR: &str = include_str!("../web/avatar.js");
const AVATAR_STYLE: &str = include_str!("../web/avatar.css");
const ICON: &str = include_str!("../web/icon.svg");
const ICON_192: &[u8] = include_bytes!("../web/icon-192.png");
const ICON_512: &[u8] = include_bytes!("../web/icon-512.png");
const MANIFEST: &str = include_str!("../web/manifest.webmanifest");

pub struct Web {
    root: PathBuf,
    workspace: PathBuf,
    bus: Arc<Bus>,
    agent: Agent,
    auth: Auth,
    previews: Arc<Previews>,
    agenda: Sender<String>,
}

impl Web {
    pub fn new(
        root: PathBuf,
        workspace: PathBuf,
        bus: Arc<Bus>,
        agent: Agent,
        auth: Auth,
        previews: Arc<Previews>,
        agenda: Sender<String>,
    ) -> Arc<Web> {
        Arc::new(Web {
            root,
            workspace,
            bus,
            agent,
            auth,
            previews,
            agenda,
        })
    }

    /// El nombre para mostrar: la parte del mail antes del arroba.
    fn user(&self, request: &Request) -> Option<String> {
        Some(user_name(self.auth.user(&request.cookie(auth::COOKIE)?)?))
    }

    fn user_head(&self, head: &preview::Head) -> Option<String> {
        Some(user_name(self.auth.user(&head.cookie(auth::COOKIE)?)?))
    }

    fn base_url(&self, request: &Request) -> String {
        if let Some(url) = crate::env("JIMMY_WEB_URL") {
            return url.trim_end_matches('/').to_string();
        }
        format!("https://{}", request.header("host").unwrap_or("localhost"))
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

fn user_name(email: String) -> String {
    match email.split_once('@') {
        Some((name, _)) => name.to_string(),
        None => email,
    }
}

fn handle(web: &Arc<Web>, stream: &mut TcpStream) -> std::io::Result<()> {
    if preview::wants_preview(stream)? {
        return preview_page(web, stream);
    }
    let Some(request) = http::read(stream)? else {
        return Ok(());
    };
    if request.too_large {
        return http::send_error(stream, 413, "eso es demasiado grande");
    }
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/") => app_page(web, &request, stream),
        ("GET", "/login") => http::send_text(stream, 200, HTML, &versioned(LOGIN)),
        ("GET", "/theme.css") => asset(stream, CSS, versioned(THEME).as_bytes()),
        ("GET", "/style.css") => asset(stream, CSS, STYLE.as_bytes()),
        ("GET", "/avatar.css") => asset(stream, CSS, AVATAR_STYLE.as_bytes()),
        ("GET", "/app.js") => asset(stream, JS, APP.as_bytes()),
        ("GET", "/files.js") => asset(stream, JS, FILES.as_bytes()),
        ("GET", "/agenda.js") => asset(stream, JS, AGENDA.as_bytes()),
        ("GET", "/project.js") => asset(stream, JS, PROJECT.as_bytes()),
        ("GET", "/markdown.js") => asset(stream, JS, MARKDOWN.as_bytes()),
        ("GET", "/machine.js") => asset(stream, JS, MACHINE.as_bytes()),
        ("GET", "/tool.js") => asset(stream, JS, TOOL.as_bytes()),
        ("GET", "/avatar.js") => asset(stream, JS, AVATAR.as_bytes()),
        ("GET", "/icon.svg") => asset(stream, "image/svg+xml", ICON.as_bytes()),
        ("GET", "/icon-192.png") => http::respond(stream, 200, "image/png", &[], ICON_192),
        ("GET", "/icon-512.png") => http::respond(stream, 200, "image/png", &[], ICON_512),
        ("GET", "/manifest.webmanifest") => asset(
            stream,
            "application/manifest+json",
            versioned(MANIFEST).as_bytes(),
        ),
        ("POST", "/api/login") => login(web, &request, stream),
        ("GET", "/auth") => auth_link(web, &request, stream),
        ("POST", "/api/logout") => logout(web, &request, stream),
        ("GET", "/api/state") => state(web, &request, stream),
        ("GET", "/api/stream") => events(web, &request, stream),
        ("GET", "/api/history") => history(web, &request, stream),
        ("GET", "/api/online") => online(web, &request, stream),
        ("GET", "/api/search") => search(web, &request, stream),
        ("POST", "/api/conversations") => create(web, &request, stream),
        ("POST", "/api/projects") => create_project(web, &request, stream),
        ("POST", "/api/rename") => rename(web, &request, stream),
        ("POST", "/api/send") => send(web, &request, stream),
        ("POST", "/api/upload") => upload(web, &request, stream),
        ("GET", "/api/file") => file(web, &request, stream),
        ("GET", "/api/tree") => tree(web, &request, stream),
        ("GET", "/api/raw") => raw(web, &request, stream),
        ("POST", "/api/typing") => typing(web, &request, stream),
        ("POST", "/api/cancel") => cancel(web, &request, stream),
        ("GET", "/api/agenda") => agenda(web, &request, stream),
        ("POST", "/api/agenda/run") => agenda_run(web, &request, stream),
        ("POST", "/api/agenda/pause") => agenda_pause(web, &request, stream),
        ("POST", "/api/agenda/read") => agenda_read(web, &request, stream),
        ("POST", "/api/rename-project") => rename_project(web, &request, stream),
        ("POST", "/api/duplicate-project") => duplicate_project(web, &request, stream),
        ("POST", "/api/delete-conversation") => delete_conversation(web, &request, stream),
        ("POST", "/api/delete-project") => delete_project(web, &request, stream),
        ("POST", "/api/preview/stop") => stop_preview(web, &request, stream),
        _ => http::send_text(stream, 404, "text/plain", "no está"),
    }
}

const HTML: &str = "text/html; charset=utf-8";
const TEXT: &str = "text/plain; charset=utf-8";

/// Un preview se sirve crudo: la cabecera se reenvía sin tocarla y lo que sigue
/// se copia tal cual, así el streaming y el WebSocket de un HMR pasan igual.
fn preview_page(web: &Arc<Web>, stream: &mut TcpStream) -> std::io::Result<()> {
    let Some(head) = preview::read_head(stream)? else {
        return Ok(());
    };
    if web.user_head(&head).is_none() {
        return http::respond(stream, 303, TEXT, &[("Location", "/login")], b"");
    }
    let Some((name, _)) = preview::name_from_path(&head.target) else {
        return http::send_text(stream, 404, TEXT, "no está");
    };
    let Some(port) = web.previews.port(name) else {
        let log = web.previews.output(name);
        let body = if log.is_empty() {
            format!("{name} no está corriendo\n")
        } else {
            format!("{name} no está corriendo\n\n{log}\n")
        };
        return http::send_text(stream, 404, TEXT, &body);
    };
    web.previews.touch(name);
    preview::forward(stream, &head, name, port)
}

fn stop_preview(web: &Arc<Web>, request: &Request, stream: &mut TcpStream) -> std::io::Result<()> {
    if web.user(request).is_none() {
        return http::send_error(stream, 401, "no estás adentro");
    }
    let order = preview::Order {
        op: "stop".into(),
        name: request.field("name").unwrap_or_default(),
        ..preview::Order::default()
    };
    match preview::call(&web.workspace, &order) {
        Ok(answer) if answer.ok => http::send_json(stream, 200, &serde_json::json!({ "ok": true })),
        Ok(answer) => http::send_error(
            stream,
            400,
            &answer
                .error
                .unwrap_or_else(|| "no pude parar el preview".into()),
        ),
        Err(error) => http::send_error(stream, 503, &error),
    }
}
const CSS: &str = "text/css; charset=utf-8";
const JS: &str = "text/javascript; charset=utf-8";
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// El HTML y los íconos se piden por su nombre pelado, así que revalidan siempre.
/// Todo lo que sale con la versión en la URL se puede guardar para siempre: un
/// deploy cambia la versión y con ella la URL.
fn asset(stream: &mut TcpStream, content_type: &str, body: &[u8]) -> std::io::Result<()> {
    http::respond(
        stream,
        200,
        content_type,
        &[("Cache-Control", IMMUTABLE)],
        body,
    )
}

fn app_page(web: &Arc<Web>, request: &Request, stream: &mut TcpStream) -> std::io::Result<()> {
    if web.user(request).is_none() {
        return http::respond(stream, 303, "text/plain", &[("Location", "/login")], b"");
    }
    http::send_text(stream, 200, HTML, &versioned(INDEX))
}

/// Los assets llevan la versión en la URL: cada deploy cambia la URL, así que ni
/// un proxy ni el navegador pueden seguir sirviendo los viejos.
fn versioned(page: &str) -> String {
    let version = crate::env("JIMMY_COMMIT_SHA")
        .or_else(|| crate::env("RAILWAY_GIT_COMMIT_SHA"))
        .unwrap_or_default();
    page.replace("/app.js", &format!("/app.js?v={version}"))
        .replace("/files.js", &format!("/files.js?v={version}"))
        .replace("/machine.js", &format!("/machine.js?v={version}"))
        .replace("/agenda.js", &format!("/agenda.js?v={version}"))
        .replace("/project.js", &format!("/project.js?v={version}"))
        .replace("/markdown.js", &format!("/markdown.js?v={version}"))
        .replace("/tool.js", &format!("/tool.js?v={version}"))
        .replace("/avatar.js", &format!("/avatar.js?v={version}"))
        .replace("/theme.css", &format!("/theme.css?v={version}"))
        .replace("/style.css", &format!("/style.css?v={version}"))
        .replace("/avatar.css", &format!("/avatar.css?v={version}"))
        .replace("/icon.svg", &format!("/icon.svg?v={version}"))
        .replace("/icon-192.png", &format!("/icon-192.png?v={version}"))
        .replace(
            "/manifest.webmanifest",
            &format!("/manifest.webmanifest?v={version}"),
        )
}

/// Pedir el link. La respuesta es siempre la misma, esté o no el mail en la
/// lista: si no está, no se manda nada y desde afuera no se nota.
fn login(web: &Arc<Web>, request: &Request, stream: &mut TcpStream) -> std::io::Result<()> {
    let email = request.field("email").unwrap_or_default();
    let sent = serde_json::json!({ "sent": true });
    let Some(token) = web.auth.request_link(&email) else {
        return http::send_json(stream, 200, &sent);
    };
    let link = format!("{}/auth?token={token}", web.base_url(request));
    let Some(mail) = &web.auth.mail else {
        if !web.auth.dev {
            eprintln!("jimmy web: sin proveedor de mail, no puedo mandarle el link a {email}");
            return http::send_json(stream, 200, &sent);
        }
        eprintln!("jimmy web: sin proveedor de mail, el link para {email} es {link}");
        let body = serde_json::json!({ "sent": true, "link": link });
        return http::send_json(stream, 200, &body);
    };
    if let Err(e) = mail.send_link(&email, &link) {
        eprintln!("jimmy web: no pude mandar el mail a {email}: {e}");
        return http::send_error(stream, 502, "no pude mandar el mail");
    }
    http::send_json(stream, 200, &sent)
}

/// El link del mail: sirve una vez, deja la cookie y manda al principio.
fn auth_link(web: &Arc<Web>, request: &Request, stream: &mut TcpStream) -> std::io::Result<()> {
    let Some(token) = request.param("token") else {
        return http::send_error(stream, 400, "falta el token");
    };
    let Some(email) = web.auth.consume_link(token) else {
        return http::respond(
            stream,
            303,
            "text/plain",
            &[("Location", "/login?error=1")],
            b"",
        );
    };
    let session = web.auth.open_session(&email);
    let cookie = format!(
        "{}={session}; Path=/; HttpOnly; SameSite=Lax; Secure; Max-Age={}",
        auth::COOKIE,
        auth::SESSION_TTL
    );
    let headers = [("Set-Cookie", cookie.as_str()), ("Location", "/")];
    http::respond(stream, 303, "text/plain", &headers, b"")
}

fn logout(web: &Arc<Web>, request: &Request, stream: &mut TcpStream) -> std::io::Result<()> {
    if let Some(token) = request.cookie(auth::COOKIE) {
        web.auth.close_session(&token);
    }
    let cookie = format!(
        "{}=; Path=/; HttpOnly; SameSite=Lax; Secure; Max-Age=0",
        auth::COOKIE
    );
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
                        "project": conversation.project,
                        "title": conversation.title,
                        "read_only": conversation.read_only,
                        "running": web.agent.running(&conversation.key),
                    })
                })
                .collect();
            serde_json::json!({
                "name": project.name,
                "path": project.path.display().to_string(),
                "unversioned": unversioned(&project.path),
                "size": match project.name == conversations::GENERAL {
                    true => 0,
                    false => conversations::size(&project.path),
                },
                "last": project
                    .conversations
                    .iter()
                    .find_map(|conversation| conversations::last_message(&conversation.dir)),
                "conversations": conversations,
            })
        })
        .collect();
    let mut previews: Vec<preview::Summary> = web.previews.list();
    previews.sort_by(|a, b| a.name.cmp(&b.name));
    let previews: Vec<serde_json::Value> = previews
        .into_iter()
        .map(|preview| {
            serde_json::json!({
                "name": preview.name,
                "port": preview.port,
                "seconds": preview.seconds,
                "path": format!("/preview/{}/", preview.name),
            })
        })
        .collect();
    http::send_json(
        stream,
        200,
        &serde_json::json!({
            "user": user,
            "workspace": web.workspace.display().to_string(),
            "machine": machine::usage(&web.root),
            "projects": projects,
            "previews": previews,
        }),
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

fn delete_conversation(
    web: &Arc<Web>,
    request: &Request,
    stream: &mut TcpStream,
) -> std::io::Result<()> {
    if web.user(request).is_none() {
        return http::send_error(stream, 401, "no estás adentro");
    }
    let key = request.field("conversation").unwrap_or_default();
    if writable(web, &key).is_err() {
        return http::send_error(stream, 400, "esa conversación no se escribe desde acá");
    }
    match web.agent.delete(&key) {
        Ok(()) => http::send_json(stream, 200, &serde_json::json!({ "deleted": true })),
        Err(error) => http::send_error(stream, 500, &error),
    }
}

fn rename_project(
    web: &Arc<Web>,
    request: &Request,
    stream: &mut TcpStream,
) -> std::io::Result<()> {
    if web.user(request).is_none() {
        return http::send_error(stream, 401, "no estás adentro");
    }
    let from = request.field("project").unwrap_or_default();
    let to = request.field("name").unwrap_or_default();
    let to = to.trim();
    if to.is_empty() || to.contains('/') || to.starts_with('.') {
        return http::send_error(stream, 400, "ese nombre no sirve para un proyecto");
    }
    match conversations::rename_project(&web.root, &web.workspace, from.trim(), to) {
        Ok(()) => http::send_json(stream, 200, &serde_json::json!({ "name": to })),
        Err(error) => http::send_error(stream, 400, &error),
    }
}

fn duplicate_project(
    web: &Arc<Web>,
    request: &Request,
    stream: &mut TcpStream,
) -> std::io::Result<()> {
    if web.user(request).is_none() {
        return http::send_error(stream, 401, "no estás adentro");
    }
    let from = request.field("project").unwrap_or_default();
    let to = request.field("name").unwrap_or_default();
    let to = to.trim();
    if to.is_empty() || to.contains('/') || to.starts_with('.') {
        return http::send_error(stream, 400, "ese nombre no sirve para un proyecto");
    }
    match conversations::duplicate(&web.workspace, from.trim(), to) {
        Ok(()) => http::send_json(stream, 200, &serde_json::json!({ "name": to })),
        Err(error) => http::send_error(stream, 400, &error),
    }
}

fn delete_project(
    web: &Arc<Web>,
    request: &Request,
    stream: &mut TcpStream,
) -> std::io::Result<()> {
    if web.user(request).is_none() {
        return http::send_error(stream, 401, "no estás adentro");
    }
    let name = request.field("project").unwrap_or_default();
    let name = name.trim();
    if name.is_empty()
        || name == conversations::GENERAL
        || name.contains('/')
        || name.starts_with('.')
    {
        return http::send_error(stream, 400, "ese nombre no es un proyecto");
    }
    let dir = web.workspace.join("projects").join(name);
    if !dir.is_dir() {
        return http::send_error(stream, 400, "ese proyecto no existe");
    }
    if let Err(why) = disposable(&dir, request.flag("force")) {
        return http::send_error(stream, 400, &format!("no lo borro: {why}"));
    }
    let conversations = conversations::projects(&web.root, &web.workspace)
        .into_iter()
        .find(|project| project.name == name)
        .map(|project| project.conversations)
        .unwrap_or_default();
    for conversation in conversations {
        let _ = web.agent.delete(&conversation.key);
    }
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => http::send_json(stream, 200, &serde_json::json!({ "deleted": true })),
        Err(error) => http::send_error(stream, 500, &error.to_string()),
    }
}

/// Un proyecto se borra si está vacío o si es un clon con todo commiteado y
/// pusheado. Lo que no está en git se borra solo si el pedido se hace cargo
/// (`force`): puede tener trabajo adentro que no existe en ningún otro lado.
fn disposable(dir: &Path, force: bool) -> Result<(), String> {
    if empty(dir) {
        return Ok(());
    }
    if unversioned(dir) {
        return match force {
            true => Ok(()),
            false => Err("tiene archivos que no están en git".into()),
        };
    }
    if !git(dir, &["status", "--porcelain"])?.trim().is_empty() {
        return Err("tiene cambios sin commitear".into());
    }
    if !git(
        dir,
        &["log", "--branches", "--not", "--remotes", "--oneline"],
    )?
    .trim()
    .is_empty()
    {
        return Err("tiene commits sin pushear".into());
    }
    Ok(())
}

/// Un proyecto sin git y con algo adentro: no hay copia en ningún otro lado,
/// así que borrarlo es una decisión del que lo pide.
fn unversioned(dir: &Path) -> bool {
    !empty(dir) && !dir.join(".git").exists()
}

fn empty(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|entries| entries.count() == 0)
        .unwrap_or(true)
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err("no pude preguntarle a git".into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn search(web: &Arc<Web>, request: &Request, stream: &mut TcpStream) -> std::io::Result<()> {
    if web.user(request).is_none() {
        return http::send_error(stream, 401, "no estás adentro");
    }
    let needle = request.param("q").unwrap_or_default();
    let results: Vec<serde_json::Value> = web
        .agent
        .search(needle, 30)
        .into_iter()
        .map(|hit| {
            serde_json::json!({
                "conversation": hit.conversation,
                "title": hit.title,
                "project": hit.project,
                "role": hit.role,
                "snippet": hit.snippet,
            })
        })
        .collect();
    http::send_json(stream, 200, &serde_json::json!({ "results": results }))
}

fn cancel(web: &Arc<Web>, request: &Request, stream: &mut TcpStream) -> std::io::Result<()> {
    let Some(user) = web.user(request) else {
        return http::send_error(stream, 401, "no estás adentro");
    };
    let key = request.field("conversation").unwrap_or_default();
    if writable(web, &key).is_err() {
        return http::send_error(stream, 400, "esa conversación no se escribe desde acá");
    }
    let Some(session) = crate::session_from_key(&key) else {
        return http::send_error(stream, 400, "clave de conversación inválida");
    };
    web.agent.cancel(&session, &user);
    http::send_json(stream, 200, &serde_json::json!({ "cancelled": true }))
}

/// Un adjunto no puede pesar más que una foto de Telegram.
const MAX_UPLOAD: usize = 10 * 1024 * 1024;

fn send(web: &Arc<Web>, request: &Request, stream: &mut TcpStream) -> std::io::Result<()> {
    let Some(user) = web.user(request) else {
        return http::send_error(stream, 401, "no estás adentro");
    };
    let key = request.field("conversation").unwrap_or_default();
    let text = request.field("text").unwrap_or_default();
    let Ok(conversation) = writable(web, &key) else {
        return http::send_error(stream, 400, "esa conversación no se escribe desde acá");
    };
    let images = match read_attachments(&conversation, &request.list("images")) {
        Ok(images) => images,
        Err(error) => return http::send_error(stream, 400, &error),
    };
    if text.trim().is_empty() && images.is_empty() {
        return http::send_error(stream, 400, "el mensaje está vacío");
    }
    if conversation.title.is_none()
        || conversation.title.as_deref() == Some(conversations::NEW_TITLE)
    {
        let title = if text.trim().is_empty() {
            "imagen".to_string()
        } else {
            title_from(&text)
        };
        let _ = conversations::rename(&web.root, &key, &title);
    }
    let Some(session) = crate::session_from_key(&key) else {
        return http::send_error(stream, 400, "clave de conversación inválida");
    };

    let web = web.clone();
    std::thread::spawn(move || {
        if let Err(error) = web.agent.respond(&Null, &session, &text, images, &user) {
            eprintln!("jimmy web: {error}");
        }
    });
    http::send_json(stream, 202, &serde_json::json!({ "started": true }))
}

/// Los adjuntos ya subidos, leídos del disco: el mensaje viaja con los bytes del
/// archivo, no con su nombre.
fn read_attachments(
    conversation: &conversations::Conversation,
    names: &[String],
) -> Result<Vec<axe::Image>, String> {
    let uploads = media::dir(conversation);
    names
        .iter()
        .map(|name| {
            let name = media::safe_name(name).ok_or_else(|| "ese adjunto no sirve".to_string())?;
            axe::image::attach(&uploads.join(name).display().to_string())
        })
        .collect()
}

/// Los bytes crudos del adjunto, con el nombre aparte en la query: el cuerpo es
/// el archivo, no lo envuelve ningún JSON.
fn upload(web: &Arc<Web>, request: &Request, stream: &mut TcpStream) -> std::io::Result<()> {
    if web.user(request).is_none() {
        return http::send_error(stream, 401, "no estás adentro");
    }
    let key = request.param("conversation").unwrap_or_default();
    let Ok(conversation) = writable(web, key) else {
        return http::send_error(stream, 400, "esa conversación no se escribe desde acá");
    };
    if request.body.is_empty() {
        return http::send_error(stream, 400, "el archivo está vacío");
    }
    if request.body.len() > MAX_UPLOAD {
        return http::send_error(stream, 400, "ese archivo es muy grande");
    }
    let name = match media::store(
        &media::dir(&conversation),
        request.param("name").unwrap_or_default(),
        &request.body,
    ) {
        Ok(name) => name,
        Err(error) => return http::send_error(stream, 500, &error),
    };
    http::send_json(stream, 200, &serde_json::json!({ "name": name }))
}

/// Un adjunto de la conversación. Cualquiera adentro puede mirarlo: es lo que
/// hay en el chat.
fn file(web: &Arc<Web>, request: &Request, stream: &mut TcpStream) -> std::io::Result<()> {
    if web.user(request).is_none() {
        return http::send_error(stream, 401, "no estás adentro");
    }
    let key = request.param("conversation").unwrap_or_default();
    let conversation = conversations::get(&web.root, &web.workspace, key);
    let Some(name) = request.param("name").and_then(media::safe_name) else {
        return http::send_error(stream, 400, "ese nombre no sirve");
    };
    let Ok(data) = std::fs::read(media::dir(&conversation).join(name)) else {
        return http::send_error(stream, 404, "no está");
    };
    http::respond(stream, 200, media::content_type(name), &[], &data)
}

/// El directorio de un proyecto, o nada si ese nombre no puede ser uno. El
/// proyecto `general` es el workspace entero: ahí se ve todo lo que hay.
fn project_dir(web: &Web, name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains('/') || name.starts_with('.') {
        return None;
    }
    let dir = conversations::project_dir(&web.workspace, name);
    dir.is_dir().then_some(dir)
}

/// Una carpeta del proyecto, un nivel. Cada carpeta la pide el que mira cuando
/// la abre.
fn tree(web: &Arc<Web>, request: &Request, stream: &mut TcpStream) -> std::io::Result<()> {
    if web.user(request).is_none() {
        return http::send_error(stream, 401, "no estás adentro");
    }
    let project = request.param("project").unwrap_or_default();
    let Some(root) = project_dir(web, project) else {
        return http::send_error(stream, 400, "ese proyecto no existe");
    };
    let path = request.param("path").unwrap_or_default();
    let Some(entries) = files::list(&root, path) else {
        return http::send_error(stream, 404, "esa carpeta no está");
    };
    let entries: Vec<serde_json::Value> = entries
        .into_iter()
        .map(|entry| {
            serde_json::json!({
                "name": entry.name,
                "dir": entry.dir,
                "size": entry.size,
                "kind": entry.kind,
            })
        })
        .collect();
    http::send_json(
        stream,
        200,
        &serde_json::json!({ "project": project, "path": path, "entries": entries }),
    )
}

/// Un archivo del proyecto. Las imágenes van enteras porque el navegador las
/// muestra; el texto va cortado en el tope y lo avisa; lo que no es texto sale
/// como binario, para que el navegador no lo muestre como si fuera una página.
fn raw(web: &Arc<Web>, request: &Request, stream: &mut TcpStream) -> std::io::Result<()> {
    if web.user(request).is_none() {
        return http::send_error(stream, 401, "no estás adentro");
    }
    let Some(root) = project_dir(web, request.param("project").unwrap_or_default()) else {
        return http::send_error(stream, 400, "ese proyecto no existe");
    };
    let Some(path) = files::resolve(&root, request.param("path").unwrap_or_default()) else {
        return http::send_error(stream, 404, "ese archivo no está");
    };
    let Ok(meta) = std::fs::metadata(&path) else {
        return http::send_error(stream, 404, "ese archivo no está");
    };
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return http::send_error(stream, 404, "ese archivo no está");
    };
    if !meta.is_file() {
        return http::send_error(stream, 404, "eso no es un archivo");
    }
    let image = media::is_image(name);
    let bytes = match files::read(&path, (!image).then_some(files::MAX_READ)) {
        Ok(bytes) => bytes,
        Err(error) => return http::send_error(stream, 500, &error),
    };
    let content_type = if image {
        media::content_type(name)
    } else if files::is_text(&bytes) {
        "text/plain; charset=utf-8"
    } else {
        "application/octet-stream"
    };
    let cut: &[(&str, &str)] = if meta.len() > bytes.len() as u64 {
        &[("X-Truncated", "1")]
    } else {
        &[]
    };
    http::respond(stream, 200, content_type, cut, &bytes)
}

fn typing(web: &Arc<Web>, request: &Request, stream: &mut TcpStream) -> std::io::Result<()> {
    let Some(user) = web.user(request) else {
        return http::send_error(stream, 401, "no estás adentro");
    };
    let key = request.field("conversation").unwrap_or_default();
    if writable(web, &key).is_err() {
        return http::send_error(stream, 400, "esa conversación no se escribe desde acá");
    }
    web.bus.show(&key, &Event::Typing { user });
    http::send_json(stream, 200, &serde_json::json!({ "typing": true }))
}

/// Cuántas corridas de cada tarea se le muestran a la web: el archivo guarda
/// muchas más, la vista muestra las últimas.
const SHOWN: usize = 5;

fn agenda_dir(web: &Web) -> PathBuf {
    web.workspace.join("state").join("schedule")
}

fn agenda(web: &Arc<Web>, request: &Request, stream: &mut TcpStream) -> std::io::Result<()> {
    if web.user(request).is_none() {
        return http::send_error(stream, 401, "no estás adentro");
    }
    let tasks: Vec<serde_json::Value> = schedule::list(&agenda_dir(web))
        .into_iter()
        .map(|entry| {
            let shown: Vec<&schedule::Run> = entry.runs.iter().rev().take(SHOWN).collect();
            serde_json::json!({
                "name": entry.name,
                "when": entry.task.when,
                "at": entry.task.at,
                "every": entry.task.every,
                "target": entry.task.target,
                "silent": entry.task.silent,
                "paused": entry.task.paused,
                "unread": entry.unread,
                "runs": shown,
            })
        })
        .collect();
    http::send_json(stream, 200, &serde_json::json!({ "tasks": tasks }))
}

fn agenda_run(web: &Arc<Web>, request: &Request, stream: &mut TcpStream) -> std::io::Result<()> {
    if web.user(request).is_none() {
        return http::send_error(stream, 401, "no estás adentro");
    }
    let name = request.field("name").unwrap_or_default();
    if !schedule::list(&agenda_dir(web))
        .iter()
        .any(|task| task.name == name)
    {
        return http::send_error(stream, 404, "esa tarea no existe");
    }
    if web.agenda.send(name).is_err() {
        return http::send_error(stream, 500, "el scheduler no está corriendo");
    }
    http::send_json(stream, 200, &serde_json::json!({ "queued": true }))
}

fn agenda_pause(web: &Arc<Web>, request: &Request, stream: &mut TcpStream) -> std::io::Result<()> {
    if web.user(request).is_none() {
        return http::send_error(stream, 401, "no estás adentro");
    }
    let name = request.field("name").unwrap_or_default();
    let paused = request.flag("paused");
    match schedule::set_paused(&agenda_dir(web), &name, paused) {
        Ok(()) => http::send_json(stream, 200, &serde_json::json!({ "paused": paused })),
        Err(e) => http::send_error(stream, 400, &e),
    }
}

/// Marca leída una tarea, o todas si no viene el nombre.
fn agenda_read(web: &Arc<Web>, request: &Request, stream: &mut TcpStream) -> std::io::Result<()> {
    if web.user(request).is_none() {
        return http::send_error(stream, 401, "no estás adentro");
    }
    let name = request.field("name");
    match schedule::mark_read(&agenda_dir(web), name.as_deref()) {
        Ok(()) => http::send_json(stream, 200, &serde_json::json!({ "read": true })),
        Err(e) => http::send_error(stream, 400, &e),
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
    let Some(user) = web.user(request) else {
        return http::send_error(stream, 401, "no estás adentro");
    };
    let Some(key) = request.param("conversation") else {
        return http::send_error(stream, 400, "falta conversation");
    };
    let conversation = conversations::get(&web.root, &web.workspace, key);
    if !conversation.dir.is_dir() {
        return http::send_error(stream, 404, "esa conversación no existe");
    }
    let since = request
        .param("since")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let window = Log::in_dir(&conversation.dir).window(usize::MAX);
    let (id, live) = web.bus.attach(key, &user);
    let result = follow(stream, &window, since, &live);
    web.bus.detach(key, id);
    result
}

/// Los eventos anteriores al tramo que el cliente tiene: con eso se desplaza
/// hacia arriba en el hilo sin volver a bajar lo que ya vio.
fn history(web: &Arc<Web>, request: &Request, stream: &mut TcpStream) -> std::io::Result<()> {
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
    let before = request
        .param("before")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let window = Log::in_dir(&conversation.dir).window(before);
    http::send_json(
        stream,
        200,
        &serde_json::json!({ "events": window.events, "first": window.first }),
    )
}

fn online(web: &Arc<Web>, request: &Request, stream: &mut TcpStream) -> std::io::Result<()> {
    let Some(user) = web.user(request) else {
        return http::send_error(stream, 401, "no estás adentro");
    };
    let (id, live) = web.bus.watch(&user);
    let result = follow(stream, &Window::default(), 0, &live);
    web.bus.detach_watch(id);
    result
}

fn follow(
    stream: &mut TcpStream,
    window: &Window,
    since: usize,
    live: &std::sync::mpsc::Receiver<crate::protocol::Event>,
) -> std::io::Result<()> {
    http::sse_open(stream)?;
    for event in window
        .events
        .iter()
        .skip(since.saturating_sub(window.first))
    {
        if let Ok(json) = serde_json::to_string(event) {
            http::sse_data(stream, &json)?;
        }
    }
    http::sse_data(stream, &synced(window.total, window.first))?;
    loop {
        match live.recv_timeout(Duration::from_secs(25)) {
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
        workspace: PathBuf,
        port: u16,
        bus: Arc<Bus>,
        previews: Arc<crate::preview::Previews>,
        agenda: std::sync::mpsc::Receiver<String>,
    }

    fn start(tag: &str) -> Server {
        start_with(tag, true)
    }

    fn start_with(tag: &str, dev: bool) -> Server {
        use std::os::unix::fs::PermissionsExt;

        let base = std::env::temp_dir().join(format!("jimmy-web-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("root");
        let workspace = base.join("workspace");
        std::fs::create_dir_all(root.join("chats/123456789")).unwrap();
        std::fs::create_dir_all(workspace.join("projects/ken")).unwrap();
        std::fs::write(
            root.join("chats/123456789/conversation.jsonl"),
            "{\"event\":\"user\",\"text\":\"hola\"}\n{\"event\":\"done\",\"text\":\"listo\"}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("chats/123456789/transcript.jsonl"),
            "{\"type\":\"message\",\"message\":{\"Role\":\"user\",\"Content\":\"el gato duerme\"}}\n",
        )
        .unwrap();

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
        let auth = Auth::new("bob@ejemplo.com, ana@ejemplo.com", &root, None, dev);
        let previews = crate::preview::Previews::new(&workspace);
        let (agenda, runner) = std::sync::mpsc::channel();
        let web = Web::new(
            root.clone(),
            workspace.clone(),
            bus.clone(),
            agent,
            auth,
            previews.clone(),
            agenda,
        );
        let listener = listen(0).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || serve(web, listener));
        Server {
            root,
            workspace,
            port,
            bus,
            previews,
            agenda: runner,
        }
    }

    fn connect(port: u16) -> TcpStream {
        TcpStream::connect(("127.0.0.1", port)).unwrap()
    }

    fn whole(mut stream: TcpStream) -> String {
        let mut out = Vec::new();
        stream.read_to_end(&mut out).unwrap();
        // Un adjunto vuelve como bytes, y para mirar el encabezado alcanza.
        String::from_utf8_lossy(&out).to_string()
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

    /// Un cuerpo crudo, como el que manda el navegador con el archivo adjunto.
    fn post_bytes(port: u16, path: &str, body: &[u8], cookie: Option<&str>) -> String {
        let mut stream = connect(port);
        let cookie = cookie
            .map(|c| format!("Cookie: {c}\r\n"))
            .unwrap_or_default();
        write!(
            stream,
            "POST {path} HTTP/1.1\r\nHost: jimmy\r\n{cookie}Content-Type: image/png\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(body).unwrap();
        whole(stream)
    }

    fn body_in(response: &str) -> String {
        response.split("\r\n\r\n").nth(1).unwrap().to_string()
    }

    fn json_in(response: &str) -> serde_json::Value {
        serde_json::from_str(&body_in(response)).unwrap()
    }

    fn weight(state: &str, project: &str) -> u64 {
        json_in(state)["projects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == project)
            .unwrap()["size"]
            .as_u64()
            .unwrap()
    }

    fn conversation(port: u16, cookie: &str) -> String {
        let created = post_with(
            port,
            "/api/conversations",
            r#"{"project":"ken"}"#,
            Some(cookie),
        );
        assert!(created.starts_with("HTTP/1.1 200"), "{created}");
        json_in(&created)["key"].as_str().unwrap().to_string()
    }

    /// El log lo escribe el hilo del turno, así que hay que esperarlo.
    fn wait_for(root: &Path, key: &str, needle: &str) -> String {
        let path = root.join("chats").join(key).join("conversation.jsonl");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            if text.contains(needle) || std::time::Instant::now() > deadline {
                return text;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// Pedir el link y seguirlo, como el que abre el mail. En las pruebas no
    /// hay proveedor, así que el link vuelve en la misma respuesta.
    fn login(port: u16, email: &str) -> String {
        let response = post(port, "/api/login", &format!(r#"{{"email":"{email}"}}"#));
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        let link = link_in(&response);
        let magic = link
            .split_once("token=")
            .expect("esperaba el token del link")
            .1;
        let followed = get(port, &format!("/auth?token={magic}"), None);
        assert!(followed.starts_with("HTTP/1.1 303"), "{followed}");
        assert!(followed.contains("Location: /"), "{followed}");
        format!("jimmy_session={}", token(&followed))
    }

    fn link_in(response: &str) -> String {
        let body: serde_json::Value =
            serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        body["link"].as_str().expect("esperaba el link").to_string()
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

        let page = get(server.port, "/login", None);
        assert!(page.starts_with("HTTP/1.1 200"), "{page}");
        assert!(page.contains("text/html"), "{page}");
        assert!(page.contains("<title>Jimmy</title>"), "{page}");

        let app = get(server.port, "/app.js", None);
        assert!(app.starts_with("HTTP/1.1 200"), "{app}");
        assert!(app.contains("javascript"), "{app}");
        assert!(app.contains("EventSource"), "{app}");
        assert!(
            app.contains(IMMUTABLE),
            "el asset va con la versión en la URL: cualquier otra caché lo revive viejo: {app}"
        );

        let style = get(server.port, "/style.css", None);
        assert!(style.contains("text/css"), "{style}");

        let theme = get(server.port, "/theme.css", None);
        assert!(theme.contains("text/css"), "{theme}");

        let markdown = get(server.port, "/markdown.js", None);
        assert!(markdown.starts_with("HTTP/1.1 200"), "{markdown}");
        assert!(markdown.contains("function markdown"), "{markdown}");

        let tool = get(server.port, "/tool.js", None);
        assert!(tool.starts_with("HTTP/1.1 200"), "{tool}");
        assert!(tool.contains("function describeTool"), "{tool}");

        let project = get(server.port, "/project.js", None);
        assert!(project.starts_with("HTTP/1.1 200"), "{project}");
        assert!(project.contains("function projectColor"), "{project}");

        let avatar = get(server.port, "/avatar.js", None);
        assert!(avatar.starts_with("HTTP/1.1 200"), "{avatar}");
        assert!(avatar.contains("function avatarState"), "{avatar}");

        let avatar_style = get(server.port, "/avatar.css", None);
        assert!(avatar_style.contains("text/css"), "{avatar_style}");

        let agenda = get(server.port, "/agenda.js", None);
        assert!(agenda.starts_with("HTTP/1.1 200"), "{agenda}");
        assert!(agenda.contains("function createAgendaTab"), "{agenda}");

        let icon = get(server.port, "/icon.svg", None);
        assert!(icon.contains("image/svg+xml"), "{icon}");
        assert!(icon.contains("<svg"), "{icon}");

        let manifest = get(server.port, "/manifest.webmanifest", None);
        assert!(manifest.contains("application/manifest+json"), "{manifest}");
        assert!(manifest.contains("\"standalone\""), "{manifest}");
        assert!(manifest.contains("icon-512.png"), "{manifest}");
        assert!(manifest.contains("\"share_target\""), "{manifest}");
        assert!(manifest.contains("\"method\": \"GET\""), "{manifest}");

        for size in [192, 512] {
            let png = get(server.port, &format!("/icon-{size}.png"), None);
            assert!(png.contains("image/png"), "{png}");
        }

        let cookie = login(server.port, "bob@ejemplo.com");
        let page = get(server.port, "/", Some(&cookie));
        assert!(page.starts_with("HTTP/1.1 200"), "{page}");
        assert!(
            page.contains("Cache-Control: no-store"),
            "el HTML no lleva versión, así que revalida siempre: {page}"
        );
        assert!(page.contains("<div id=\"panes\">"), "{page}");
        assert!(page.contains("/markdown.js?v="), "{page}");
        assert!(page.contains("/tool.js?v="), "{page}");
        assert!(page.contains("/app.js?v="), "{page}");
        assert!(page.contains("/files.js?v="), "{page}");
        assert!(page.contains("/project.js?v="), "{page}");
        assert!(
            page.find("/tool.js?v=") < page.find("/app.js?v="),
            "la app usa lo que define el tool: {page}"
        );
        assert!(
            page.find("/project.js?v=") < page.find("/app.js?v="),
            "el color del proyecto se carga antes que la app: {page}"
        );
        assert!(
            page.find("/files.js?v=") < page.find("/app.js?v="),
            "el explorador se carga antes que la app: {page}"
        );
        assert!(
            page.find("/markdown.js?v=") < page.find("/app.js?v="),
            "el markdown se carga antes que la app: {page}"
        );
        assert!(page.contains("/theme.css?v="), "{page}");
        assert!(page.contains("/style.css?v="), "{page}");
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn every_asset_in_the_page_carries_the_version() {
        let page = versioned(INDEX);
        let assets: Vec<&str> = page
            .split('"')
            .filter(|value| {
                value.starts_with('/') && (value.contains(".js") || value.contains(".css"))
            })
            .collect();
        assert!(
            assets
                .iter()
                .any(|asset| asset.starts_with("/machine.js?v=")),
            "{page}"
        );
        for asset in assets {
            assert!(
                asset.contains("?v="),
                "{asset} viaja sin versión: los assets van con caché immutable, así que el navegador lo revive viejo"
            );
        }
    }

    #[test]
    fn without_the_dev_flag_the_link_stays_out_of_the_response() {
        let server = start_with("plain", false);
        let answer = post(server.port, "/api/login", r#"{"email":"bob@ejemplo.com"}"#);
        assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
        assert!(
            !answer.contains("link"),
            "sin proveedor y sin flag, el link no sale de la respuesta: {answer}"
        );
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn the_session_cookie_is_http_only_secure_and_lasts_a_month() {
        let server = start("cookie");
        let answer = post(server.port, "/api/login", r#"{"email":"bob@ejemplo.com"}"#);
        let magic = link_in(&answer).rsplit('=').next().unwrap().to_string();
        let followed = get(server.port, &format!("/auth?token={magic}"), None);
        let set = followed
            .lines()
            .find(|line| line.starts_with("Set-Cookie"))
            .expect("esperaba la cookie");
        assert!(set.contains("HttpOnly"), "{set}");
        assert!(set.contains("SameSite=Lax"), "{set}");
        assert!(set.contains("Secure"), "{set}");
        assert!(
            set.contains(&format!("Max-Age={}", auth::SESSION_TTL)),
            "{set}"
        );

        let cookie = format!("jimmy_session={}", token(&followed));
        let out = post_with(server.port, "/api/logout", "{}", Some(&cookie));
        let gone = out
            .lines()
            .find(|line| line.starts_with("Set-Cookie"))
            .expect("esperaba la cookie de salida");
        assert!(gone.contains("Max-Age=0"), "{gone}");
        assert!(gone.contains("HttpOnly"), "{gone}");
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn a_preview_needs_a_session_and_says_what_is_missing() {
        let server = start("preview");

        let without = get(server.port, "/preview/loquesea/", None);
        assert!(without.starts_with("HTTP/1.1 303"), "{without}");
        assert!(without.contains("Location: /login"), "{without}");

        let cookie = login(server.port, "bob@ejemplo.com");
        let out = get(server.port, "/preview/loquesea/", Some(&cookie));
        assert!(out.starts_with("HTTP/1.1 404"), "{out}");
        assert!(out.contains("loquesea no está corriendo"), "{out}");

        let root = get(server.port, "/preview/", Some(&cookie));
        assert!(root.starts_with("HTTP/1.1 404"), "{root}");
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn a_preview_stops_from_the_api() {
        let server = start("preview-stop");
        std::thread::spawn({
            let previews = server.previews.clone();
            move || crate::preview::listen(previews)
        });
        let cookie = login(server.port, "bob@ejemplo.com");

        let order = crate::preview::Order {
            op: "start".into(),
            name: "ken".into(),
            command: Some("exec sleep 60".into()),
            ..crate::preview::Order::default()
        };
        let started = crate::preview::call(&server.workspace, &order).unwrap();
        assert!(started.ok, "{:?}", started.error);
        let running = json_in(&get(server.port, "/api/state", Some(&cookie)));
        assert_eq!(running["previews"][0]["name"], "ken");

        let without = post(server.port, "/api/preview/stop", r#"{"name":"ken"}"#);
        assert!(without.starts_with("HTTP/1.1 401"), "{without}");

        let stopped = post_with(
            server.port,
            "/api/preview/stop",
            r#"{"name":"ken"}"#,
            Some(&cookie),
        );
        assert!(stopped.starts_with("HTTP/1.1 200"), "{stopped}");
        let after = json_in(&get(server.port, "/api/state", Some(&cookie)));
        assert!(after["previews"].as_array().unwrap().is_empty(), "{after}");

        let again = post_with(
            server.port,
            "/api/preview/stop",
            r#"{"name":"ken"}"#,
            Some(&cookie),
        );
        assert!(again.starts_with("HTTP/1.1 400"), "{again}");
        assert!(again.contains("no está corriendo"), "{again}");
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn everything_but_logging_in_needs_a_session() {
        let server = start("auth");
        assert!(get(server.port, "/api/state", None).starts_with("HTTP/1.1 401"));
        assert!(get(server.port, "/api/agenda", None).starts_with("HTTP/1.1 401"));
        assert!(post(server.port, "/api/agenda/run", r#"{"name":"x"}"#).starts_with("HTTP/1.1 401"));
        assert!(post(
            server.port,
            "/api/agenda/pause",
            r#"{"name":"x","paused":true}"#
        )
        .starts_with("HTTP/1.1 401"));
        assert!(
            post(server.port, "/api/agenda/read", r#"{"name":"x"}"#).starts_with("HTTP/1.1 401")
        );
        let without = post(server.port, "/api/login", r#"{"email":"otro@ejemplo.com"}"#);
        assert!(without.starts_with("HTTP/1.1 200"), "{without}");
        assert!(
            !without.contains("link"),
            "a un mail de afuera no se le manda nada"
        );
        assert!(
            get(server.port, "/api/state", Some("jimmy_session=nada")).starts_with("HTTP/1.1 401")
        );
        assert!(get(server.port, "/nada", None).starts_with("HTTP/1.1 404"));
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn the_state_brings_the_last_answer_of_the_project() {
        let server = start("last");
        let cookie = login(server.port, "bob@ejemplo.com");
        let created = post_with(
            server.port,
            "/api/conversations",
            r#"{"project":"ken","title":"una charla"}"#,
            Some(&cookie),
        );
        let body: serde_json::Value =
            serde_json::from_str(created.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        let key = body["key"].as_str().unwrap().to_string();
        std::fs::write(
            server.root.join("chats").join(&key).join("transcript.jsonl"),
            "{\"type\":\"message\",\"message\":{\"Role\":\"assistant\",\"Content\":\"quedó el PR #165\"}}\n",
        )
        .unwrap();

        let state = get(server.port, "/api/state", Some(&cookie));
        assert!(state.contains(r#""last":"quedó el PR #165""#), "{state}");
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn a_project_is_renamed_with_the_conversations_inside() {
        let server = start("rename-project");
        let cookie = login(server.port, "bob@ejemplo.com");
        post_with(
            server.port,
            "/api/conversations",
            r#"{"project":"ken","title":"una charla"}"#,
            Some(&cookie),
        );

        let renamed = post_with(
            server.port,
            "/api/rename-project",
            r#"{"project":"ken","name":"ken-viejo"}"#,
            Some(&cookie),
        );
        assert!(renamed.starts_with("HTTP/1.1 200"), "{renamed}");
        assert!(!server.workspace.join("projects/ken").exists());
        assert!(server.workspace.join("projects/ken-viejo").is_dir());

        let state = get(server.port, "/api/state", Some(&cookie));
        assert!(state.contains(r#""name":"ken-viejo""#), "{state}");
        assert!(state.contains("una charla"), "{state}");

        let missing = post_with(
            server.port,
            "/api/rename-project",
            r#"{"project":"fantasma","name":"otro"}"#,
            Some(&cookie),
        );
        assert!(missing.starts_with("HTTP/1.1 400"), "{missing}");
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn a_project_is_duplicated_with_the_files_inside() {
        let server = start("duplicate-project");
        let cookie = login(server.port, "bob@ejemplo.com");
        std::fs::write(
            server.workspace.join("projects/ken/nota.txt"),
            "los mismos archivos",
        )
        .unwrap();
        std::fs::create_dir_all(server.workspace.join("projects/ken/src")).unwrap();
        std::fs::write(
            server.workspace.join("projects/ken/src/main.go"),
            "package main",
        )
        .unwrap();

        let copied = post_with(
            server.port,
            "/api/duplicate-project",
            r#"{"project":"ken","name":"ken-2"}"#,
            Some(&cookie),
        );
        assert!(copied.starts_with("HTTP/1.1 200"), "{copied}");
        assert_eq!(
            std::fs::read_to_string(server.workspace.join("projects/ken-2/nota.txt")).unwrap(),
            "los mismos archivos"
        );
        assert_eq!(
            std::fs::read_to_string(server.workspace.join("projects/ken-2/src/main.go")).unwrap(),
            "package main"
        );
        assert!(
            server.workspace.join("projects/ken/nota.txt").is_file(),
            "el original queda"
        );

        let state = get(server.port, "/api/state", Some(&cookie));
        assert!(state.contains(r#""name":"ken-2""#), "{state}");

        let again = post_with(
            server.port,
            "/api/duplicate-project",
            r#"{"project":"ken","name":"ken-2"}"#,
            Some(&cookie),
        );
        assert!(again.starts_with("HTTP/1.1 400"), "{again}");

        let missing = post_with(
            server.port,
            "/api/duplicate-project",
            r#"{"project":"fantasma","name":"copia"}"#,
            Some(&cookie),
        );
        assert!(missing.starts_with("HTTP/1.1 400"), "{missing}");

        let general = post_with(
            server.port,
            "/api/duplicate-project",
            r#"{"project":"general","name":"copia"}"#,
            Some(&cookie),
        );
        assert!(general.starts_with("HTTP/1.1 400"), "{general}");

        let unauthenticated = post(
            server.port,
            "/api/duplicate-project",
            r#"{"project":"ken","name":"copia"}"#,
        );
        assert!(
            unauthenticated.starts_with("HTTP/1.1 401"),
            "{unauthenticated}"
        );
        assert!(
            !server.workspace.join("projects/copia").exists(),
            "sin sesión no se copia nada"
        );
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn the_state_lists_projects_and_conversations() {
        let server = start("state");
        let cookie = login(server.port, "bob@ejemplo.com");

        let mut state = get(server.port, "/api/state", Some(&cookie));
        assert!(state.contains("\"general\""), "{state}");
        assert!(state.contains("\"123456789\""), "{state}");
        assert!(state.contains("\"read_only\":true"), "{state}");
        assert!(state.contains("\"ken\""), "{state}");
        assert!(state.contains("\"bob\""), "{state}");
        assert!(state.contains("\"workspace\""), "{state}");
        assert!(state.contains("\"unversioned\":false"), "{state}");
        assert!(state.contains("\"machine\""), "{state}");
        assert!(state.contains("\"disk\""), "{state}");
        assert_eq!(weight(&state, "ken"), 0, "el proyecto vacío no pesa");
        assert_eq!(weight(&state, "general"), 0, "general es el workspace");

        std::fs::write(server.workspace.join("projects/ken/nota.txt"), "x").unwrap();
        state = get(server.port, "/api/state", Some(&cookie));
        assert!(state.contains("\"unversioned\":true"), "{state}");
        assert!(weight(&state, "ken") > 0, "el archivo suma peso: {state}");

        let logout = post(server.port, "/api/logout", "{}");
        assert!(logout.contains("Max-Age=0"), "{logout}");
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn the_web_creates_and_writes_its_own_conversations() {
        let server = start("write");
        let cookie = login(server.port, "bob@ejemplo.com");

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
        assert!(
            text.contains("\"author\":\"bob\""),
            "el mensaje dice quién lo mandó: {text}"
        );
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
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn a_conversation_that_comes_from_a_transport_is_not_written_from_the_web() {
        let server = start("readonly");
        let cookie = login(server.port, "bob@ejemplo.com");
        let sent = post_with(
            server.port,
            "/api/send",
            r#"{"conversation":"123456789","text":"hola"}"#,
            Some(&cookie),
        );
        assert!(sent.starts_with("HTTP/1.1 400"), "{sent}");
        let renamed = post_with(
            server.port,
            "/api/rename",
            r#"{"conversation":"123456789","title":"mío"}"#,
            Some(&cookie),
        );
        assert!(renamed.starts_with("HTTP/1.1 400"), "{renamed}");
        assert!(!server.root.join("chats/123456789/meta.json").exists());

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
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn an_image_goes_up_as_a_name_and_comes_back_whole() {
        let server = start("upload");
        let cookie = login(server.port, "bob@ejemplo.com");
        let key = conversation(server.port, &cookie);

        let png = b"\x89PNG\r\n\x1a\nlos bytes que sean";
        let uploaded = post_bytes(
            server.port,
            &format!("/api/upload?conversation={key}&name=foto.png"),
            png,
            Some(&cookie),
        );
        assert!(uploaded.starts_with("HTTP/1.1 200"), "{uploaded}");
        let name = json_in(&uploaded)["name"].as_str().unwrap().to_string();
        assert!(name.ends_with("-foto.png"), "{name}");

        let served = get(
            server.port,
            &format!("/api/file?conversation={key}&name={name}"),
            Some(&cookie),
        );
        assert!(served.starts_with("HTTP/1.1 200"), "{served}");
        assert!(served.contains("Content-Type: image/png"), "{served}");
        assert!(body_in(&served).ends_with("los bytes que sean"), "{served}");

        let sent = post_with(
            server.port,
            "/api/send",
            &format!(r#"{{"conversation":"{key}","text":"","images":["{name}"]}}"#),
            Some(&cookie),
        );
        assert!(
            sent.starts_with("HTTP/1.1 202"),
            "una foto sin texto es un mensaje: {sent}"
        );

        let log = wait_for(&server.root, &key, "\"done\"");
        assert!(
            log.contains(&format!("\"images\":[\"{name}\"]")),
            "el log guarda el nombre y no los bytes: {log}"
        );
        let state = get(server.port, "/api/state", Some(&cookie));
        assert!(
            state.contains("\"title\":\"imagen\""),
            "una conversación que arranca con una foto se llama por eso: {state}"
        );
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn an_upload_only_touches_its_own_conversation() {
        let server = start("traversal");
        let cookie = login(server.port, "bob@ejemplo.com");
        let key = conversation(server.port, &cookie);

        let uploaded = post_bytes(
            server.port,
            &format!("/api/upload?conversation={key}&name=../../afuera.png"),
            b"x",
            Some(&cookie),
        );
        let name = json_in(&uploaded)["name"].as_str().unwrap().to_string();
        assert!(!name.contains('/'), "{name}");
        assert!(
            server
                .root
                .join("chats")
                .join(&key)
                .join("uploads")
                .join(&name)
                .is_file(),
            "el adjunto queda en su carpeta"
        );
        assert!(
            !server.root.join("chats/afuera.png").exists(),
            "y no un directorio más arriba"
        );

        let traversal = get(
            server.port,
            &format!("/api/file?conversation={key}&name=../../meta.json"),
            Some(&cookie),
        );
        assert!(traversal.starts_with("HTTP/1.1 400"), "{traversal}");

        let missing = get(
            server.port,
            &format!("/api/file?conversation={key}&name=16-no-esta.png"),
            Some(&cookie),
        );
        assert!(missing.starts_with("HTTP/1.1 404"), "{missing}");

        let anonymous = post_bytes(
            server.port,
            &format!("/api/upload?conversation={key}&name=a.png"),
            b"x",
            None,
        );
        assert!(anonymous.starts_with("HTTP/1.1 401"), "{anonymous}");

        let readonly = post_bytes(
            server.port,
            "/api/upload?conversation=123456789&name=a.png",
            b"x",
            Some(&cookie),
        );
        assert!(readonly.starts_with("HTTP/1.1 400"), "{readonly}");

        let empty = post_with(
            server.port,
            "/api/send",
            &format!(r#"{{"conversation":"{key}","text":""}}"#),
            Some(&cookie),
        );
        assert!(
            empty.starts_with("HTTP/1.1 400"),
            "sin texto y sin adjuntos no hay mensaje: {empty}"
        );

        let unknown = post_with(
            server.port,
            "/api/send",
            &format!(r#"{{"conversation":"{key}","text":"mirá","images":["16-no-esta.png"]}}"#),
            Some(&cookie),
        );
        assert!(unknown.starts_with("HTTP/1.1 400"), "{unknown}");

        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn the_file_tree_lists_a_project_one_level_at_a_time() {
        let server = start("files-tree");
        let cookie = login(server.port, "bob@ejemplo.com");
        let ken = server.workspace.join("projects/ken");
        std::fs::create_dir_all(ken.join("src/transport")).unwrap();
        std::fs::create_dir_all(ken.join("target")).unwrap();
        std::fs::create_dir_all(ken.join(".git")).unwrap();
        std::fs::write(ken.join("src/web.rs"), "fn main() {}\n").unwrap();
        std::fs::write(ken.join("src/transport/telegram.rs"), "// nada\n").unwrap();
        std::fs::write(ken.join("target/gordo"), "no se lista").unwrap();
        std::fs::write(ken.join("README.md"), "# ken\n").unwrap();
        std::fs::write(ken.join(".env"), "SECRETO=1\n").unwrap();
        std::fs::write(ken.join("logo.png"), b"\x89PNG\r\n\x1a\nno importa").unwrap();

        let tree = get(server.port, "/api/tree?project=ken", Some(&cookie));
        assert!(tree.starts_with("HTTP/1.1 200"), "{tree}");
        let entries = json_in(&tree)["entries"].clone();
        let listed: Vec<&str> = entries
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["name"].as_str().unwrap())
            .collect();
        assert_eq!(listed, ["src", "logo.png", "README.md"], "{entries}");
        assert_eq!(entries[0]["dir"], serde_json::json!(true));
        assert_eq!(entries[1]["kind"], serde_json::json!("image"));
        assert_eq!(entries[2]["kind"], serde_json::json!("markdown"));

        let deeper = get(server.port, "/api/tree?project=ken&path=src", Some(&cookie));
        let entries = json_in(&deeper)["entries"].clone();
        let listed: Vec<&str> = entries
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["name"].as_str().unwrap())
            .collect();
        assert_eq!(listed, ["transport", "web.rs"], "{entries}");
        assert_eq!(entries[1]["kind"], serde_json::json!("text"));

        let general = get(server.port, "/api/tree?project=general", Some(&cookie));
        assert!(general.contains("\"name\":\"projects\""), "{general}");

        let source = get(
            server.port,
            "/api/raw?project=ken&path=src/web.rs",
            Some(&cookie),
        );
        assert!(source.starts_with("HTTP/1.1 200"), "{source}");
        assert!(source.contains("Content-Type: text/plain"), "{source}");
        assert!(body_in(&source).ends_with("fn main() {}\n"), "{source}");

        let image = get(
            server.port,
            "/api/raw?project=ken&path=logo.png",
            Some(&cookie),
        );
        assert!(image.contains("Content-Type: image/png"), "{image}");

        std::fs::write(ken.join("binario"), b"\x00\x01\x02hola").unwrap();
        let binary = get(
            server.port,
            "/api/raw?project=ken&path=binario",
            Some(&cookie),
        );
        assert!(
            binary.contains("Content-Type: application/octet-stream"),
            "{binary}"
        );

        std::fs::write(ken.join("grande.txt"), "a".repeat(600 * 1024)).unwrap();
        let huge = get(
            server.port,
            "/api/raw?project=ken&path=grande.txt",
            Some(&cookie),
        );
        assert!(huge.contains("X-Truncated: 1"), "el corte se avisa");
        assert_eq!(body_in(&huge).len(), files::MAX_READ as usize);

        let folder = get(server.port, "/api/raw?project=ken&path=src", Some(&cookie));
        assert!(folder.starts_with("HTTP/1.1 404"), "{folder}");

        assert!(get(server.port, "/api/tree?project=ken", None).starts_with("HTTP/1.1 401"));
        assert!(
            get(server.port, "/api/raw?project=ken&path=README.md", None)
                .starts_with("HTTP/1.1 401")
        );
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn the_file_tree_cannot_leave_the_project() {
        let server = start("files-outside");
        let cookie = login(server.port, "bob@ejemplo.com");
        let ken = server.workspace.join("projects/ken");
        std::fs::write(server.root.join("afuera.txt"), "mas secreto").unwrap();
        std::fs::write(server.workspace.join("notes.md"), "secreto").unwrap();
        std::os::unix::fs::symlink(&server.root, ken.join("atajo")).unwrap();

        let paths = [
            "/api/tree?project=ken&path=..",
            "/api/tree?project=ken&path=../../..",
            "/api/tree?project=ken&path=atajo",
            "/api/tree?project=..",
            "/api/tree?project=%2e%2e%2f%2e%2e",
            "/api/tree?project=no-existe",
            "/api/raw?project=ken&path=../notes.md",
            "/api/raw?project=ken&path=../../afuera.txt",
            "/api/raw?project=ken&path=atajo/afuera.txt",
            "/api/raw?project=ken&path=.env",
            "/api/raw?project=ken&path=no-esta.txt",
        ];
        for path in paths {
            let response = get(server.port, path, Some(&cookie));
            assert!(
                response.starts_with("HTTP/1.1 40"),
                "{path} devolvió {response}"
            );
        }
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn the_stream_says_who_is_watching() {
        let server = start("presence");
        let cookie = login(server.port, "bob@ejemplo.com");

        let mut stream = connect(server.port);
        write!(
            stream,
            "GET /api/stream?conversation=123456789 HTTP/1.1\r\nHost: jimmy\r\nCookie: {cookie}\r\n\r\n"
        )
        .unwrap();
        let mut lines = BufReader::new(stream.try_clone().unwrap()).lines();
        let seen: Vec<String> = lines
            .by_ref()
            .take(12)
            .filter_map(Result::ok)
            .filter(|line| line.starts_with("data: "))
            .collect();
        assert!(
            seen.iter()
                .any(|line| line.contains("presence") && line.contains("bob")),
            "esperaba quién está mirando: {seen:?}"
        );
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn the_online_stream_knows_everyone_connected() {
        let server = start("online");
        let bob = login(server.port, "bob@ejemplo.com");

        let mut stream = connect(server.port);
        write!(
            stream,
            "GET /api/online HTTP/1.1\r\nHost: jimmy\r\nCookie: {bob}\r\n\r\n"
        )
        .unwrap();
        let mut lines = BufReader::new(stream.try_clone().unwrap()).lines();
        assert!(next_data(&mut lines, "\"online\"").contains("bob"));

        let ana = login(server.port, "ana@ejemplo.com");
        let mut second = connect(server.port);
        write!(
            second,
            "GET /api/online HTTP/1.1\r\nHost: jimmy\r\nCookie: {ana}\r\n\r\n"
        )
        .unwrap();
        let seen = next_data(&mut lines, "\"online\"");
        assert!(seen.contains("ana") && seen.contains("bob"), "{seen}");
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn typing_goes_live_but_is_not_written_down() {
        let server = start("typing");
        let cookie = login(server.port, "bob@ejemplo.com");
        let created = post_with(
            server.port,
            "/api/conversations",
            r#"{"project":"ken"}"#,
            Some(&cookie),
        );
        let body: serde_json::Value =
            serde_json::from_str(created.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        let key = body["key"].as_str().unwrap().to_string();

        let without = post(
            server.port,
            "/api/typing",
            &format!(r#"{{"conversation":"{key}"}}"#),
        );
        assert!(without.starts_with("HTTP/1.1 401"), "{without}");
        let readonly = post_with(
            server.port,
            "/api/typing",
            r#"{"conversation":"123456789"}"#,
            Some(&cookie),
        );
        assert!(readonly.starts_with("HTTP/1.1 400"), "{readonly}");

        let mut stream = connect(server.port);
        write!(
            stream,
            "GET /api/stream?conversation={key} HTTP/1.1\r\nHost: jimmy\r\nCookie: {cookie}\r\n\r\n"
        )
        .unwrap();
        let mut lines = BufReader::new(stream.try_clone().unwrap()).lines();
        assert!(next_data(&mut lines, "\"synced\"").contains("synced"));

        let sent = post_with(
            server.port,
            "/api/typing",
            &format!(r#"{{"conversation":"{key}"}}"#),
            Some(&cookie),
        );
        assert!(sent.starts_with("HTTP/1.1 200"), "{sent}");
        let seen = next_data(&mut lines, "\"typing\"");
        assert!(seen.contains("bob"), "{seen}");

        let log = server
            .root
            .join("chats")
            .join(&key)
            .join("conversation.jsonl");
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        assert!(!text.contains("typing"), "{text}");
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    fn next_data(lines: &mut std::io::Lines<BufReader<TcpStream>>, needle: &str) -> String {
        lines
            .by_ref()
            .take(200)
            .filter_map(Result::ok)
            .find(|line| line.starts_with("data: ") && line.contains(needle))
            .unwrap_or_default()
    }

    #[test]
    fn the_stream_opens_at_a_turn_and_the_history_goes_back() {
        let server = start("history");
        let cookie = login(server.port, "bob@ejemplo.com");
        let mut log = String::new();
        for turn in 0..300 {
            log.push_str(&format!(
                "{{\"event\":\"user\",\"text\":\"{turn}\"}}\n{{\"event\":\"tool_start\",\"id\":\"{turn}\",\"name\":\"read\",\"args\":\"{{}}\"}}\n{{\"event\":\"done\",\"text\":\"{turn}\"}}\n"
            ));
        }
        std::fs::write(server.root.join("chats/123456789/conversation.jsonl"), &log).unwrap();

        let mut stream = connect(server.port);
        write!(
            stream,
            "GET /api/stream?conversation=123456789&since=0 HTTP/1.1\r\nHost: jimmy\r\nCookie: {cookie}\r\n\r\n"
        )
        .unwrap();
        let mut lines = BufReader::new(stream.try_clone().unwrap()).lines();
        let mut events: Vec<serde_json::Value> = Vec::new();
        loop {
            let line = lines.next().unwrap().unwrap();
            let Some(data) = line.strip_prefix("data: ") else {
                continue;
            };
            let event: serde_json::Value = serde_json::from_str(data).unwrap();
            if event["event"] == "synced" {
                let synced = event;
                assert_eq!(synced["count"], 900);
                let first = synced["first"].as_u64().unwrap();
                assert!(first > 0, "un log largo no entra entero: {synced}");
                assert_eq!(first + events.len() as u64, 900);
                assert_eq!(events[0]["event"], "user", "el tramo arranca en un turno");
                break;
            }
            events.push(event);
        }
        assert!(events.len() <= crate::log::REPLAY);

        let body = get(
            server.port,
            "/api/history?conversation=123456789&before=702",
            Some(&cookie),
        );
        let earlier: serde_json::Value =
            serde_json::from_str(body.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        let events = earlier["events"].as_array().unwrap();
        assert_eq!(events[0]["event"], "user");
        assert_eq!(
            earlier["first"].as_u64().unwrap() + events.len() as u64,
            702
        );

        let empty = get(
            server.port,
            "/api/history?conversation=123456789&before=0",
            Some(&cookie),
        );
        let empty: serde_json::Value =
            serde_json::from_str(empty.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(empty["first"], 0);
        assert!(empty["events"].as_array().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn cancel_only_goes_to_a_conversation_you_can_write() {
        let server = start("cancel");
        let cookie = login(server.port, "bob@ejemplo.com");

        let refused = post_with(
            server.port,
            "/api/cancel",
            r#"{"conversation":"123456789"}"#,
            Some(&cookie),
        );
        assert!(refused.starts_with("HTTP/1.1 400"), "{refused}");

        let created = post_with(
            server.port,
            "/api/conversations",
            r#"{"project":"ken"}"#,
            Some(&cookie),
        );
        let body: serde_json::Value =
            serde_json::from_str(created.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        let key = body["key"].as_str().unwrap().to_string();
        let cancelled = post_with(
            server.port,
            "/api/cancel",
            &format!(r#"{{"conversation":"{key}"}}"#),
            Some(&cookie),
        );
        assert!(cancelled.starts_with("HTTP/1.1 200"), "{cancelled}");
        let log = std::fs::read_to_string(
            server
                .root
                .join("chats")
                .join(&key)
                .join("conversation.jsonl"),
        )
        .unwrap_or_default();
        assert!(
            log.contains("\"stopped\"") && log.contains("bob"),
            "el log dice quién frenó: {log}"
        );

        let unknown = post_with(
            server.port,
            "/api/cancel",
            r#"{"conversation":"no-existe"}"#,
            Some(&cookie),
        );
        assert!(unknown.starts_with("HTTP/1.1 400"), "{unknown}");
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn a_body_that_is_too_big_is_refused_without_reading_it() {
        let server = start("big");
        let mut stream = connect(server.port);
        write!(
            stream,
            "POST /api/send HTTP/1.1\r\nHost: jimmy\r\nContent-Length: {}\r\n\r\n",
            http::MAX_BODY + 1
        )
        .unwrap();
        let response = whole(stream);
        assert!(response.starts_with("HTTP/1.1 413"), "{response}");
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn search_looks_in_what_was_said() {
        let server = start("search");
        let cookie = login(server.port, "bob@ejemplo.com");

        let found = get(server.port, "/api/search?q=gato", Some(&cookie));
        assert!(found.contains("el gato duerme"), "{found}");
        assert!(found.contains("123456789"), "{found}");
        assert!(found.contains("\"role\":\"user\""), "{found}");

        let nothing = get(server.port, "/api/search?q=elefante", Some(&cookie));
        assert!(nothing.contains("\"results\":[]"), "{nothing}");

        let short = get(server.port, "/api/search?q=g", Some(&cookie));
        assert!(short.contains("\"results\":[]"), "{short}");
        assert!(get(server.port, "/api/search?q=gato", None).starts_with("HTTP/1.1 401"));
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn delete_takes_the_conversation_with_it() {
        let server = start("delete");
        let cookie = login(server.port, "bob@ejemplo.com");

        let created = post_with(
            server.port,
            "/api/conversations",
            r#"{"project":"ken"}"#,
            Some(&cookie),
        );
        let body: serde_json::Value =
            serde_json::from_str(created.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        let key = body["key"].as_str().unwrap().to_string();
        assert!(server.root.join("chats").join(&key).is_dir());

        let deleted = post_with(
            server.port,
            "/api/delete-conversation",
            &format!(r#"{{"conversation":"{key}"}}"#),
            Some(&cookie),
        );
        assert!(deleted.starts_with("HTTP/1.1 200"), "{deleted}");
        assert!(!server.root.join("chats").join(&key).exists());

        let refused = post_with(
            server.port,
            "/api/delete-conversation",
            r#"{"conversation":"123456789"}"#,
            Some(&cookie),
        );
        assert!(refused.starts_with("HTTP/1.1 400"), "{refused}");
        assert!(server.root.join("chats/123456789").is_dir());
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn a_project_is_deleted_only_when_it_has_nothing_to_lose() {
        let server = start("delete-project");
        let cookie = login(server.port, "bob@ejemplo.com");
        let projects = server.workspace.join("projects");

        std::fs::write(projects.join("ken/nota.txt"), "trabajo sin versionar").unwrap();
        let dirty = post_with(
            server.port,
            "/api/delete-project",
            r#"{"project":"ken"}"#,
            Some(&cookie),
        );
        assert!(dirty.starts_with("HTTP/1.1 400"), "{dirty}");
        assert!(dirty.contains("no lo borro"), "{dirty}");
        assert!(projects.join("ken").is_dir());

        let forced = post_with(
            server.port,
            "/api/delete-project",
            r#"{"project":"ken","force":true}"#,
            Some(&cookie),
        );
        assert!(forced.starts_with("HTTP/1.1 200"), "{forced}");
        assert!(!projects.join("ken").exists());

        let general = post_with(
            server.port,
            "/api/delete-project",
            r#"{"project":"general"}"#,
            Some(&cookie),
        );
        assert!(general.starts_with("HTTP/1.1 400"), "{general}");

        std::fs::create_dir_all(projects.join("vacio")).unwrap();
        let clean = post_with(
            server.port,
            "/api/delete-project",
            r#"{"project":"vacio"}"#,
            Some(&cookie),
        );
        assert!(clean.starts_with("HTTP/1.1 200"), "{clean}");
        assert!(!projects.join("vacio").exists());

        let repo = projects.join("clon");
        std::fs::create_dir_all(&repo).unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "jimmy@test"],
            vec!["config", "user.name", "jimmy"],
        ] {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(&args)
                .output()
                .unwrap();
        }
        std::fs::write(repo.join("nota.txt"), "hola").unwrap();
        for args in [vec!["add", "-A"], vec!["commit", "-q", "-m", "algo"]] {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(&args)
                .output()
                .unwrap();
        }
        let unpushed = post_with(
            server.port,
            "/api/delete-project",
            r#"{"project":"clon"}"#,
            Some(&cookie),
        );
        assert!(unpushed.starts_with("HTTP/1.1 400"), "{unpushed}");
        assert!(unpushed.contains("sin pushear"), "{unpushed}");
        assert!(repo.is_dir());

        let forced_repo = post_with(
            server.port,
            "/api/delete-project",
            r#"{"project":"clon","force":true}"#,
            Some(&cookie),
        );
        assert!(forced_repo.starts_with("HTTP/1.1 400"), "{forced_repo}");
        assert!(forced_repo.contains("sin pushear"), "{forced_repo}");
        assert!(repo.is_dir());
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn the_stream_replays_the_backlog_and_then_follows() {
        let server = start("stream");
        let cookie = login(server.port, "bob@ejemplo.com");

        let mut stream = connect(server.port);
        write!(
            stream,
            "GET /api/stream?conversation=123456789 HTTP/1.1\r\nHost: jimmy\r\nCookie: {cookie}\r\n\r\n"
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

        let log = Log::in_dir(&server.root.join("chats/123456789"));
        server.bus.publish(
            "123456789",
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
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn the_stream_skips_what_the_watcher_already_has() {
        let server = start("stream-since");
        let cookie = login(server.port, "bob@ejemplo.com");

        let mut stream = connect(server.port);
        write!(
            stream,
            "GET /api/stream?conversation=123456789&since=1 HTTP/1.1\r\nHost: jimmy\r\nCookie: {cookie}\r\n\r\n"
        )
        .unwrap();
        let mut lines = BufReader::new(stream.try_clone().unwrap()).lines();

        let mut seen = Vec::new();
        while seen.len() < 2 {
            let line = lines.next().unwrap().unwrap();
            if let Some(data) = line.strip_prefix("data: ") {
                seen.push(data.to_string());
            }
        }
        assert!(seen[0].contains("\"listo\""), "{seen:?}");
        assert!(seen[1].contains("\"count\":2"), "{seen:?}");
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    fn variable(css: &str, name: &str) -> String {
        let prefix = format!("--{name}:");
        css.lines()
            .filter_map(|line| line.trim().strip_prefix(&prefix))
            .map(|value| value.trim().trim_end_matches(';').to_string())
            .next()
            .unwrap_or_else(|| panic!("falta la variable --{name} en el css"))
    }

    fn channel(color: &str) -> f64 {
        let value = u8::from_str_radix(color, 16).expect("color no hexadecimal") as f64 / 255.0;
        if value <= 0.03928 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }

    fn relative_luminance(color: &str) -> f64 {
        let color = color.trim_start_matches('#');
        assert_eq!(color.len(), 6, "esperaba un color de seis dígitos: {color}");
        0.2126 * channel(&color[0..2])
            + 0.7152 * channel(&color[2..4])
            + 0.0722 * channel(&color[4..6])
    }

    fn contrast(one: &str, other: &str) -> f64 {
        let (one, other) = (relative_luminance(one), relative_luminance(other));
        let (high, low) = if one > other {
            (one, other)
        } else {
            (other, one)
        };
        (high + 0.05) / (low + 0.05)
    }

    fn block_of(css: &str, selector: &str) -> String {
        let start = css
            .find(selector)
            .unwrap_or_else(|| panic!("falta el bloque {selector}"))
            + selector.len();
        let end = css[start..]
            .find("\n}")
            .unwrap_or_else(|| panic!("el bloque {selector} no cierra"))
            + start;
        css[start..end].to_string()
    }

    #[test]
    fn both_themes_keep_their_text_readable() {
        let themes = [
            ("oscuro", ":root {"),
            ("claro", ":root[data-theme=\"light\"] {"),
        ];
        for (theme, selector) in themes {
            let block = block_of(THEME, selector);
            let at = |name: &str| variable(&block, name);
            let backgrounds = [
                ("bg", at("bg")),
                ("panel", at("panel")),
                ("raised", at("raised")),
            ];

            let readable = ["text", "muted", "faint", "accent", "ok", "warn", "err"];
            let in_code = [
                "code",
                "tok-comment",
                "tok-string",
                "tok-keyword",
                "tok-number",
                "tok-add",
                "tok-del",
                "tok-meta",
            ];
            for name in readable.into_iter().chain(in_code) {
                let color = at(name);
                for (fondo, base) in &backgrounds {
                    let ratio = contrast(&color, base);
                    assert!(
                        ratio >= 4.5,
                        "tema {theme}: {name} sobre {fondo} da {ratio:.2}, por debajo de 4.5:1 de AA"
                    );
                }
            }

            for (fondo, base) in &backgrounds[0..2] {
                let border = at("line-strong");
                let ratio = contrast(&border, base);
                assert!(
                    ratio >= 3.0,
                    "tema {theme}: el borde de los campos (#{border}) sobre {fondo} da {ratio:.2}, por debajo de 3:1"
                );
            }
        }
    }

    fn agenda_file(server: &Server, name: &str, body: &str) {
        let dir = server.workspace.join("state/schedule");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{name}.toml")), body).unwrap();
    }

    #[test]
    fn the_agenda_lists_tasks_with_their_last_runs() {
        let server = start("agenda");
        let cookie = login(server.port, "bob@ejemplo.com");
        agenda_file(
            &server,
            "memoria",
            "at = \"05:00\"\ntarget = \"123456789\"\nprompt = \"reportá\"\n",
        );
        agenda_file(
            &server,
            "perezosa",
            "every = \"6h\"\nprompt = \"p\"\npaused = true\n",
        );
        let runs: String = (1..8)
            .map(|i| {
                format!(
                    "{{\"ts\":{i},\"date\":\"2026-09-14\",\"ms\":{},\"ok\":true,\"text\":\"corrida {i}\"}}\n",
                    i * 10
                )
            })
            .collect();
        std::fs::write(server.workspace.join("state/schedule/memoria.jsonl"), runs).unwrap();

        let response = get(server.port, "/api/agenda", Some(&cookie));
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        let body = json_in(&response);
        let tasks = body["tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0]["name"], "memoria");
        assert_eq!(tasks[0]["at"], "05:00");
        assert_eq!(tasks[0]["target"], "123456789");
        let runs = tasks[0]["runs"].as_array().unwrap();
        assert_eq!(runs.len(), 5, "sólo se muestran las últimas cinco");
        assert_eq!(runs[0]["text"], "corrida 7");
        assert_eq!(runs[4]["text"], "corrida 3");
        assert_eq!(tasks[1]["name"], "perezosa");
        assert_eq!(tasks[1]["paused"], true);
        assert!(tasks[1]["runs"].as_array().unwrap().is_empty());
        assert_eq!(tasks[0]["unread"], 7, "nada leído todavía");

        let read = post_with(
            server.port,
            "/api/agenda/read",
            r#"{"name":"memoria"}"#,
            Some(&cookie),
        );
        assert!(read.starts_with("HTTP/1.1 200"), "{read}");
        let body = json_in(&get(server.port, "/api/agenda", Some(&cookie)));
        assert_eq!(body["tasks"][0]["unread"], 0);

        let path = server.workspace.join("state/schedule/memoria.jsonl");
        let mut lines = std::fs::read_to_string(&path).unwrap();
        lines.push_str(
            "{\"ts\":99,\"date\":\"2026-09-15\",\"ms\":5,\"ok\":true,\"text\":\"otra\"}\n",
        );
        std::fs::write(&path, lines).unwrap();
        let body = json_in(&get(server.port, "/api/agenda", Some(&cookie)));
        assert_eq!(body["tasks"][0]["unread"], 1, "la que llegó después cuenta");
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn running_a_task_hands_its_name_to_the_scheduler() {
        let server = start("agenda-run");
        let cookie = login(server.port, "bob@ejemplo.com");
        agenda_file(&server, "memoria", "at = \"05:00\"\nprompt = \"p\"\n");

        let queued = post_with(
            server.port,
            "/api/agenda/run",
            r#"{"name":"memoria"}"#,
            Some(&cookie),
        );
        assert!(queued.starts_with("HTTP/1.1 200"), "{queued}");
        assert_eq!(
            server.agenda.recv_timeout(Duration::from_secs(1)).unwrap(),
            "memoria"
        );

        let missing = post_with(
            server.port,
            "/api/agenda/run",
            r#"{"name":"nada"}"#,
            Some(&cookie),
        );
        assert!(missing.starts_with("HTTP/1.1 404"), "{missing}");
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn pausing_a_task_from_the_web_keeps_the_rest_of_the_file() {
        let server = start("agenda-pause");
        let cookie = login(server.port, "bob@ejemplo.com");
        agenda_file(
            &server,
            "memoria",
            "at = \"05:00\"\ntarget = \"123\"\nprompt = \"reportá\"\nsilent = true\n",
        );

        let paused = post_with(
            server.port,
            "/api/agenda/pause",
            r#"{"name":"memoria","paused":true}"#,
            Some(&cookie),
        );
        assert!(paused.starts_with("HTTP/1.1 200"), "{paused}");
        let body = json_in(&get(server.port, "/api/agenda", Some(&cookie)));
        assert_eq!(body["tasks"][0]["paused"], true);
        assert_eq!(body["tasks"][0]["silent"], true);
        assert_eq!(body["tasks"][0]["at"], "05:00");

        let resumed = post_with(
            server.port,
            "/api/agenda/pause",
            r#"{"name":"memoria","paused":false}"#,
            Some(&cookie),
        );
        assert!(resumed.starts_with("HTTP/1.1 200"), "{resumed}");
        assert_eq!(
            json_in(&get(server.port, "/api/agenda", Some(&cookie)))["tasks"][0]["paused"],
            false
        );
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }
}
