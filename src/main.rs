mod agent;
mod audio;
mod markdown;
mod memo;
mod prompt;
mod schedule;
mod telegram;

use agent::Agent;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use telegram::Telegram;

struct Config {
    token: String,
    api_key: String,
    transcribe_key: Option<String>,
    base: String,
    model: String,
    context_window: Option<usize>,
    root: PathBuf,
    workspace: String,
    prompt: String,
    vars: String,
    allowed: Vec<i64>,
}

impl Config {
    fn from_env() -> Result<Self, String> {
        let token = env("TELEGRAM_BOT_TOKEN").ok_or("TELEGRAM_BOT_TOKEN no está configurado")?;
        let api_key = env("OPENAI_API_KEY").ok_or("OPENAI_API_KEY no está configurado")?;
        let root = root_from_env();
        let workspace = workspace_from_env().display().to_string();
        let allowed = env("TELEGRAM_ALLOWED_USER_IDS")
            .map(|v| {
                v.split(',')
                    .filter_map(|id| id.trim().parse::<i64>().ok())
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            token,
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
    let tg = Telegram::new(config.token.clone());
    schedule::spawn(
        tg.clone(),
        agent.clone(),
        PathBuf::from(config.workspace.clone()),
    );
    eprintln!(
        "jimmy: iniciado (model={} base={} root={} workspace={})",
        config.model,
        config.base,
        config.root.display(),
        config.workspace
    );

    let mut offset = 0i64;
    loop {
        let updates = match tg.get_updates(offset) {
            Ok(updates) => updates,
            Err(e) => {
                eprintln!("jimmy: getUpdates: {e}");
                std::thread::sleep(Duration::from_secs(3));
                continue;
            }
        };
        for update in updates {
            offset = update.update_id + 1;
            let Some(message) = update.message else {
                continue;
            };
            if message.from.as_ref().is_some_and(|from| from.is_bot) {
                continue;
            }
            if !config.allowed.is_empty()
                && !message
                    .from
                    .as_ref()
                    .is_some_and(|from| config.allowed.contains(&from.id))
            {
                eprintln!(
                    "jimmy: ignoré un mensaje de {}",
                    message.from.map(|f| f.id).unwrap_or(0)
                );
                continue;
            }
            let chat_id = message.chat.id;
            let text = message
                .text
                .clone()
                .or_else(|| message.caption.clone())
                .unwrap_or_default();
            let file_id = message
                .photo
                .last()
                .map(|photo| photo.file_id.clone())
                .or_else(|| {
                    message
                        .document
                        .as_ref()
                        .filter(|document| document.mime_type.starts_with("image/"))
                        .map(|document| document.file_id.clone())
                });
            let voice = message
                .voice
                .as_ref()
                .map(|voice| (voice.file_id.clone(), voice.duration));
            if text.is_empty() && file_id.is_none() && voice.is_none() {
                continue;
            }
            let agent = agent.clone();
            let tg = tg.clone();
            let transcribe_key = config.transcribe_key.clone();
            std::thread::spawn(move || {
                let lock = chat_lock(chat_id);
                let _guard = lock.lock().unwrap();
                let mut text = text;
                if let Some((file_id, duration)) = voice {
                    match transcribe_voice(&tg, chat_id, &file_id, duration, transcribe_key) {
                        Ok(transcript) => {
                            let _ = tg.send_message(chat_id, &format!("🎤 {transcript}"));
                            text = transcript;
                        }
                        Err(e) => {
                            let _ = tg.send_message(
                                chat_id,
                                &format!("⚠️ no pude transcribir el audio: {e}"),
                            );
                            return;
                        }
                    }
                }
                eprintln!("jimmy: chat {chat_id} -> {}", clamp(&text, 80));
                if let Some(reply) = agent.command(chat_id, &text) {
                    if let Err(e) = tg.send_message(chat_id, &reply) {
                        eprintln!("jimmy: chat {chat_id} falló: {e}");
                    }
                    return;
                }
                let images = match file_id {
                    Some(file_id) => match fetch_image(&tg, chat_id, &file_id) {
                        Ok(image) => vec![image],
                        Err(e) => {
                            let _ = tg
                                .send_message(chat_id, &format!("⚠️ no pude bajar la imagen: {e}"));
                            return;
                        }
                    },
                    None => Vec::new(),
                };
                if let Err(e) = agent.respond(&tg, chat_id, &text, images) {
                    eprintln!("jimmy: chat {chat_id} falló: {e}");
                }
            });
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
    tg: &Telegram,
    chat_id: i64,
    file_id: &str,
    duration: u64,
    api_key: Option<String>,
) -> Result<String, String> {
    let Some(api_key) = api_key else {
        return Err("falta TRANSCRIBE_API_KEY".into());
    };
    let file_path = tg.get_file(file_id)?;
    let data = tg.download(&file_path)?;
    let ext = audio_extension(&file_path);
    let name = format!("jimmy-voice-{chat_id}-{}.{ext}", axe::session::now_ms());
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

fn fetch_image(tg: &Telegram, chat_id: i64, file_id: &str) -> Result<axe::Image, String> {
    let file_path = tg.get_file(file_id)?;
    let data = tg.download(&file_path)?;
    let ext = Path::new(&file_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("jpg");
    let name = format!("jimmy-image-{chat_id}-{}.{ext}", axe::session::now_ms());
    let path = std::env::temp_dir().join(name);
    std::fs::write(&path, &data).map_err(|e| e.to_string())?;
    axe::image::attach(&path.display().to_string())
}

fn chat_lock(chat_id: i64) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<i64, Arc<Mutex<()>>>>> = OnceLock::new();
    let locks = LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = locks.lock().unwrap();
    map.entry(chat_id)
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
