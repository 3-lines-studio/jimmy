//! The web frontend: the same conversations the transports have, served to a
//! browser over HTTP and SSE.
//!
//! What arrives through a transport is watched, not written; the conversations
//! the web creates are its own and can be written from here.

use crate::agent::Agent;
use crate::auth::{self, Auth};
use crate::bus::Bus;
use crate::conversations;
use crate::http::{self, Request};
use crate::log::Log;
use crate::media;
use crate::protocol::Event;
use crate::transport::{Msg, Session, Transport};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::Arc;
use std::time::Duration;

/// El final del backlog: lo de antes ya está en pantalla. Lleva cuántos eventos
/// tiene el log, para que el que mira guarde el número y al volver a la
/// conversación pida desde ahí en vez de bajar todo de nuevo.
fn synced(total: usize) -> String {
    format!(r#"{{"event":"synced","count":{total}}}"#)
}

const INDEX: &str = include_str!("../web/index.html");
const LOGIN: &str = include_str!("../web/login.html");
const THEME: &str = include_str!("../web/theme.css");
const STYLE: &str = include_str!("../web/style.css");
const APP: &str = include_str!("../web/app.js");
const MARKDOWN: &str = include_str!("../web/markdown.js");
const TOOL: &str = include_str!("../web/tool.js");
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
}

impl Web {
    pub fn new(
        root: PathBuf,
        workspace: PathBuf,
        bus: Arc<Bus>,
        agent: Agent,
        auth: Auth,
    ) -> Arc<Web> {
        Arc::new(Web {
            root,
            workspace,
            bus,
            agent,
            auth,
        })
    }

