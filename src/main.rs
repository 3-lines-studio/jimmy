mod agent;
mod audio;
mod auth;
mod bus;
mod conversations;
mod files;
mod http;
mod keepalive;
mod log;
mod machine;
mod mail;
mod markdown;
mod mcp;
mod media;
mod oauth;
mod pool;
mod preview;
mod protocol;
mod random;
mod schedule;
mod servers;
mod skill;
mod store;
mod tools;
mod transport;
mod vault;
mod web;
mod worker;

use agent::Agent;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;
use transport::slack;
use transport::telegram;
use transport::{Event, EventSource, Idle, Null, Session, Transport};

const SYSTEM: &str = include_str!("../prompts/jimmy.md");

struct Config {
    api_key: String,
    base: String,
    model: String,
    context_window: Option<usize>,
    root: PathBuf,
    workspace: String,
    allowed: Vec<String>,
    transcribe_key: Option<String>,
}

impl Config {
    fn from_env() -> Result<Self, String> {
        let api_key = env("OPENAI_API_KEY").ok_or("OPENAI_API_KEY no está configurado")?;
        let root = root_from_env();
        let workspace = workspace_from_env().display().to_string();
        let allowed = env("JIMMY_ALLOWED_USER_IDS")
            .map(|v| {
                v.split(',')
                    .map(|id| id.trim().to_string())
                    .filter(|id| !id.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            api_key,
            base: env("AXE_BASE").unwrap_or_else(|| "https://api.deepseek.com".into()),
            model: env("AXE_MODEL").unwrap_or_else(|| "deepseek-flash".into()),
            context_window: Some(
                env("AXE_CONTEXT_WINDOW")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1_000_000),
            ),
            root,
            workspace,
            allowed,
            transcribe_key: env("TRANSCRIBE_API_KEY"),
        })
    }
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

fn invoked_as() -> Option<String> {
    let name = Path::new(&std::env::args().next()?)
        .file_name()?
        .to_string_lossy()
        .into_owned();
    matches!(name.as_str(), "heimdall" | "doppler").then_some(name)
}

fn heimdall_command(name: &str) -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if env("HEIMDALL_DIR").is_none() {
        std::env::set_var("HEIMDALL_DIR", vault::dir(&root_from_env()));
    }
    match heimdall::cli::run(&args) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{name}: {e}");
            1
        }
    }
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

const SUBCOMMANDS: [&str; 5] = ["send", "conversations", "skill", "preview", "worker"];

fn unknown_command(args: &[String]) -> Option<&str> {
    let first = args.first()?;
    (!SUBCOMMANDS.contains(&first.as_str())).then_some(first.as_str())
}

