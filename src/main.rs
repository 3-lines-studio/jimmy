mod agent;
mod audio;
mod auth;
mod bus;
mod conversations;
mod files;
mod http;
mod log;
mod machine;
mod mail;
mod markdown;
mod media;
mod memlog;
mod memo;
mod pool;
mod preview;
mod prompt;
mod protocol;
mod random;
mod reap;
#[allow(dead_code)]
mod remote;
mod sandbox;
mod schedule;
mod skill;
mod store;
#[allow(dead_code)]
mod tensorlake;
mod tools;
mod transport;
mod ulid;
mod watch;
mod web;
mod worker;
mod workspace;

use agent::Agent;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;
use transport::slack;
use transport::telegram;
use transport::{Event, EventSource, Idle, Null, Session, Transport};

struct Config {
    api_key: String,
    transcribe_key: Option<String>,
    base: String,
    model: String,
    context_window: Option<usize>,
    root: PathBuf,
    workspace: String,
    prompt: String,
    vars: String,
    allowed: Vec<String>,
}

impl Config {
    fn from_env() -> Result<Self, String> {
        let api_key = env("OPENAI_API_KEY").ok_or("OPENAI_API_KEY no está configurado")?;
        let root = root_from_env();
        let workspace = workspace_from_env().display().to_string();
        let allowed = env("JIMMY_ALLOWED_USER_IDS")
            .or_else(|| env("TELEGRAM_ALLOWED_USER_IDS"))
            .map(|v| {
                v.split(',')
                    .map(|id| id.trim().to_string())
                    .filter(|id| !id.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            api_key,
            transcribe_key: env("TRANSCRIBE_API_KEY"),
            base: env("AXE_BASE").unwrap_or_else(|| "https://api.deepseek.com".into()),
            model: env("AXE_MODEL").unwrap_or_else(|| "deepseek-flash".into()),
            context_window: Some(
                env("AXE_CONTEXT_WINDOW")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1_000_000),
            ),
            root,
            workspace,
            prompt: env("JIMMY_PROMPT").unwrap_or_else(|| prompt::DEFAULT.into()),
            vars: env("JIMMY_VARS").unwrap_or_default(),
            allowed,
        })
    }
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

fn root_from_env() -> PathBuf {
    PathBuf::from(
        env("JIMMY_ROOT")
            .or_else(|| env("RAILWAY_VOLUME_MOUNT_PATH"))
            .unwrap_or_else(|| "/data".into()),
    )
}

fn workspace_from_env() -> PathBuf {
    let root = root_from_env();
    PathBuf::from(
        env("JIMMY_WORKSPACE").unwrap_or_else(|| root.join("workspace").display().to_string()),
    )
}

/// Los nombres que jimmy atiende como orden y no como arranque del bot.
const SUBCOMMANDS: [&str; 6] = [
    "memo",
    "send",
    "conversations",
    "skill",
    "preview",
    "worker",
];

/// Con argumentos, jimmy sólo hace lo que le pidieron. Un nombre que no conoce
/// no puede terminar arrancando el bot entero: sería una segunda instancia
/// escuchando el mismo puerto y robándole los mensajes a la que ya corre.
fn unknown_command(args: &[String]) -> Option<&str> {
    let first = args.first()?;
    (!SUBCOMMANDS.contains(&first.as_str())).then_some(first.as_str())
}

fn main() {
    axe::set_non_dumpable();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("memo") {
        std::process::exit(memo_command(&args[1..]));
    }
    if args.first().map(String::as_str) == Some("send") {
        std::process::exit(send_command(&args[1..]));
    }
    if args.first().map(String::as_str) == Some("conversations") {
        std::process::exit(conversations_command(&args[1..]));
    }
    if args.first().map(String::as_str) == Some("skill") {
        std::process::exit(skill_command(&args[1..]));
    }
    if args.first().map(String::as_str) == Some("preview") {
        std::process::exit(preview_command(&args[1..]));
    }
    if args.first().map(String::as_str) == Some("worker") {
        let code = match worker::run(args[1..].to_vec()) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("jimmy worker: {e}");
                1
            }
        };
        std::process::exit(code);
    }
    if let Some(name) = unknown_command(&args) {
        eprintln!("jimmy: no conozco el comando {name}");
        eprintln!("uso: jimmy [{}] ...", SUBCOMMANDS.join("|"));
        std::process::exit(2);
    }
    let config = match Config::from_env() {
        Ok(config) => config,
        Err(e) => {
            eprintln!("jimmy: {e}");
            std::process::exit(1);
        }
    };
    axe::sentinel::seed(&config.api_key);
    if transport_name() != "none" && config.allowed.is_empty() {
        eprintln!(
            "jimmy: atención: JIMMY_ALLOWED_USER_IDS está vacío, así que el bot no le contesta a nadie"
        );
    }
    for dir in ["", "notes", "projects", "files", "scratch", "state"] {
        std::fs::create_dir_all(Path::new(&config.workspace).join(dir)).ok();
    }
    let store = match store::Store::open(&config.root.join("jimmy.db")) {
        Ok(store) => Arc::new(store),
        Err(error) => {
            eprintln!("jimmy: no pude abrir la base del control plane: {error}");
            std::process::exit(1);
        }
    };
    let agent = match build_agent(&config, Some(store.clone())) {
        Ok(agent) => agent,
        Err(e) => {
            eprintln!("jimmy: {e}");
            std::process::exit(1);
        }
    };
    let transport = match transport_from_env() {
        Ok(transport) => transport,
        Err(e) => {
            eprintln!("jimmy: {e}");
            std::process::exit(1);
        }
    };
    let mut source = match source_from_env() {
        Ok(source) => source,
        Err(e) => {
            eprintln!("jimmy: {e}");
            std::process::exit(1);
        }
    };
    let workspace = PathBuf::from(config.workspace.clone());
    let previews = preview::Previews::new(Path::new(&config.workspace));
    let agenda = schedule::spawn(
        transport.clone(),
        agent.clone(),
        store,
        config.root.clone(),
        workspace.clone(),
    );
    serve_web(
        &config,
        agent.bus(),
        agent.clone(),
        agenda,
        previews.clone(),
    );
    let mut reaper = reap::Reaper::default();
    let mut watch = watch::Watch::default();
    let root = config.root.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(60));
        reaper.reap(std::time::Instant::now());
        memlog::sample(&workspace);
        let memory = machine::usage(&root).memory;
        let quiet = !pool::busy() && previews.list().is_empty();
        if watch.overdue(memory.used, quiet) {
            eprintln!(
                "jimmy: {} MB en el cgroup sin nadie corriendo: salgo para que me levanten",
                memory.used / 1024 / 1024
            );
            std::process::exit(1);
        }
    });
    install_sigterm();
    recover(&agent, transport.clone(), &config.root);
    eprintln!(
        "jimmy: iniciado (transport={} model={} base={} root={} workspace={})",
        transport_name(),
        config.model,
        config.base,
        config.root.display(),
        config.workspace
    );

    loop {
        let events = match source.recv() {
            Ok(events) => events,
            Err(e) => {
                eprintln!("jimmy: {e}");
                std::thread::sleep(Duration::from_secs(3));
                continue;
            }
        };
        for event in events {
            if event.is_bot {
                continue;
            }
            if !config.allowed.contains(&event.sender) {
                eprintln!("jimmy: ignoré un mensaje de {}", event.sender);
                continue;
            }
            if event.stop {
                eprintln!("jimmy: freno el turno de {}", event.session.key());
                agent.cancel(&event.session, &event.author);
                continue;
            }
            let agent = agent.clone();
            let transport = transport.clone();
            let transcribe_key = config.transcribe_key.clone();
            std::thread::spawn(move || {
                let Event {
                    session,
                    author,
                    text,
                    image,
                    voice,
                    ..
                } = event;
                let mut text = text;
                if let Some((file_id, duration)) = voice {
                    match transcribe_voice(
                        transport.as_ref(),
                        &session,
                        &file_id,
                        duration,
                        transcribe_key,
                    ) {
                        Ok(transcript) => {
                            transport.note(&session, &format!("🎤 {transcript}"));
                            text = transcript;
                        }
                        Err(e) => {
                            transport
                                .note(&session, &format!("⚠️ no pude transcribir el audio: {e}"));
                            return;
                        }
                    }
                }
                eprintln!("jimmy: {} -> {}", session.channel, clamp(&text, 80));
                if let Some(reply) = agent.command(&session, &text) {
                    transport.note(&session, &reply);
                    return;
                }
                if text.split_whitespace().next() == Some("/compact") {
                    if let Err(e) = agent.compact(transport.as_ref(), &session) {
                        eprintln!("jimmy: {} falló: {e}", session.channel);
                    }
                    return;
                }
                let images = match image {
                    Some(file_id) => match fetch_image(transport.as_ref(), &session, &file_id) {
                        Ok(image) => vec![image],
                        Err(e) => {
                            transport.note(&session, &format!("⚠️ no pude bajar la imagen: {e}"));
                            return;
                        }
                    },
                    None => Vec::new(),
                };
                if let Err(e) = agent.respond(transport.as_ref(), &session, &text, images, &author)
                {
                    eprintln!("jimmy: {} falló: {e}", session.channel);
                }
            });
        }
    }
}

