use crate::telegram::Telegram;
use axe::run::{self, Outcome, RunOptions, Sink};
use axe::session::{self, ContextOptions, Entry};
use axe::{Image, Message, OpenAI};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

const MESSAGE_CHARS: usize = 4000;
const MARKDOWN_CHARS: usize = 3500;
const OUTPUT_RESERVE: usize = 64 * 1024;
const MEMORY_CHARS: usize = 8_000;
const HELP: &str = "Comandos:\n/status — contexto usado y versión\n/clear — borrar el contexto de este chat\n/help — esto";
const CONTEXT_OPTIONS: ContextOptions = ContextOptions {
    original_task: false,
    workspace_state: true,
};

#[derive(Clone)]
pub struct Agent {
    base: String,
    model: String,
    api_key: String,
    context_window: Option<usize>,
    root: PathBuf,
    workspace: String,
    context: String,
}

impl Agent {
    pub fn new(
        base: String,
        model: String,
        api_key: String,
        context_window: Option<usize>,
        root: PathBuf,
        workspace: String,
    ) -> Self {
        let context = runtime_context(&model, &base, &root, &workspace);
        Self {
            base,
            model,
            api_key,
            context_window,
            root,
            workspace,
            context,
        }
    }

    pub fn respond(
        &self,
        tg: &Telegram,
        chat_id: i64,
        text: &str,
        images: Vec<Image>,
    ) -> Result<(), String> {
        let dir = self.chat_dir(chat_id);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

        let mut entries = load_entries(&dir);
        let mut history = session::context_messages_with(&entries, CONTEXT_OPTIONS);
        session::drop_incomplete_tool_calls(&mut history);

        let user = Message {
            role: "user".into(),
            content: text.to_string(),
            tool_calls: Vec::new(),
            tool_call_id: String::new(),
            reasoning: String::new(),
            images,
        };
        history.push(user.clone());
        entries.push(Entry::Message { message: user });

        self.execute(tg, chat_id, history, entries, Some(dir))
    }

    pub fn run_task(&self, tg: &Telegram, chat_id: i64, prompt: &str) -> Result<(), String> {
        let user = Message {
            role: "user".into(),
            content: prompt.to_string(),
            tool_calls: Vec::new(),
            tool_call_id: String::new(),
            reasoning: String::new(),
            images: Vec::new(),
        };
        self.execute(tg, chat_id, vec![user], Vec::new(), None)
    }

    fn chat_dir(&self, chat_id: i64) -> PathBuf {
        self.root.join("chats").join(chat_id.to_string())
    }