fn main() {
    axe::set_non_dumpable();
    if let Some(name) = invoked_as() {
        std::process::exit(heimdall_command(&name));
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
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
    let agent = match build_agent(&config) {
        Ok(agent) => agent,
        Err(e) => {
            eprintln!("jimmy: {e}");
            std::process::exit(1);
        }
    };
    keepalive::spawn(agent.clone());
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
    let previews = preview::Previews::new(Path::new(&config.workspace));
    let agenda = schedule::spawn(
        transport.clone(),
        agent.clone(),
        PathBuf::from(config.workspace.clone()),
    );
    vault::open(&config.root);
    serve_web(
        &config,
        agent.bus(),
        agent.clone(),
        agenda,
        previews.clone(),
    );
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
                    Some(file_id) => {
                        match fetch_image(&agent, transport.as_ref(), &session, &file_id) {
                            Ok(image) => vec![image],
                            Err(e) => {
                                transport
                                    .note(&session, &format!("⚠️ no pude bajar la imagen: {e}"));
                                return;
                            }
                        }
                    }
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

fn conversations_command(args: &[String]) -> i32 {
    let root = root_from_env();
    let workspace = workspace_from_env();
    let result = match args.first().map(String::as_str) {
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
    eprintln!("uso: jimmy conversations [new <proyecto> [título] | rename <clave> <título>]");
    2
}

fn serve_web(
    config: &Config,
    bus: Arc<bus::Bus>,
    agent: Agent,
    agenda: Sender<String>,
    previews: Arc<preview::Previews>,
) {
    let Some(port) = env("JIMMY_WEB_PORT").and_then(|port| port.parse::<u16>().ok()) else {
        return;
    };
    let auth = auth::Auth::new(
        &env("JIMMY_WEB_EMAILS").unwrap_or_default(),
        &config.root,
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
        web::Paths {
            root: config.root.clone(),
            workspace: PathBuf::from(&config.workspace),
        },
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
    std::thread::Builder::new()
        .name("web".into())
        .spawn(move || web::serve(web, listener))
        .expect("failed to spawn thread");
    std::thread::spawn(move || preview::listen(previews));
}

fn build_agent(config: &Config) -> Result<Agent, String> {
    Ok(Agent::new(
        config.base.clone(),
        config.model.clone(),
        config.api_key.clone(),
        config.context_window,
        config.root.clone(),
        config.workspace.clone(),
        SYSTEM.to_string(),
    ))
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
    agent: &Agent,
    transport: &dyn Transport,
    session: &Session,
    file_id: &str,
) -> Result<axe::Image, String> {
    let (file_path, data) = transport.download(file_id)?;
    let ext = Path::new(&file_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("jpg");
    let fallback = format!("jimmy-{}-{}.{ext}", session.channel, axe::session::now_ms());
    let name = Path::new(&file_path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .unwrap_or(fallback);
    let conversation = agent.conversation(session);
    save_image(&media::dir(&conversation), &name, &data)
}

fn save_image(dir: &Path, name: &str, data: &[u8]) -> Result<axe::Image, String> {
    let name = media::store(dir, name, data)?;
    let path = dir.join(&name);
    axe::image::attach_data(&path.display().to_string(), data)
}

fn recover(agent: &Agent, transport: Arc<dyn Transport>, root: &Path) {
    for key in conversations::chats(root) {
        heal_projection(&conversations::chat_dir(root, &key));
    }
    for key in inflight_chats(root) {
        let Some(session) = session_from_key(&key) else {
            continue;
        };
        let dir = agent.conversation(&session).dir;
        match store::recover_turn(&dir) {
            Ok(store::Recovered::Resume) => {
                eprintln!("jimmy: reanudando el turno de {key}");
                let agent = agent.clone();
                let transport = transport.clone();
                std::thread::spawn(move || {
                    if let Err(e) = agent.resume(transport.as_ref(), &session) {
                        eprintln!("jimmy: no pude reanudar {key}: {e}");
                    }
                });
            }
            Ok(store::Recovered::Blocked(limit)) => {
                let _ = std::fs::remove_file(dir.join("inflight"));
                eprintln!("jimmy: {key} quedó bloqueado tras {limit} reinicios");
                transport.note(
                    &session,
                    &format!(
                        "⚠️ el turno quedó bloqueado tras {limit} reinicios. Mandá otro mensaje."
                    ),
                );
            }
            Ok(store::Recovered::Nothing) => {
                let _ = std::fs::remove_file(dir.join("inflight"));
            }
            Err(e) => eprintln!("jimmy: no pude recuperar {key}: {e}"),
        }
    }
}

fn heal_projection(dir: &Path) {
    use axe::durable::{Journal, TaskState};
    use axe::session::Entry;

    let stamp = |name: &str| {
        std::fs::metadata(dir.join(name))
            .and_then(|meta| meta.modified())
            .ok()
    };
    let Some(durable) = [stamp("journal.jsonl"), stamp("state.json")]
        .into_iter()
        .flatten()
        .max()
    else {
        return;
    };
    if stamp("conversation.jsonl").is_some_and(|log| log >= durable) {
        return;
    }
    let Ok(tasks) = Journal::load_tasks(dir) else {
        return;
    };
    let Some(turn) = tasks
        .iter()
        .filter(|task| task.kind == "turn")
        .max_by_key(|task| task.id)
    else {
        return;
    };
    if turn.state != TaskState::Done {
        return;
    }
    let Ok(records) = Journal::load(dir) else {
        return;
    };
    let Some(answer) = records.iter().rev().find_map(|record| match &record.entry {
        Entry::Message { message }
            if message.role == "assistant" && !message.content.trim().is_empty() =>
        {
            Some(message.content.clone())
        }
        _ => None,
    }) else {
        return;
    };
    let log = crate::log::Log::in_dir(dir);
    if log.last_answer().as_deref() == Some(answer.as_str()) {
        return;
    }
    log.append(&crate::protocol::Event::Done { text: answer });
}

fn inflight_chats(root: &Path) -> Vec<String> {
    let mut keys = Vec::new();
    walk_inflight(&root.join("chats"), "", &mut keys);
    keys.sort();
    keys
}

fn walk_inflight(dir: &Path, prefix: &str, keys: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let key = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if path.join("inflight").exists() {
            keys.push(key);
            continue;
        }
        if conversations::is_conversation(&path) {
            continue;
        }
        walk_inflight(&path, &key, keys);
    }
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

    #[test]
    fn an_unknown_command_never_starts_the_bot() {
        assert_eq!(unknown_command(&[]), None);
        assert_eq!(unknown_command(&["send".into(), "x".into()]), None);
        assert_eq!(unknown_command(&["worker".into(), "--chat".into()]), None);
        assert_eq!(unknown_command(&["stats".into()]), Some("stats"));
        assert_eq!(unknown_command(&["--help".into()]), Some("--help"));
    }

    #[test]
    fn a_turn_missing_from_the_log_is_healed() {
        let root = std::env::temp_dir().join(format!("jimmy-heal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("chats/x");
        std::fs::create_dir_all(&dir).unwrap();
        crate::log::Log::in_dir(&dir).append(&crate::protocol::Event::User {
            text: "hola".into(),
            author: String::new(),
            images: Vec::new(),
        });
        {
            let mut journal = crate::store::open(&dir).unwrap();
            let mut task = crate::store::new_turn(&mut journal, "");
            task.state = axe::durable::TaskState::Done;
            let message = axe::Message {
                role: "assistant".into(),
                content: "listo".into(),
                tool_calls: Vec::new(),
                tool_call_id: String::new(),
                reasoning: String::new(),
                images: Vec::new(),
            };
            journal
                .commit(vec![axe::session::Entry::Message { message }], vec![task])
                .unwrap();
        }

        heal_projection(&dir);
        heal_projection(&dir);

        assert_eq!(
            crate::log::Log::in_dir(&dir).last_answer().as_deref(),
            Some("listo")
        );
        assert_eq!(conversations::last_message(&dir).as_deref(), Some("listo"));
        let log = std::fs::read_to_string(dir.join("conversation.jsonl")).unwrap();
        assert_eq!(log.matches("\"done\"").count(), 1, "{log}");
        std::fs::remove_dir_all(&root).unwrap();
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
    fn inflight_chats_reaches_a_slack_thread() {
        let root =
            std::env::temp_dir().join(format!("jimmy-inflight-slack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("chats/C01/1726570000.123")).unwrap();
        std::fs::create_dir_all(root.join("chats/C01/1726570000.999/uploads")).unwrap();
        std::fs::write(root.join("chats/C01/1726570000.123/meta.json"), "{}").unwrap();
        std::fs::write(root.join("chats/C01/1726570000.123/inflight"), b"").unwrap();
        std::fs::write(root.join("chats/C01/1726570000.999/meta.json"), "{}").unwrap();
        std::fs::write(root.join("chats/C01/1726570000.999/uploads/inflight"), b"").unwrap();

        assert_eq!(inflight_chats(&root), ["C01/1726570000.123"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_system_prompt_carries_no_placeholders() {
        assert!(SYSTEM.contains("Never answer in Chinese."));
        assert!(!SYSTEM.contains("{{"));
    }
}