/// Lo que hay en el volumen, en JSON: proyectos, conversaciones y lo que la
/// lista necesita saber de cada uno. Es lo que el control plane copia a su base
/// para poder mostrar la lista sin abrir el sandbox, y lo lee el mismo código
/// que arma la lista de acá, así no hay dos layouts.
fn index_json(root: &Path, workspace: &Path) -> Result<String, String> {
    let proyectos: Vec<serde_json::Value> = conversations::projects(root, workspace)
        .into_iter()
        .map(|proyecto| {
            let general = proyecto.name == conversations::GENERAL;
            let conversaciones: Vec<serde_json::Value> = proyecto
                .conversations
                .iter()
                .map(|conversacion| {
                    serde_json::json!({
                        "key": conversacion.key,
                        "project": conversacion.project,
                        "title": conversacion.title,
                        "read_only": conversacion.read_only,
                        "last": conversations::last_message(&conversacion.dir),
                        "touched_at": conversacion
                            .updated()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|desde| desde.as_secs())
                            .unwrap_or(0),
                    })
                })
                .collect();
            serde_json::json!({
                "name": proyecto.name,
                "size": if general { 0 } else { conversations::size(&proyecto.path) },
                "unversioned": crate::workspace::unversioned(&proyecto.path),
                "conversations": conversaciones,
            })
        })
        .collect();
    serde_json::to_string(&serde_json::json!({ "projects": proyectos })).map_err(|e| e.to_string())
}

