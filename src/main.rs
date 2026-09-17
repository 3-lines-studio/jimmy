mod agent;
mod audio;
mod markdown;
mod memo;
mod prompt;
mod schedule;
mod skill;
mod transport;

use agent::Agent;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use transport::slack;
use transport::telegram;
use transport::{Event, EventSource, Session, Transport};

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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("memo") {
        std::process::exit(memo_command(&args[1..]));
    }
    if args.first().map(String::as_str) == Some("send") {
        std::process::exit(send_command(&args[1..]));
    }
    if args.first().map(String::as_str) == Some("skill") {
        std::process::exit(skill_command(&args[1..]));
    }
    let config = match Config::from_env() {
        Ok(config) => config,
        Err(e) => {
            eprintln!("jimmy: {e}");
            std::process::exit(1);
        }
    };
    if config.allowed.is_empty() {
        eprintln!(
            "jimmy: atención: TELEGRAM_ALLOWED_USER_IDS está vacío, cualquiera puede usar el bot"
        );
    }
    for dir in ["", "notes", "projects", "files", "scratch", "state"] {
        std::fs::create_dir_all(Path::new(&config.workspace).join(dir)).ok();
    }
    let fragments = match prompt::assemble(
        &config.prompt,
        &prompt::dirs(&config.root),
        &prompt::parse_vars(&config.vars),
    ) {
        Ok(fragments) => fragments,
        Err(e) => {
            eprintln!("jimmy: {e}");
            std::process::exit(1);
        }
    };
    let agent = Agent::new(
        config.base.clone(),
        config.model.clone(),
        config.api_key.clone(),
        config.context_window,
        config.root.clone(),
        config.workspace.clone(),
        fragments,
    );
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
    schedule::spawn(
        transport.clone(),
        agent.clone(),
        PathBuf::from(config.workspace.clone()),
    );
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
            if !config.allowed.is_empty() && !config.allowed.contains(&event.sender) {
                eprintln!("jimmy: ignoré un mensaje de {}", event.sender);
                continue;
            }
            let agent = agent.clone();
            let transport = transport.clone();
            let transcribe_key = config.transcribe_key.clone();
            std::thread::spawn(move || {
                let Event {
                    session,
                    text,
                    image,
                    voice,
                    ..
                } = event;
                let lock = chat_lock(&session.key());
                let _guard = lock.lock().unwrap();
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
                if let Err(e) = agent.respond(transport.as_ref(), &session, &text, images) {
                    eprintln!("jimmy: {} falló: {e}", session.channel);
                }
            });
        }
    }
}

fn transport_from_env() -> Result<Arc<dyn Transport>, String> {
    match transport_name().as_str() {
        "telegram" => Ok(Arc::new(telegram::Telegram::new(telegram_token()?))),
        "slack" => Ok(Arc::new(slack::Slack::new(
            slack_bot_token()?,
            slack_app_token()?,
        ))),
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
        other => Err(format!("transporte desconocido: {other}")),
    }
}

fn transport_name() -> String {
    env("JIMMY_TRANSPORT").unwrap_or_else(|| "telegram".into())
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
    match transport.send_media(&session, Path::new(&path), caption.as_deref()) {
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

fn skills_dir() -> PathBuf {
    match env("JIMMY_SKILLS") {
        Some(path) => PathBuf::from(path),
        None => root_from_env().join("skills"),
    }
}

fn skill_command(args: &[String]) -> i32 {
    let dir = skills_dir();
    match args.first().map(String::as_str) {
        Some("list") => {
            println!("{}", skill::list(&dir));
            0
        }
        Some("load") => match args.get(1) {
            Some(name) => match skill::load(&dir, name) {
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
        Some("demote") => memo::demote(&workspace),
        Some("miss") => match args[1..].join(" ").trim() {
            "" => Err("memo miss: falta el texto".into()),
            text => memo::miss(&workspace, text),
        },
        _ => Err("uso: jimmy memo <sync|demote|miss texto>".into()),
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

fn chat_lock(key: &str) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    let locks = LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = locks.lock().unwrap();
    map.entry(key.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
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
    fn telegram_voice_extension_is_ogg_not_oga() {
        assert_eq!(audio_extension("voice/file_12.oga"), "ogg");
    }

    #[test]
    fn other_audio_extensions_pass_through() {
        assert_eq!(audio_extension("audio/song.mp3"), "mp3");
        assert_eq!(audio_extension("audio/no-extension"), "ogg");
    }
}
