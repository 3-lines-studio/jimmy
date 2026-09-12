mod agent;
mod telegram;

use agent::Agent;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use telegram::Telegram;

struct Config {
    token: String,
    api_key: String,
    base: String,
    model: String,
    context_window: Option<usize>,
    root: PathBuf,
    workspace: String,
    allowed: Vec<i64>,
}

impl Config {
    fn from_env() -> Result<Self, String> {
        let token = env("TELEGRAM_BOT_TOKEN").ok_or("TELEGRAM_BOT_TOKEN no está configurado")?;
        let api_key = env("OPENAI_API_KEY").ok_or("OPENAI_API_KEY no está configurado")?;
        let root = PathBuf::from(
            env("JIMMY_ROOT")
                .or_else(|| env("RAILWAY_VOLUME_MOUNT_PATH"))
                .unwrap_or_else(|| "/data".into()),
        );
        let workspace =
            env("JIMMY_WORKSPACE").unwrap_or_else(|| root.join("workspace").display().to_string());
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
        })
    }
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

fn main() {
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
    std::fs::create_dir_all(&config.workspace).ok();
    let agent = Agent::new(
        config.base.clone(),
        config.model.clone(),
        config.api_key.clone(),
        config.context_window,
        config.root.clone(),
        config.workspace.clone(),
    );
    let tg = Telegram::new(config.token.clone());
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
            let Some(text) = message.text else {
                continue;
            };
            let chat_id = message.chat.id;
            let agent = agent.clone();
            let tg = tg.clone();
            std::thread::spawn(move || {
                let lock = chat_lock(chat_id);
                let _guard = lock.lock().unwrap();
                eprintln!("jimmy: chat {chat_id} -> {}", clamp(&text, 80));
                if let Err(e) = agent.respond(&tg, chat_id, &text) {
                    eprintln!("jimmy: chat {chat_id} falló: {e}");
                }
            });
        }
    }
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