fn conversations_command(args: &[String]) -> i32 {
    let root = root_from_env();
    let workspace = workspace_from_env();
    let result = match args.first().map(String::as_str) {
        Some("--json") => index_json(&root, &workspace).map(|texto| println!("{texto}")),
        Some("new") => match args.get(1) {
            Some(project) => {
                let title = args
                    .get(2)
                    .map(String::as_str)
                    .unwrap_or(conversations::NEW_TITLE);
                conversations::create(&root, &workspace, project, title)
                    .map(|key| println!("{key}"))
            }
            None => return usage(),
        },
        Some("rename") => match (args.get(1), args.get(2)) {
            (Some(key), Some(title)) => conversations::rename(&root, key, title),
            _ => return usage(),
        },
        Some(_) => return usage(),
        None => {
            for project in conversations::projects(&root, &workspace) {
                println!("{}  {}", project.name, project.path.display());
                for conversation in &project.conversations {
                    let title = conversation.title.as_deref().unwrap_or("(sin título)");
                    let mode = if conversation.read_only {
                        "solo lectura"
                    } else {
                        "web"
                    };
                    println!("  {} · {} · {title}", conversation.key, mode);
                }
            }
            Ok(())
        }
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("jimmy conversations: {e}");
            2
        }
    }
}

fn usage() -> i32 {
    eprintln!(
        "uso: jimmy conversations [--json | new <proyecto> [título] | rename <clave> <título>]"
    );
    2
}