    fn execute(
        &self,
        tg: &Telegram,
        chat_id: i64,
        mut history: Vec<Message>,
        mut entries: Vec<Entry>,
        dir: Option<PathBuf>,
    ) -> Result<(), String> {
        let tools = axe::tui::build_tools(&self.workspace);
        let mut system = format!(
            "{}\n\n{}Chat actual: {chat_id}\nTranscript: {}/transcript.jsonl\n",
            axe::system_prompt(&tools, &self.workspace),
            self.context,
            self.chat_dir(chat_id).display()
        );
        let memory = read_memory(&self.workspace);
        if !memory.is_empty() {
            system.push_str("\n## Memoria\n");
            system.push_str(&memory);
            system.push('\n');
        }
        let provider = OpenAI::new(self.base.clone(), self.api_key.clone());
        let opts = RunOptions {
            model: &self.model,
            system: &system,
            tools: &tools,
            max_turns: usize::MAX,
        };
        let threshold = self
            .context_window
            .map(|w| w.saturating_sub(OUTPUT_RESERVE));
        let cancel = Arc::new(AtomicBool::new(false));

        let status = tg.send_message(chat_id, "⚙️ pensando…").ok();
        let mut sink = TgSink { threshold };

        let mut overflow_retried = false;
        loop {
            let end = run::run_stream(&provider, &opts, &history, &cancel, &mut sink);
            for message in &end.messages[history.len()..] {
                entries.push(Entry::Message {
                    message: message.clone(),
                });
            }
            if end.usage.input > 0 || end.usage.output > 0 {
                entries.push(Entry::Usage {
                    input: end.usage.input,
                    output: end.usage.output,
                    cached_input: end.context.cached_input,
                    context_input: end.context.input,
                    context_output: end.context.output,
                });
            }
            match end.outcome {
                Outcome::Done | Outcome::MaxTurns => {
                    save(&dir, &entries)?;
                    let reply = answer(&end.messages[history.len()..]);
                    finalize_markdown(tg, chat_id, status, &reply);
                    return Ok(());
                }
                Outcome::Cancelled => {
                    save(&dir, &entries)?;
                    finalize(tg, chat_id, status, "⚠️ interrumpido");
                    return Err("interrumpido".into());
                }
                Outcome::Compact => {
                    history = match compact(&provider, &self.model, &mut entries) {
                        Ok(history) => history,
                        Err(e) => {
                            save(&dir, &entries)?;
                            finalize(tg, chat_id, status, &format!("⚠️ {e}"));
                            return Err(e);
                        }
                    };
                }
                Outcome::Failed(e) => {
                    if !overflow_retried && session::is_overflow_error(&e) {
                        overflow_retried = true;
                        history = match compact(&provider, &self.model, &mut entries) {
                            Ok(history) => history,
                            Err(e) => {
                                save(&dir, &entries)?;
                                finalize(tg, chat_id, status, &format!("⚠️ {e}"));
                                return Err(e);
                            }
                        };
                        continue;
                    }
                    save(&dir, &entries)?;
                    finalize(tg, chat_id, status, &format!("⚠️ error: {e}"));
                    return Err(e);
                }
            }
        }
    }
    pub fn command(&self, chat_id: i64, text: &str) -> Option<String> {
        match text.split_whitespace().next()? {
            "/start" | "/help" => Some(HELP.into()),
            "/status" => Some(self.status(chat_id)),
            "/clear" => Some(self.clear(chat_id)),
            _ => None,
        }
    }

    fn status(&self, chat_id: i64) -> String {
        let dir = self.root.join("chats").join(chat_id.to_string());
        let entries = load_entries(&dir);
        let used = session::latest_context_tokens(&entries).unwrap_or(0);
        let window = self.context_window.unwrap_or(0);
        let percent = used.saturating_mul(100).checked_div(window).unwrap_or(0);
        let commit = env("RAILWAY_GIT_COMMIT_SHA")
            .or_else(|| env("JIMMY_COMMIT_SHA"))
            .map(|sha| sha.chars().take(7).collect::<String>())
            .unwrap_or_else(|| "?".into());
        format!(
            "📊 Estado\n\nModelo: {}\nContexto: {}K / {}K ({}%)\nCommit: {}\nWorkspace: {}",
            self.model,
            used / 1000,
            window / 1000,
            percent,
            commit,
            self.workspace
        )
    }