    /// El nombre para mostrar: la parte del mail antes del arroba.
    fn user(&self, request: &Request) -> Option<String> {
        let email = self.auth.user(&request.cookie(auth::COOKIE)?)?;
        Some(match email.split_once('@') {
            Some((name, _)) => name.to_string(),
            None => email,
        })
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

fn handle(web: &Arc<Web>, stream: &mut TcpStream) -> std::io::Result<()> {
    let Some(request) = http::read(stream)? else {
        return Ok(());
    };
    if request.too_large {
        return http::send_error(stream, 413, "eso es demasiado grande");
    }
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/") => app_page(web, &request, stream),
        ("GET", "/login") => http::send_text(stream, 200, HTML, &versioned(LOGIN)),
        ("GET", "/theme.css") => {
            http::send_text(stream, 200, "text/css; charset=utf-8", &versioned(THEME))
        }
        ("GET", "/style.css") => http::send_text(stream, 200, "text/css; charset=utf-8", STYLE),
        ("GET", "/app.js") => http::send_text(stream, 200, "text/javascript; charset=utf-8", APP),
        ("GET", "/markdown.js") => {
            http::send_text(stream, 200, "text/javascript; charset=utf-8", MARKDOWN)
        }
        ("GET", "/tool.js") => http::send_text(stream, 200, "text/javascript; charset=utf-8", TOOL),
        ("GET", "/icon.svg") => http::send_text(stream, 200, "image/svg+xml", ICON),
        ("GET", "/icon-192.png") => http::respond(stream, 200, "image/png", &[], ICON_192),
        ("GET", "/icon-512.png") => http::respond(stream, 200, "image/png", &[], ICON_512),
        ("GET", "/manifest.webmanifest") => http::send_text(
            stream,
            200,
            "application/manifest+json",
            &versioned(MANIFEST),
        ),
        ("POST", "/api/login") => login(web, &request, stream),
        ("GET", "/auth") => auth_link(web, &request, stream),
        ("POST", "/api/logout") => logout(web, &request, stream),
        ("GET", "/api/state") => state(web, &request, stream),
        ("GET", "/api/stream") => events(web, &request, stream),
        ("GET", "/api/online") => online(web, &request, stream),
        ("GET", "/api/search") => search(web, &request, stream),
        ("POST", "/api/conversations") => create(web, &request, stream),
        ("POST", "/api/projects") => create_project(web, &request, stream),
        ("POST", "/api/rename") => rename(web, &request, stream),
        ("POST", "/api/send") => send(web, &request, stream),
        ("POST", "/api/upload") => upload(web, &request, stream),
        ("GET", "/api/file") => file(web, &request, stream),
        ("POST", "/api/typing") => typing(web, &request, stream),
        ("POST", "/api/cancel") => cancel(web, &request, stream),
        ("POST", "/api/delete-conversation") => delete_conversation(web, &request, stream),
        ("POST", "/api/delete-project") => delete_project(web, &request, stream),
        _ => http::send_text(stream, 404, "text/plain", "no está"),
    }
}

const HTML: &str = "text/html; charset=utf-8";

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
        .replace("/markdown.js", &format!("/markdown.js?v={version}"))
        .replace("/tool.js", &format!("/tool.js?v={version}"))
        .replace("/theme.css", &format!("/theme.css?v={version}"))
        .replace("/style.css", &format!("/style.css?v={version}"))
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
                "conversations": conversations,
            })
        })
        .collect();
    http::send_json(
        stream,
        200,
        &serde_json::json!({
            "user": user,
            "workspace": web.workspace.display().to_string(),
            "projects": projects,
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
        if let Err(error) = web.agent.respond(&Silent, &session, &text, images, &user) {
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
    let log = Log::in_dir(&conversation.dir);
    let since = request
        .param("since")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let (backlog, live) = web.bus.attach(key, &log, &user);
    let result = follow(stream, &backlog, since, &live);
    web.bus.detach(key, &user);
    result
}

fn online(web: &Arc<Web>, request: &Request, stream: &mut TcpStream) -> std::io::Result<()> {
    let Some(user) = web.user(request) else {
        return http::send_error(stream, 401, "no estás adentro");
    };
    let live = web.bus.watch(&user);
    let result = follow(stream, &[], 0, &live);
    web.bus.detach_watch(&user);
    result
}

fn follow(
    stream: &mut TcpStream,
    backlog: &[crate::protocol::Event],
    since: usize,
    live: &std::sync::mpsc::Receiver<crate::protocol::Event>,
) -> std::io::Result<()> {
    http::sse_open(stream)?;
    for event in backlog.iter().skip(since) {
        if let Ok(json) = serde_json::to_string(event) {
            http::sse_data(stream, &json)?;
        }
    }
    http::sse_data(stream, &synced(backlog.len()))?;
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
        workspace: PathBuf,
        port: u16,
        bus: Arc<Bus>,
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
        std::fs::create_dir_all(root.join("chats/7469057930")).unwrap();
        std::fs::create_dir_all(workspace.join("projects/ken")).unwrap();
        std::fs::write(
            root.join("chats/7469057930/conversation.jsonl"),
            "{\"event\":\"user\",\"text\":\"hola\"}\n{\"event\":\"done\",\"text\":\"listo\"}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("chats/7469057930/transcript.jsonl"),
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
        let auth = Auth::new("berti@ejemplo.com, ana@ejemplo.com", &root, None, dev);
        let web = Web::new(root.clone(), workspace.clone(), bus.clone(), agent, auth);
        let listener = listen(0).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || serve(web, listener));
        Server {
            root,
            workspace,
            port,
            bus,
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
            app.contains("Cache-Control: no-store"),
            "sin esto un proxy le pone su propio max-age y sirve la UI vieja: {app}"
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

        let cookie = login(server.port, "berti@ejemplo.com");
        let page = get(server.port, "/", Some(&cookie));
        assert!(page.starts_with("HTTP/1.1 200"), "{page}");
        assert!(page.contains("<div id=\"panes\">"), "{page}");
        assert!(page.contains("/markdown.js?v="), "{page}");
        assert!(page.contains("/tool.js?v="), "{page}");
        assert!(page.contains("/app.js?v="), "{page}");
        assert!(
            page.find("/tool.js?v=") < page.find("/app.js?v="),
            "la app usa lo que define el tool: {page}"
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
    fn without_the_dev_flag_the_link_stays_out_of_the_response() {
        let server = start_with("plain", false);
        let answer = post(
            server.port,
            "/api/login",
            r#"{"email":"berti@ejemplo.com"}"#,
        );
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
        let answer = post(
            server.port,
            "/api/login",
            r#"{"email":"berti@ejemplo.com"}"#,
        );
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
    fn everything_but_logging_in_needs_a_session() {
        let server = start("auth");
        assert!(get(server.port, "/api/state", None).starts_with("HTTP/1.1 401"));
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
    fn the_state_lists_projects_and_conversations() {
        let server = start("state");
        let cookie = login(server.port, "berti@ejemplo.com");

        let mut state = get(server.port, "/api/state", Some(&cookie));
        assert!(state.contains("\"general\""), "{state}");
        assert!(state.contains("\"7469057930\""), "{state}");
        assert!(state.contains("\"read_only\":true"), "{state}");
        assert!(state.contains("\"ken\""), "{state}");
        assert!(state.contains("\"berti\""), "{state}");
        assert!(state.contains("\"workspace\""), "{state}");
        assert!(state.contains("\"unversioned\":false"), "{state}");

        std::fs::write(server.workspace.join("projects/ken/nota.txt"), "x").unwrap();
        state = get(server.port, "/api/state", Some(&cookie));
        assert!(state.contains("\"unversioned\":true"), "{state}");

        let logout = post(server.port, "/api/logout", "{}");
        assert!(logout.contains("Max-Age=0"), "{logout}");
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn the_web_creates_and_writes_its_own_conversations() {
        let server = start("write");
        let cookie = login(server.port, "berti@ejemplo.com");

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
            text.contains("\"author\":\"berti\""),
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
        let cookie = login(server.port, "berti@ejemplo.com");
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
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn an_image_goes_up_as_a_name_and_comes_back_whole() {
        let server = start("upload");
        let cookie = login(server.port, "berti@ejemplo.com");
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
        let cookie = login(server.port, "berti@ejemplo.com");
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
            "/api/upload?conversation=7469057930&name=a.png",
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
    fn the_stream_says_who_is_watching() {
        let server = start("presence");
        let cookie = login(server.port, "berti@ejemplo.com");

        let mut stream = connect(server.port);
        write!(
            stream,
            "GET /api/stream?conversation=7469057930 HTTP/1.1\r\nHost: jimmy\r\nCookie: {cookie}\r\n\r\n"
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
                .any(|line| line.contains("presence") && line.contains("berti")),
            "esperaba quién está mirando: {seen:?}"
        );
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn the_online_stream_knows_everyone_connected() {
        let server = start("online");
        let berti = login(server.port, "berti@ejemplo.com");

        let mut stream = connect(server.port);
        write!(
            stream,
            "GET /api/online HTTP/1.1\r\nHost: jimmy\r\nCookie: {berti}\r\n\r\n"
        )
        .unwrap();
        let mut lines = BufReader::new(stream.try_clone().unwrap()).lines();
        assert!(next_data(&mut lines, "\"online\"").contains("berti"));

        let ana = login(server.port, "ana@ejemplo.com");
        let mut second = connect(server.port);
        write!(
            second,
            "GET /api/online HTTP/1.1\r\nHost: jimmy\r\nCookie: {ana}\r\n\r\n"
        )
        .unwrap();
        let seen = next_data(&mut lines, "\"online\"");
        assert!(seen.contains("ana") && seen.contains("berti"), "{seen}");
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn typing_goes_live_but_is_not_written_down() {
        let server = start("typing");
        let cookie = login(server.port, "berti@ejemplo.com");
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
            r#"{"conversation":"7469057930"}"#,
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
        assert!(seen.contains("berti"), "{seen}");

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
    fn cancel_only_goes_to_a_conversation_you_can_write() {
        let server = start("cancel");
        let cookie = login(server.port, "berti@ejemplo.com");

        let refused = post_with(
            server.port,
            "/api/cancel",
            r#"{"conversation":"7469057930"}"#,
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
            log.contains("\"stopped\"") && log.contains("berti"),
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
        let cookie = login(server.port, "berti@ejemplo.com");

        let found = get(server.port, "/api/search?q=gato", Some(&cookie));
        assert!(found.contains("el gato duerme"), "{found}");
        assert!(found.contains("7469057930"), "{found}");
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
        let cookie = login(server.port, "berti@ejemplo.com");

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
            r#"{"conversation":"7469057930"}"#,
            Some(&cookie),
        );
        assert!(refused.starts_with("HTTP/1.1 400"), "{refused}");
        assert!(server.root.join("chats/7469057930").is_dir());
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn a_project_is_deleted_only_when_it_has_nothing_to_lose() {
        let server = start("delete-project");
        let cookie = login(server.port, "berti@ejemplo.com");
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
        let cookie = login(server.port, "berti@ejemplo.com");

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
        let _ = std::fs::remove_dir_all(server.root.parent().unwrap());
    }

    #[test]
    fn the_stream_skips_what_the_watcher_already_has() {
        let server = start("stream-since");
        let cookie = login(server.port, "berti@ejemplo.com");

        let mut stream = connect(server.port);
        write!(
            stream,
            "GET /api/stream?conversation=7469057930&since=1 HTTP/1.1\r\nHost: jimmy\r\nCookie: {cookie}\r\n\r\n"
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
}