/// The web frontend is opt-in: without a port to listen on, jimmy is what it
/// always was.
fn serve_web(
    config: &Config,
    bus: Arc<bus::Bus>,
    agent: Agent,
    agenda: Sender<()>,
    previews: Arc<preview::Previews>,
) {
    let Some(port) = env("JIMMY_WEB_PORT").and_then(|port| port.parse::<u16>().ok()) else {
        return;
    };
    let store = match store::Store::open(&config.root.join("jimmy.db")) {
        Ok(store) => Arc::new(store),
        Err(error) => {
            eprintln!("jimmy: no pude abrir la base del control plane: {error}");
            return;
        }
    };
    let auth = auth::Auth::new(
        store,
        &env("JIMMY_WEB_EMAILS").unwrap_or_default(),
        mail::Mail::from_env(),
        env("JIMMY_WEB_DEV").is_some_and(|value| value == "1"),
    );
    if auth.allowed().is_empty() {
        eprintln!("jimmy: no hay mails autorizados; poné JIMMY_WEB_EMAILS");
    } else if auth.mail.is_none() && !auth.dev {
        eprintln!(
            "jimmy: sin RESEND_API_KEY ni JIMMY_WEB_FROM nadie puede entrar; \
             JIMMY_WEB_DEV=1 devuelve el link en la respuesta"
        );
    }
    if auth.dev {
        eprintln!("jimmy: JIMMY_WEB_DEV=1: el link de entrada sale en la respuesta");
    }
    let web = web::Web::new(
        config.root.clone(),
        PathBuf::from(&config.workspace),
        bus,
        agent,
        auth,
        previews.clone(),
        agenda,
    );
    let listener = match web::listen(port) {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("jimmy: {e}");
            return;
        }
    };
    eprintln!("jimmy: web escuchando en el puerto {port}");
    std::thread::spawn(move || web::serve(web, listener));
    std::thread::spawn(move || preview::listen(previews));
}

fn build_agent(config: &Config, store: Option<Arc<store::Store>>) -> Result<Agent, String> {
    let mut vars = prompt::parse_vars(&config.vars);
    vars.push(("skills".into(), skill::index(&skills_dirs())));
    let fragments = prompt::assemble(&config.prompt, &prompt::dirs(&config.root), &vars)?;
    let mut agent = Agent::new(
        config.base.clone(),
        config.model.clone(),
        config.api_key.clone(),
        config.context_window,
        config.root.clone(),
        config.workspace.clone(),
        fragments,
    );
    if let Some(store) = store {
        agent.set_store(store);
    }
    Ok(agent)
}

pub(crate) fn flag(args: &[String], name: &str) -> Option<String> {
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == name {
            return it.next().cloned();
        }
    }
    None
}

fn transport_from_env() -> Result<Arc<dyn Transport>, String> {
    match transport_name().as_str() {
        "telegram" => Ok(Arc::new(telegram::Telegram::new(telegram_token()?))),
        "slack" => Ok(Arc::new(slack::Slack::new(
            slack_bot_token()?,
            slack_app_token()?,
        ))),
        "none" => Ok(Arc::new(Null)),
        other => Err(format!("transporte desconocido: {other}")),
    }
}

fn source_from_env() -> Result<Box<dyn EventSource>, String> {
    match transport_name().as_str() {
        "telegram" => Ok(Box::new(telegram::Updates::new(telegram::Telegram::new(
            telegram_token()?,
        )))),
        "slack" => Ok(Box::new(slack::Events::new(slack::Slack::new(
            slack_bot_token()?,
            slack_app_token()?,
        )))),
        "none" => Ok(Box::new(Idle)),
        other => Err(format!("transporte desconocido: {other}")),
    }
}