    fn clear(&self, chat_id: i64) -> String {
        let dir = self.root.join("chats").join(chat_id.to_string());
        let path = dir.join("transcript.jsonl");
        if !path.exists() {
            return "🧹 no había nada que borrar".into();
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        match std::fs::rename(&path, dir.join(format!("transcript.{stamp}.jsonl"))) {
            Ok(()) => "🧹 contexto borrado".into(),
            Err(e) => format!("⚠️ no pude borrar: {e}"),
        }
    }
}

struct TgSink {
    threshold: Option<usize>,
}

impl Sink for TgSink {
    fn should_compact(&mut self, input: usize, output: usize) -> bool {
        self.threshold
            .is_some_and(|threshold| input.saturating_add(output) > threshold)
    }
}

fn runtime_context(model: &str, base: &str, root: &Path, workspace: &str) -> String {
    let mut out = String::from("## Entorno de ejecución\n");
    out.push_str(&format!("- Modelo: {model} vía {base}\n"));
    out.push_str(&format!("- Raíz persistente: {}\n", root.display()));
    out.push_str(&format!("- Workspace: {workspace}\n"));
    out.push_str(&format!(
        "- Plataforma: {}/{}\n",
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    if let Some(sha) = env("RAILWAY_GIT_COMMIT_SHA").or_else(|| env("JIMMY_COMMIT_SHA")) {
        let short: String = sha.chars().take(7).collect();
        out.push_str(&format!("- Commit en ejecución: {short}\n"));
    }
    if env("RAILWAY_PROJECT_ID").is_some() {
        out.push_str(&format!(
            "- Corre en Railway desde un Dockerfile, con volumen persistente en {}\n",
            root.display()
        ));
    }
    out
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

fn read_memory(workspace: &str) -> String {
    let path = Path::new(workspace).join("notes/memory.md");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return String::new();
    };
    let text = text.trim();
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= MEMORY_CHARS {
        return text.to_string();
    }
    let tail: String = chars[chars.len() - MEMORY_CHARS..].iter().collect();
    let tail = tail.split_once('\n').map(|(_, rest)| rest).unwrap_or(&tail);
    format!("[truncada: entradas más recientes]\n{tail}")
}

fn compact(
    provider: &OpenAI,
    model: &str,
    entries: &mut Vec<Entry>,
) -> Result<Vec<Message>, String> {
    let (summary, tokens_before, retained) =
        session::compact_with(provider, model, entries, CONTEXT_OPTIONS)?;
    entries.push(Entry::Compaction {
        summary,
        tokens_before,
        timestamp: session::now_ms(),
        retained,
    });
    let mut out = session::context_messages_with(entries, CONTEXT_OPTIONS);
    session::drop_incomplete_tool_calls(&mut out);
    Ok(out)
}

fn finalize(tg: &Telegram, chat_id: i64, status: Option<i64>, text: &str) {
    let mut parts = chunks(text, MESSAGE_CHARS).into_iter();
    if let Some(id) = status {
        match parts.next() {
            Some(first) => {
                if tg.edit_message(chat_id, id, &first).is_err() {
                    tg.delete_message(chat_id, id);
                    let _ = tg.send_message(chat_id, &first);
                }
            }
            None => tg.delete_message(chat_id, id),
        }
    }
    for part in parts {
        let _ = tg.send_message(chat_id, &part);
    }
}

fn finalize_markdown(tg: &Telegram, chat_id: i64, status: Option<i64>, text: &str) {
    let mut parts = crate::markdown::split(text, MARKDOWN_CHARS).into_iter();
    if let Some(id) = status {
        match parts.next() {
            Some(first) => {
                let html = crate::markdown::to_telegram_html(&first);
                if tg.edit_html(chat_id, id, &html).is_err() {
                    tg.delete_message(chat_id, id);
                    send_markdown(tg, chat_id, &first);
                }
            }
            None => tg.delete_message(chat_id, id),
        }
    }
    for part in parts {
        send_markdown(tg, chat_id, &part);
    }
}

fn send_markdown(tg: &Telegram, chat_id: i64, markdown: &str) {
    let html = crate::markdown::to_telegram_html(markdown);
    if tg.send_html(chat_id, &html).is_err() {
        let _ = tg.send_message(chat_id, markdown);
    }
}

fn answer(messages: &[Message]) -> String {
    let mut parts = Vec::new();
    for message in messages {
        if message.role == "assistant"
            && !message.content.is_empty()
            && message.tool_calls.is_empty()
        {
            parts.push(message.content.trim());
        }
    }
    let out = parts.join("\n\n");
    if out.trim().is_empty() {
        "✅ listo".into()
    } else {
        out
    }
}

fn save(dir: &Option<PathBuf>, entries: &[Entry]) -> Result<(), String> {
    match dir {
        Some(dir) => save_entries(dir, entries),
        None => Ok(()),
    }
}

fn load_entries(dir: &Path) -> Vec<Entry> {
    let Ok(text) = std::fs::read_to_string(dir.join("transcript.jsonl")) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn save_entries(dir: &Path, entries: &[Entry]) -> Result<(), String> {
    let mut out = String::new();
    for entry in entries {
        out.push_str(&serde_json::to_string(entry).map_err(|e| e.to_string())?);
        out.push('\n');
    }
    axe::atomic_write(&dir.join("transcript.jsonl"), out.as_bytes()).map_err(|e| e.to_string())
}

fn chunks(s: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut count = 0;
    for ch in s.chars() {
        if count == max {
            out.push(std::mem::take(&mut current));
            count = 0;
        }
        current.push(ch);
        count += 1;
    }
    if !current.is_empty() {
        out.push(current);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}