fn transport_name() -> String {
    env("JIMMY_TRANSPORT").unwrap_or_else(|| "none".into())
}

fn telegram_token() -> Result<String, String> {
    env("TELEGRAM_BOT_TOKEN").ok_or("TELEGRAM_BOT_TOKEN no está configurado".into())
}

fn slack_bot_token() -> Result<String, String> {
    env("SLACK_BOT_TOKEN").ok_or("SLACK_BOT_TOKEN no está configurado".into())
}

fn slack_app_token() -> Result<String, String> {
    env("SLACK_APP_TOKEN").ok_or("SLACK_APP_TOKEN no está configurado".into())
}

fn send_command(args: &[String]) -> i32 {
    let mut path = None;
    let mut target = None;
    let mut caption = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--target" => target = it.next().cloned(),
            "--caption" => caption = it.next().cloned(),
            other if path.is_none() => path = Some(other.to_string()),
            other => {
                eprintln!("jimmy send: argumento inesperado: {other}");
                return 2;
            }
        }
    }
    let Some(path) = path else {
        eprintln!("uso: jimmy send <archivo> --target TARGET [--caption TEXTO]");
        return 2;
    };
    let Some(target) = target.or_else(|| env("JIMMY_TARGET")) else {
        eprintln!("jimmy send: falta --target");
        return 2;
    };
    let root = root_from_env();
    let workspace = workspace_from_env();
    let conversation = conversations::get(&root, &workspace, &target);
    if !conversation.read_only && conversation.dir.is_dir() {
        return match keep(&conversation, &path, caption.as_deref()) {
            Ok(name) => {
                println!("{name}");
                0
            }
            Err(e) => {
                eprintln!("jimmy send: {e}");
                1
            }
        };
    }
    let transport = match transport_from_env() {
        Ok(transport) => transport,
        Err(e) => {
            eprintln!("jimmy send: {e}");
            return 1;
        }
    };
    let session = match transport.parse_target(&target) {
        Ok(session) => session,
        Err(e) => {
            eprintln!("jimmy send: {e}");
            return 1;
        }
    };
    let sent = transport.send_media(&session, Path::new(&path), caption.as_deref());
    if media::is_image(&path) {
        if let Err(e) = keep(&conversation, &path, caption.as_deref()) {
            eprintln!("jimmy send: la web no lo va a mostrar: {e}");
        }
    }
    match sent {
        Ok(msg) => {
            println!("{}", msg.0);
            0
        }
        Err(e) => {
            eprintln!("jimmy send: {e}");
            1
        }
    }
}

/// Una imagen que va a una conversación de la web no sale por ningún transporte:
/// queda en el chat, que es donde se mira. Y la que sale por Telegram también
/// queda, porque la web muestra las mismas conversaciones.
fn keep(
    conversation: &conversations::Conversation,
    path: &str,
    caption: Option<&str>,
) -> Result<String, String> {
    if !media::is_image(path) {
        return Err("sólo sé mostrar imágenes: png, jpg, gif o webp".into());
    }
    let bytes = std::fs::read(path).map_err(|e| format!("no pude leer {path}: {e}"))?;
    let base = Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = media::store(&media::dir(conversation), &base, &bytes)?;
    media::queue(
        conversation,
        &protocol::Event::Image {
            name: name.clone(),
            caption: caption.unwrap_or_default().to_string(),
        },
    )?;
    Ok(name)
}

fn skills_dirs() -> Vec<PathBuf> {
    let local = match env("JIMMY_SKILLS") {
        Some(path) => PathBuf::from(path),
        None => root_from_env().join("skills"),
    };
    skill::dirs(local)
}

fn skill_command(args: &[String]) -> i32 {
    let dirs = skills_dirs();
    match args.first().map(String::as_str) {
        Some("list") => {
            println!("{}", skill::list(&dirs));
            0
        }
        Some("load") => match args.get(1) {
            Some(name) => match skill::load(&dirs, name) {
                Ok(text) => {
                    println!("{text}");
                    0
                }
                Err(e) => {
                    eprintln!("jimmy skill: {e}");
                    1
                }
            },
            None => {
                eprintln!("uso: jimmy skill load <nombre>");
                2
            }
        },
        _ => {
            eprintln!("uso: jimmy skill <list|load nombre>");
            2
        }
    }
}

fn memo_command(args: &[String]) -> i32 {
    let workspace = workspace_from_env();
    let result = match args.first().map(String::as_str) {
        Some("sync") => memo::sync(&workspace),
        Some("list") => Ok(memo::list(&workspace)),
        Some("render") => Ok(memo::render(
            &workspace,
            args.get(1).map(String::as_str).unwrap_or(""),
        )),
        Some("add") => match (args.get(1), args.get(2), args.get(3..)) {
            (Some(key), Some(kind), Some(rest)) if !rest.is_empty() => {
                memo::add(&workspace, key, kind, &rest.join(" "))
            }
            _ => Err("memo add: uso `jimmy memo add <clave> <tipo> <texto>`".into()),
        },
        Some("miss") => match args[1..].join(" ").trim() {
            "" => Err("memo miss: falta el texto".into()),
            text => memo::miss(&workspace, text),
        },
        Some("show") => match args[1..].join(" ").trim() {
            "" => Err("memo show: falta la clave".into()),
            key => memo::show(&workspace, key),
        },
        _ => Err(
            "uso: jimmy memo <sync|list|render [proyecto]|add clave tipo texto|miss texto|show clave>"
                .into(),
        ),
    };
    match result {
        Ok(report) => {
            println!("{report}");
            0
        }
        Err(e) => {
            eprintln!("jimmy memo: {e}");
            1
        }
    }
}

fn transcribe_voice(
    transport: &dyn Transport,
    session: &Session,
    file_id: &str,
    duration: u64,
    api_key: Option<String>,
) -> Result<String, String> {
    let Some(api_key) = api_key else {
        return Err("falta TRANSCRIBE_API_KEY".into());
    };
    let (file_path, data) = transport.download(file_id)?;
    let ext = audio_extension(&file_path);
    let name = format!(
        "jimmy-voice-{}-{}.{ext}",
        session.channel,
        axe::session::now_ms()
    );
    let path = std::env::temp_dir().join(name);
    std::fs::write(&path, &data).map_err(|e| e.to_string())?;
    let result = audio::transcribe(&api_key, &path, duration);
    let _ = std::fs::remove_file(&path);
    result
}

fn audio_extension(file_path: &str) -> &str {
    Path::new(file_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .filter(|ext| *ext != "oga")
        .unwrap_or("ogg")
}

fn fetch_image(
    transport: &dyn Transport,
    session: &Session,
    file_id: &str,
) -> Result<axe::Image, String> {
    let (file_path, data) = transport.download(file_id)?;
    let ext = Path::new(&file_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("jpg");
    let name = format!(
        "jimmy-image-{}-{}.{ext}",
        session.channel,
        axe::session::now_ms()
    );
    let path = std::env::temp_dir().join(name);
    std::fs::write(&path, &data).map_err(|e| e.to_string())?;
    axe::image::attach(&path.display().to_string())
}

fn recover(agent: &Agent, transport: Arc<dyn Transport>, root: &Path) {
    for key in inflight_chats(root) {
        let Some(session) = session_from_key(&key) else {
            continue;
        };
        eprintln!("jimmy: reanudando el turno de {key}");
        let agent = agent.clone();
        let transport = transport.clone();
        std::thread::spawn(move || {
            if let Err(e) = agent.resume(transport.as_ref(), &session) {
                eprintln!("jimmy: no pude reanudar {key}: {e}");
            }
        });
    }
}

/// Chats with a turn that was cut short, which is what the marker file means.
fn inflight_chats(root: &Path) -> Vec<String> {
    let Ok(chats) = std::fs::read_dir(root.join("chats")) else {
        return Vec::new();
    };
    let mut keys: Vec<String> = chats
        .flatten()
        .filter(|entry| entry.path().join("inflight").exists())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    keys.sort();
    keys
}

fn session_from_key(key: &str) -> Option<Session> {
    match key.split_once('/') {
        Some((channel, thread)) => Some(Session {
            channel: channel.into(),
            thread: Some(thread.into()),
        }),
        None => Some(Session {
            channel: key.into(),
            thread: None,
        }),
    }
}

fn preview_command(args: &[String]) -> i32 {
    let workspace = workspace_from_env();
    let order = match args.first().map(String::as_str) {
        Some("start") => {
            let Some(name) = args.get(1) else {
                eprintln!("uso: jimmy preview start <nombre> [--cmd 'comando'] [--cwd dir]");
                return 2;
            };
            preview::Order {
                op: "start".into(),
                name: name.clone(),
                command: flag(args, "--cmd"),
                cwd: flag(args, "--cwd"),
            }
        }
        Some("stop") => {
            let Some(name) = args.get(1) else {
                eprintln!("uso: jimmy preview stop <nombre>");
                return 2;
            };
            preview::Order {
                op: "stop".into(),
                name: name.clone(),
                ..preview::Order::default()
            }
        }
        Some("recipes") => preview::Order {
            op: "recipes".into(),
            ..preview::Order::default()
        },
        Some("list") | None => preview::Order {
            op: "list".into(),
            ..preview::Order::default()
        },
        _ => {
            eprintln!(
                "uso: jimmy preview <start nombre [--cmd 'comando'] [--cwd dir]|stop nombre|list|recipes>"
            );
            return 2;
        }
    };
    let answer = match preview::call(&workspace, &order) {
        Ok(answer) => answer,
        Err(e) => {
            eprintln!("jimmy preview: {e}");
            return 1;
        }
    };
    if !answer.ok {
        eprintln!(
            "jimmy preview: {}",
            answer.error.unwrap_or_else(|| "no pude".into())
        );
        return 1;
    }
    if let Some(output) = answer.output {
        println!("{output}");
    }
    for preview in answer.previews {
        println!(
            "{} puerto {} pid {} en /preview/{}/ (arriba hace {}s, sin visitas hace {}s)",
            preview.name, preview.port, preview.pid, preview.name, preview.seconds, preview.idle
        );
    }
    for recipe in answer.recipes {
        println!(
            "{}{}",
            recipe.name,
            recipe
                .about
                .map(|about| format!(" — {about}"))
                .unwrap_or_default()
        );
        println!("    {}", recipe.cmd.trim());
    }
    0
}

fn install_sigterm() {
    unsafe { libc::signal(libc::SIGTERM, handle_sigterm as *const () as usize) };
}

extern "C" fn handle_sigterm(_: libc::c_int) {
    pool::kill_all();
    axe::tools::kill_children();
    unsafe { libc::_exit(0) };
}

fn clamp(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El índice que lee el control plane desde el sandbox sale con lo que la
    /// lista necesita, y lo arma el mismo código que la arma acá.
    #[test]
    fn el_indice_de_la_org_sale_con_lo_que_la_lista_necesita() {
        let base = std::env::temp_dir().join(format!("jimmy-indice-{}", crate::random::hex(4)));
        let root = base.join("root");
        let workspace = base.join("workspace");
        std::fs::create_dir_all(workspace.join("projects/ken")).unwrap();
        std::fs::write(workspace.join("projects/ken/nota.md"), "hola").unwrap();
        std::fs::create_dir_all(root.join("chats/web-1")).unwrap();
        std::fs::write(
            root.join("chats/web-1/meta.json"),
            r#"{"project":"ken","title":"una charla"}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("chats/web-1/transcript.jsonl"),
            "{\"type\":\"message\",\"message\":{\"Role\":\"assistant\",\"Content\":\"listo\"}}\n",
        )
        .unwrap();

        let json: serde_json::Value =
            serde_json::from_str(&index_json(&root, &workspace).unwrap()).unwrap();
        let proyectos = json["projects"].as_array().unwrap();
        assert_eq!(proyectos.len(), 2, "general y ken: {proyectos:?}");
        assert_eq!(proyectos[0]["name"], "general");
        let ken = proyectos.iter().find(|p| p["name"] == "ken").unwrap();
        assert_eq!(ken["unversioned"], true);
        assert!(ken["size"].as_u64().unwrap() > 0);
        let charla = &ken["conversations"][0];
        assert_eq!(charla["key"], "web-1");
        assert_eq!(charla["project"], "ken");
        assert_eq!(charla["title"], "una charla");
        assert_eq!(charla["read_only"], false);
        assert_eq!(charla["last"], "listo");
        assert!(charla["touched_at"].as_u64().unwrap() > 0);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn an_unknown_command_never_starts_the_bot() {
        assert_eq!(unknown_command(&[]), None);
        assert_eq!(unknown_command(&["memo".into(), "sync".into()]), None);
        assert_eq!(unknown_command(&["worker".into(), "--chat".into()]), None);
        assert_eq!(unknown_command(&["stats".into()]), Some("stats"));
        assert_eq!(unknown_command(&["--help".into()]), Some("--help"));
    }

    #[test]
    fn telegram_voice_extension_is_ogg_not_oga() {
        assert_eq!(audio_extension("voice/file_12.oga"), "ogg");
    }

    #[test]
    fn other_audio_extensions_pass_through() {
        assert_eq!(audio_extension("audio/song.mp3"), "mp3");
        assert_eq!(audio_extension("audio/no-extension"), "ogg");
    }

    #[test]
    fn session_key_round_trips() {
        assert_eq!(
            session_from_key("123456789").unwrap(),
            Session::channel("123456789")
        );
        assert_eq!(
            session_from_key("C1/1699.1").unwrap(),
            Session {
                channel: "C1".into(),
                thread: Some("1699.1".into())
            }
        );
    }

    #[test]
    fn an_image_for_a_web_conversation_is_kept_and_queued() {
        let dir = std::env::temp_dir().join(format!("jimmy-send-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let image = dir.join("circulo.png");
        std::fs::write(&image, b"\x89PNG\r\n\x1a\nlos bytes").unwrap();
        let conversation = conversations::Conversation {
            key: "web-1".into(),
            dir: dir.join("chats/web-1"),
            cwd: dir.clone(),
            project: "general".into(),
            title: None,
            read_only: false,
        };

        let name = keep(&conversation, image.to_str().unwrap(), Some("el círculo")).unwrap();
        assert!(name.ends_with("-circulo.png"), "{name}");
        let kept = std::fs::read(media::dir(&conversation).join(&name)).unwrap();
        assert_eq!(kept, b"\x89PNG\r\n\x1a\nlos bytes");

        let queued = media::drain(&conversation);
        assert_eq!(queued.len(), 1);
        assert!(
            matches!(&queued[0], protocol::Event::Image { name: kept, caption } if kept == &name && caption == "el círculo")
        );

        let other = dir.join("informe.pdf");
        assert!(keep(&conversation, other.to_str().unwrap(), None).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn inflight_chats_lists_only_the_ones_with_a_marker() {
        let root = std::env::temp_dir().join(format!("jimmy-inflight-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("chats/123")).unwrap();
        std::fs::create_dir_all(root.join("chats/456")).unwrap();
        std::fs::write(root.join("chats/123/inflight"), b"").unwrap();
        assert_eq!(inflight_chats(&root), ["123"]);
        std::fs::write(root.join("chats/456/inflight"), b"").unwrap();
        assert_eq!(inflight_chats(&root), ["123", "456"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_default_spec_assembles_with_the_vars_the_agent_gets() {
        let mut vars = prompt::parse_vars("usuario=Ana,asistente=Jimmy");
        vars.push(("skills".into(), "browse — Nav".into()));
        let out = prompt::assemble(prompt::DEFAULT, &prompt::dirs(Path::new(".")), &vars).unwrap();
        assert!(out.contains("browse — Nav"));
    }
}
