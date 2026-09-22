use crate::bus::Bus;
use crate::conversations;
use crate::log::Log;
use crate::pool::{Pool, Turn};
use crate::protocol::{self, Event};
use crate::transport::{Session, Transport};
use crate::worker::Pipe;
use axe::run::{self, Outcome, RunOptions, Sink};
use axe::session::{self, ContextOptions, Entry};
use axe::{Image, Message, OpenAI, ToolCall, ToolOutput, Usage};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

const OUTPUT_RESERVE: usize = 64 * 1024;
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
    cwd: String,
    fragments: String,
    context: String,
    pool: Arc<Pool>,
    bus: Arc<Bus>,
    pipe: Option<Arc<Pipe>>,
    cancel: Arc<AtomicBool>,
}

impl Agent {
    pub fn new(
        base: String,
        model: String,
        api_key: String,
        context_window: Option<usize>,
        root: PathBuf,
        workspace: String,
        fragments: String,
    ) -> Self {
        let cwd = workspace.clone();
        let context = runtime_context(&model, &base, &root, &workspace, &cwd);
        let pool = Pool::new(
            worker_env(&base, &model, &api_key, context_window, &root, &workspace),
            None,
        );
        Self {
            base,
            model,
            api_key,
            context_window,
            root,
            cwd: workspace.clone(),
            workspace,
            fragments,
            context,
            pool,
            bus: Bus::new(),
            pipe: None,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    /// El worker reemplaza esto por un flag propio, que puede levantar mientras
    /// el turno corre.
    pub(crate) fn set_cancel(&mut self, cancel: Arc<AtomicBool>) {
        self.cancel = cancel;
    }

    pub fn cancel(&self, key: &str) {
        self.pool.cancel(key);
    }

    /// Corta el worker y se lleva la carpeta de la conversación.
    pub fn delete(&self, key: &str) -> Result<(), String> {
        self.pool.kill(key);
        let dir = self.conversation_dir(key);
        if !dir.is_dir() {
            return Ok(());
        }
        std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())
    }

    pub fn conversation_dir(&self, key: &str) -> PathBuf {
        conversations::get(&self.root, Path::new(&self.workspace), key).dir
    }

    /// Busca en lo que se dijo, no en lo que se escribió en los archivos: es lo
    /// que uno quiere de un chat.
    pub fn search(&self, needle: &str, limit: usize) -> Vec<Hit> {
        let needle = needle.trim().to_lowercase();
        if needle.chars().count() < 2 {
            return Vec::new();
        }
        let mut hits = Vec::new();
        for project in conversations::projects(&self.root, Path::new(&self.workspace)) {
            for conversation in project.conversations {
                let Ok(text) = std::fs::read_to_string(conversation.dir.join("transcript.jsonl"))
                else {
                    continue;
                };
                for line in text.lines() {
                    let Some(hit) = search_line(line, &needle) else {
                        continue;
                    };
                    hits.push(Hit {
                        conversation: conversation.key.clone(),
                        title: conversation
                            .title
                            .clone()
                            .unwrap_or_else(|| conversation.key.clone()),
                        project: project.name.clone(),
                        role: hit.0,
                        snippet: hit.1,
                    });
                    if hits.len() >= limit {
                        return hits;
                    }
                }
            }
        }
        hits
    }

    /// Who is watching the conversations, for the web frontend to attach to.
    pub fn bus(&self) -> Arc<Bus> {
        self.bus.clone()
    }

    pub fn running(&self, key: &str) -> bool {
        self.pool.running(key)
    }

    /// Where the tools run. It is the workspace unless the conversation belongs
    /// to a project, and the runtime context says so, so it is rebuilt here.
    pub(crate) fn set_cwd(&mut self, cwd: &str) {
        self.cwd = cwd.to_string();
        self.context = runtime_context(&self.model, &self.base, &self.root, &self.workspace, cwd);
    }

    /// Tell the parent what the turn is doing, event by event. Only the worker
    /// sets this: it is the one with a pipe at the other end of the process.
    pub(crate) fn set_pipe(&mut self, pipe: Arc<Pipe>) {
        self.pipe = Some(pipe);
    }

    /// Point the pool at a different binary. Tests only.
    #[cfg(test)]
    pub(crate) fn use_worker_exe(&mut self, exe: PathBuf) {
        self.pool = Pool::new(
            worker_env(
                &self.base,
                &self.model,
                &self.api_key,
                self.context_window,
                &self.root,
                &self.workspace,
            ),
            Some(exe),
        );
    }

    /// Hand the turn to this conversation's worker and relay what it answers.
    pub fn respond(
        &self,
        transport: &dyn Transport,
        session: &Session,
        text: &str,
        images: Vec<Image>,
    ) -> Result<(), String> {
        self.relay(
            transport,
            session,
            protocol::Command::Prompt {
                text: text.to_string(),
                images,
            },
        )
    }

    pub fn resume(&self, transport: &dyn Transport, session: &Session) -> Result<(), String> {
        self.relay(transport, session, protocol::Command::Resume)
    }

    fn relay(
        &self,
        transport: &dyn Transport,
        session: &Session,
        command: protocol::Command,
    ) -> Result<(), String> {
        let status = transport.progress(session);
        let conversation = self.conversation(session);
        let _ = std::fs::create_dir_all(&conversation.dir);
        let log = Log::in_dir(&conversation.dir);
        if let protocol::Command::Prompt { text, .. } = &command {
            self.bus
                .publish(&conversation.key, &log, &Event::User { text: text.clone() });
        }
        match self
            .pool
            .turn(session, &conversation, command, &mut |event| {
                self.bus.publish(&conversation.key, &log, event)
            }) {
            Ok(Turn::Answer(text)) => {
                transport.answer(session, status, &text);
                Ok(())
            }
            Ok(Turn::Failed(message)) => {
                transport.fail(session, status, &message);
                Err(message)
            }
            Err(error) => {
                transport.fail(session, status, &format!("⚠️ {error}"));
                Err(error)
            }
        }
    }

    /// The turn as the worker runs it, in its own process.
    pub(crate) fn local_prompt(
        &self,
        transport: &dyn Transport,
        session: &Session,
        text: &str,
        images: Vec<Image>,
    ) -> Result<(), String> {
        let dir = self.conversation(session).dir;
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
        let entry = Entry::Message { message: user };
        append_entry(&dir, &entry)?;
        entries.push(entry);

        self.execute(transport, session, history, entries, Some(dir))
    }

    pub(crate) fn local_resume(
        &self,
        transport: &dyn Transport,
        session: &Session,
        dir: &Path,
    ) -> Result<(), String> {
        let entries = load_entries(dir);
        let mut history = session::context_messages_with(&entries, CONTEXT_OPTIONS);
        session::drop_incomplete_tool_calls(&mut history);
        if let Some(reply) = final_answer(&history) {
            transport.answer(session, None, reply);
            let _ = std::fs::remove_file(dir.join("inflight"));
            return Ok(());
        }
        if history.is_empty() {
            let _ = std::fs::remove_file(dir.join("inflight"));
            return Ok(());
        }
        self.execute(
            transport,
            session,
            history,
            entries,
            Some(dir.to_path_buf()),
        )
    }

    pub fn run_task(
        &self,
        transport: &dyn Transport,
        session: &Session,
        prompt: &str,
    ) -> Result<(), String> {
        let user = Message {
            role: "user".into(),
            content: prompt.to_string(),
            tool_calls: Vec::new(),
            tool_call_id: String::new(),
            reasoning: String::new(),
            images: Vec::new(),
        };
        self.execute(transport, session, vec![user], Vec::new(), None)
    }

    pub(crate) fn conversation(&self, session: &Session) -> conversations::Conversation {
        conversations::get(&self.root, Path::new(&self.workspace), &session.key())
    }

    fn execute(
        &self,
        transport: &dyn Transport,
        session: &Session,
        mut history: Vec<Message>,
        mut entries: Vec<Entry>,
        dir: Option<PathBuf>,
    ) -> Result<(), String> {
        let _inflight = dir.as_ref().map(|d| Inflight::new(d));
        let mut tools = axe::tui::build_tools(&self.cwd);
        tools.extend(crate::tools::all());
        let mut system = axe::system_prompt(&tools);
        if !self.fragments.is_empty() {
            system.push_str("\n\n");
            system.push_str(&self.fragments);
        }
        system.push('\n');
        system.push_str(&self.context);
        system.push_str(&format!(
            "Chat actual: {}\nTranscript: {}/transcript.jsonl\n",
            session.key(),
            self.conversation(session).dir.display()
        ));
        let memory = crate::memo::render(Path::new(&self.workspace));
        if !memory.is_empty() {
            system.push_str("\n## Memoria en contexto\n");
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
        let cancel = self.cancel.clone();

        let status = transport.progress(session);
        let mut sink = EventSink {
            threshold,
            events: dir.clone(),
            transcript: dir.as_ref().map(|d| d.join("transcript.jsonl")),
            pipe: self.pipe.clone(),
        };

        let mut overflow_retried = false;
        loop {
            let started = Instant::now();
            let end = run::run_stream(&provider, &opts, &history, &cancel, &mut sink);
            if let Some(dir) = &dir {
                record(
                    dir,
                    serde_json::json!({
                        "ts": now(),
                        "kind": "run",
                        "ms": started.elapsed().as_millis(),
                        "outcome": outcome_name(&end.outcome),
                        "input": end.usage.input,
                        "output": end.usage.output,
                        "cached_input": end.usage.cached_input,
                        "context_input": end.context.input,
                    }),
                );
            }
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
                    save(&dir, &mut entries)?;
                    let reply = answer(&end.messages[history.len()..]);
                    transport.answer(session, status, &reply);
                    return Ok(());
                }
                Outcome::Cancelled => {
                    save(&dir, &mut entries)?;
                    transport.fail(session, status, "⚠️ interrumpido");
                    return Err("interrumpido".into());
                }
                Outcome::Compact => {
                    history = match compact(&provider, &self.model, &mut entries) {
                        Ok(history) => history,
                        Err(e) => {
                            save(&dir, &mut entries)?;
                            transport.fail(session, status, &format!("⚠️ {e}"));
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
                                save(&dir, &mut entries)?;
                                transport.fail(session, status, &format!("⚠️ {e}"));
                                return Err(e);
                            }
                        };
                        continue;
                    }
                    save(&dir, &mut entries)?;
                    transport.fail(session, status, &format!("⚠️ error: {e}"));
                    return Err(e);
                }
            }
        }
    }
    pub fn command(&self, session: &Session, text: &str) -> Option<String> {
        match text.split_whitespace().next()? {
            "/start" | "/help" => Some(HELP.into()),
            "/status" => Some(self.status(session)),
            "/clear" => Some(self.clear(session)),
            _ => None,
        }
    }

    fn status(&self, session: &Session) -> String {
        let dir = self.conversation(session).dir;
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

    fn clear(&self, session: &Session) -> String {
        let dir = self.conversation(session).dir;
        let path = dir.join("transcript.jsonl");
        if !path.exists() {
            return "🧹 no había nada que borrar".into();
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let archive = dir.join(format!("transcript.{stamp}.jsonl"));
        if let Err(e) = std::fs::rename(&path, &archive) {
            return format!("⚠️ no pude borrar: {e}");
        }
        match gzip(&archive) {
            Ok(()) => "🧹 contexto borrado".into(),
            Err(e) => format!("🧹 contexto borrado, pero no comprimí: {e}"),
        }
    }
}

/// Un pedazo de conversación que contiene lo que se buscó.
pub struct Hit {
    pub conversation: String,
    pub title: String,
    pub project: String,
    pub role: String,
    pub snippet: String,
}

/// Busca en una entrada del transcript y devuelve quién lo dijo y el pedazo
/// donde aparece.
fn search_line(line: &str, needle: &str) -> Option<(String, String)> {
    let entry: serde_json::Value = serde_json::from_str(line).ok()?;
    let message = entry.get("message")?;
    let content = message.get("Content")?.as_str()?;
    let role = message
        .get("Role")
        .and_then(|role| role.as_str())
        .unwrap_or("")
        .to_string();
    let position = content.to_lowercase().find(needle)?;
    let start = content[..position]
        .char_indices()
        .rev()
        .nth(60)
        .map_or(0, |(i, _)| i);
    let end = content[position..]
        .char_indices()
        .nth(120)
        .map_or(content.len(), |(i, _)| position + i);
    let mut snippet = content[start..end].replace('\n', " ");
    if start > 0 {
        snippet.insert(0, '…');
    }
    if end < content.len() {
        snippet.push('…');
    }
    Some((role, snippet))
}

struct EventSink {
    threshold: Option<usize>,
    events: Option<PathBuf>,
    transcript: Option<PathBuf>,
    pipe: Option<Arc<Pipe>>,
}

struct Inflight(PathBuf);

impl Inflight {
    fn new(dir: &Path) -> Self {
        let path = dir.join("inflight");
        let _ = std::fs::write(&path, b"");
        Self(path)
    }
}

impl Drop for Inflight {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

impl Sink for EventSink {
    fn should_compact(&mut self, input: usize, output: usize) -> bool {
        self.threshold
            .is_some_and(|threshold| input.saturating_add(output) > threshold)
    }

    fn assistant_delta(&mut self, text: &str) {
        let Some(pipe) = &self.pipe else {
            return;
        };
        let _ = pipe.emit(&Event::Delta {
            text: text.to_string(),
        });
    }

    fn tool_delta(&mut self, call: &ToolCall, text: &str) {
        let Some(pipe) = &self.pipe else {
            return;
        };
        let _ = pipe.emit(&Event::ToolDelta {
            id: call.id.clone(),
            text: text.to_string(),
        });
    }

    fn tool_start(&mut self, call: &ToolCall) {
        let Some(pipe) = &self.pipe else {
            return;
        };
        let _ = pipe.emit(&Event::ToolStart {
            id: call.id.clone(),
            name: call.name.clone(),
            args: call.arguments.clone(),
        });
    }

    fn tool_result(&mut self, call: &ToolCall, output: &ToolOutput, elapsed: Duration) {
        let failed = output.text.starts_with("error:");
        if let Some(dir) = &self.events {
            record(
                dir,
                serde_json::json!({
                    "ts": now(),
                    "kind": "tool",
                    "name": call.name,
                    "ms": elapsed.as_millis(),
                    "bytes": output.text.len(),
                    "failed": failed,
                }),
            );
        }
        let Some(pipe) = &self.pipe else {
            return;
        };
        let _ = pipe.emit(&Event::ToolResult {
            id: call.id.clone(),
            text: output.text.clone(),
            ms: elapsed.as_millis() as u64,
            failed,
        });
    }

    fn assistant(&mut self, _turn: usize, message: &Message, _usage: Usage) {
        append_message(&self.transcript, message);
        if message.content.is_empty() {
            return;
        }
        let Some(pipe) = &self.pipe else {
            return;
        };
        let _ = pipe.emit(&Event::Assistant {
            text: message.content.clone(),
        });
    }

    fn tool(&mut self, _turn: usize, message: &Message) {
        append_message(&self.transcript, message);
    }
}

/// What the worker needs to rebuild the same agent on the other side of the
/// pipe. Everything else it inherits.
fn worker_env(
    base: &str,
    model: &str,
    api_key: &str,
    context_window: Option<usize>,
    root: &Path,
    workspace: &str,
) -> Vec<(String, String)> {
    vec![
        ("OPENAI_API_KEY".into(), api_key.to_string()),
        ("AXE_BASE".into(), base.to_string()),
        ("AXE_MODEL".into(), model.to_string()),
        (
            "AXE_CONTEXT_WINDOW".into(),
            context_window.map(|w| w.to_string()).unwrap_or_default(),
        ),
        ("JIMMY_ROOT".into(), root.display().to_string()),
        ("JIMMY_WORKSPACE".into(), workspace.to_string()),
    ]
}

fn runtime_context(model: &str, base: &str, root: &Path, workspace: &str, cwd: &str) -> String {
    let mut out = String::from("## Entorno de ejecución\n");
    out.push_str(&format!("- Modelo: {model} vía {base}\n"));
    out.push_str(&format!("- Raíz persistente: {}\n", root.display()));
    out.push_str(&format!("- Workspace: {workspace}\n"));
    if cwd != workspace {
        out.push_str(&format!("- Directorio de trabajo: {cwd}\n"));
    }
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

fn final_answer(history: &[Message]) -> Option<&str> {
    let last = history.last()?;
    (last.role == "assistant" && last.tool_calls.is_empty()).then_some(last.content.as_str())
}

fn answer(messages: &[Message]) -> String {
    let mut parts = Vec::new();
    for message in messages {
        if message.role == "assistant" && !message.content.is_empty() {
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

fn save(dir: &Option<PathBuf>, entries: &mut [Entry]) -> Result<(), String> {
    match dir {
        Some(dir) => save_entries(dir, entries),
        None => Ok(()),
    }
}

fn drop_superseded_images(entries: &mut [Entry]) {
    let Some(last) = entries
        .iter()
        .rposition(|entry| matches!(entry, Entry::Compaction { .. }))
    else {
        return;
    };
    for entry in &mut entries[..last] {
        if let Entry::Message { message } = entry {
            for image in &mut message.images {
                image.url.clear();
            }
        }
    }
}

fn gzip(path: &Path) -> Result<(), String> {
    let out = std::process::Command::new("gzip")
        .arg(path)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(());
    }
    Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
}

fn load_entries(dir: &Path) -> Vec<Entry> {
    let Ok(text) = std::fs::read_to_string(dir.join("transcript.jsonl")) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn save_entries(dir: &Path, entries: &mut [Entry]) -> Result<(), String> {
    drop_superseded_images(entries);
    let mut out = String::new();
    for entry in entries {
        out.push_str(&serde_json::to_string(entry).map_err(|e| e.to_string())?);
        out.push('\n');
    }
    axe::atomic_write(&dir.join("transcript.jsonl"), out.as_bytes()).map_err(|e| e.to_string())
}

fn append_entry(dir: &Path, entry: &Entry) -> Result<(), String> {
    append_line(&dir.join("transcript.jsonl"), entry)
}

fn append_message(path: &Option<PathBuf>, message: &Message) {
    if let Some(path) = path {
        let _ = append_line(
            path,
            &Entry::Message {
                message: message.clone(),
            },
        );
    }
}

fn append_line(path: &Path, entry: &Entry) -> Result<(), String> {
    let mut line = serde_json::to_string(entry).map_err(|e| e.to_string())?;
    line.push('\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(line.as_bytes()).map_err(|e| e.to_string())
}

fn record(dir: &Path, event: serde_json::Value) {
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("events.jsonl"))
    else {
        return;
    };
    let mut line = event.to_string();
    line.push('\n');
    let _ = file.write_all(line.as_bytes());
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn outcome_name(outcome: &Outcome) -> &'static str {
    match outcome {
        Outcome::Done => "done",
        Outcome::MaxTurns => "max_turns",
        Outcome::Cancelled => "cancelled",
        Outcome::Compact => "compact",
        Outcome::Failed(_) => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::Msg;
    use axe::ToolCall;
    use std::sync::Mutex;

    fn assistant(content: &str, tool_calls: Vec<ToolCall>) -> Message {
        Message {
            role: "assistant".into(),
            content: content.into(),
            tool_calls,
            tool_call_id: String::new(),
            reasoning: String::new(),
            images: Vec::new(),
        }
    }

    fn call() -> ToolCall {
        ToolCall {
            id: "1".into(),
            name: "bash".into(),
            arguments: "{}".into(),
        }
    }

    #[test]
    fn gzip_compresses_and_removes_the_original() {
        let dir = std::env::temp_dir().join(format!("jimmy-gzip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.jsonl");
        std::fs::write(&path, "hola\n").unwrap();
        gzip(&path).unwrap();
        assert!(!path.exists());
        assert_eq!(
            std::fs::read(dir.join("a.jsonl.gz")).unwrap()[..2],
            [0x1f, 0x8b]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn concurrent_records_keep_one_json_per_line() {
        let dir = std::env::temp_dir().join(format!("jimmy-events-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let threads: Vec<_> = (0..8)
            .map(|n| {
                let dir = dir.clone();
                std::thread::spawn(move || {
                    for i in 0..50 {
                        record(
                            &dir,
                            serde_json::json!({"n": n, "i": i, "text": "x".repeat(200)}),
                        );
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let text = std::fs::read_to_string(dir.join("events.jsonl")).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 400);
        for line in lines {
            serde_json::from_str::<serde_json::Value>(line).unwrap();
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn image(url: &str) -> Image {
        Image {
            path: "/data/files/a.png".into(),
            url: url.into(),
        }
    }

    fn photo(url: &str) -> Entry {
        let mut message = assistant("mirá", Vec::new());
        message.images = vec![image(url)];
        Entry::Message { message }
    }

    fn compaction(retained: Vec<Message>) -> Entry {
        Entry::Compaction {
            summary: "resumen".into(),
            tokens_before: 0,
            timestamp: 0,
            retained,
        }
    }

    fn urls(entries: &[Entry]) -> Vec<String> {
        entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Message { message } => Some(message),
                _ => None,
            })
            .flat_map(|message| message.images.iter())
            .map(|image| image.url.clone())
            .collect()
    }

    #[test]
    fn images_before_a_compaction_lose_their_bytes() {
        let mut retained = assistant("quedate", Vec::new());
        retained.images = vec![image("data:image/png;base64,QUJD")];
        let mut entries = vec![
            photo("data:image/png;base64,QUJD"),
            compaction(vec![retained]),
            photo("data:image/png;base64,REVG"),
        ];
        drop_superseded_images(&mut entries);
        assert_eq!(urls(&entries), ["", "data:image/png;base64,REVG"]);
        assert_eq!(entries[0].clone(), photo("").clone());
        match &entries[1] {
            Entry::Compaction { retained, .. } => {
                assert_eq!(retained[0].images[0].url, "data:image/png;base64,QUJD");
            }
            _ => panic!("esperaba una compactación"),
        }
    }

    #[test]
    fn images_survive_without_a_compaction() {
        let mut entries = vec![photo("data:image/png;base64,QUJD")];
        drop_superseded_images(&mut entries);
        assert_eq!(urls(&entries), ["data:image/png;base64,QUJD"]);
    }

    #[test]
    fn answer_keeps_text_that_came_with_tool_calls() {
        let messages = vec![
            assistant("primera parte", vec![call()]),
            assistant("segunda parte", Vec::new()),
        ];
        assert_eq!(answer(&messages), "primera parte\n\nsegunda parte");
    }

    #[test]
    fn answer_falls_back_when_there_is_only_a_tool_call() {
        let messages = vec![assistant("", vec![call()])];
        assert_eq!(answer(&messages), "✅ listo");
    }

    #[derive(Default)]
    struct Fake {
        answers: Mutex<Vec<String>>,
        failures: Mutex<Vec<String>>,
    }

    impl Transport for Fake {
        fn parse_target(&self, _: &str) -> Result<Session, String> {
            Ok(Session::channel("x"))
        }
        fn progress(&self, _: &Session) -> Option<Msg> {
            None
        }
        fn answer(&self, _: &Session, _: Option<Msg>, markdown: &str) {
            self.answers.lock().unwrap().push(markdown.to_string());
        }
        fn note(&self, _: &Session, _: &str) {}
        fn fail(&self, _: &Session, _: Option<Msg>, text: &str) {
            self.failures.lock().unwrap().push(text.to_string());
        }
        fn download(&self, _: &str) -> Result<(String, Vec<u8>), String> {
            Err("no".into())
        }
        fn send_media(&self, _: &Session, _: &Path, _: Option<&str>) -> Result<Msg, String> {
            Err("no".into())
        }
    }

    fn resume_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jimmy-resume-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn message(role: &str, content: &str) -> Message {
        let mut message = assistant(content, Vec::new());
        message.role = role.into();
        message
    }

    fn write_transcript(dir: &Path, entries: &[Entry]) {
        let mut text = String::new();
        for entry in entries {
            text.push_str(&serde_json::to_string(entry).unwrap());
            text.push('\n');
        }
        std::fs::write(dir.join("transcript.jsonl"), text).unwrap();
    }

    fn agent() -> Agent {
        agent_in(&std::env::temp_dir())
    }

    fn agent_in(root: &Path) -> Agent {
        Agent::new(
            "http://localhost".into(),
            "model".into(),
            "key".into(),
            Some(1_000_000),
            root.to_path_buf(),
            "/tmp".into(),
            String::new(),
        )
    }

    /// Cada llamada escribe un archivo nuevo: reescribir uno que otro proceso
    /// todavía está ejecutando da «Text file busy».
    fn worker_script(name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let dir = resume_dir(name);
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = dir.join(format!("{unique}-{name}"));
        std::fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn respond_relays_the_workers_answer() {
        let root = resume_dir("relay");
        let mut agent = agent_in(&root);
        agent.use_worker_exe(worker_script(
            "relay.sh",
            "echo '{\"event\":\"ready\"}'
while read -r line; do
  case \"$line\" in *shutdown*) exit 0 ;; esac
  echo '{\"event\":\"assistant\",\"text\":\"eco\"}'
  echo '{\"event\":\"tool_start\",\"id\":\"c1\",\"name\":\"bash\",\"args\":\"{}\"}'
  echo '{\"event\":\"tool_result\",\"id\":\"c1\",\"text\":\"hola\",\"ms\":3,\"failed\":false}'
  echo '{\"event\":\"done\",\"text\":\"eco\"}'
done
",
        ));
        let fake = Fake::default();
        agent
            .respond(&fake, &Session::channel("x"), "hola", Vec::new())
            .unwrap();
        assert_eq!(fake.answers.lock().unwrap().as_slice(), ["eco"]);
        assert!(fake.failures.lock().unwrap().is_empty());

        let log = std::fs::read_to_string(root.join("chats/x/conversation.jsonl")).unwrap();
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(lines.len(), 5, "{log}");
        assert!(
            lines[0].contains("\"user\"") && lines[0].contains("hola"),
            "{log}"
        );
        assert!(lines[1].contains("\"assistant\""), "{log}");
        assert!(lines[2].contains("\"tool_start\""), "{log}");
        assert!(lines[3].contains("\"tool_result\""), "{log}");
        assert!(lines[4].contains("\"done\""), "{log}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_user_message_reaches_whoever_is_watching() {
        let root = resume_dir("watch");
        let mut agent = agent_in(&root);
        agent.use_worker_exe(worker_script(
            "watch.sh",
            "echo '{\"event\":\"ready\"}'
while read -r line; do
  case \"$line\" in *shutdown*) exit 0 ;; esac
  echo '{\"event\":\"assistant\",\"text\":\"eco\"}'
  echo '{\"event\":\"done\",\"text\":\"eco\"}'
done
",
        ));
        let log = Log::in_dir(&root.join("chats/x"));
        let (_backlog, live) = agent.bus().attach("x", &log, "berti");
        let fake = Fake::default();
        agent
            .respond(&fake, &Session::channel("x"), "hola", Vec::new())
            .unwrap();

        let seen: Vec<String> = live
            .iter()
            .take(4)
            .map(|event| serde_json::to_string(&event).unwrap())
            .filter(|line| !line.contains("\"presence\""))
            .collect();
        assert!(seen[0].contains("\"user\""), "{seen:?}");
        assert!(seen[0].contains("hola"), "{seen:?}");
        assert!(seen[1].contains("\"assistant\""), "{seen:?}");
        assert!(seen[2].contains("\"done\""), "{seen:?}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn respond_reports_a_worker_that_dies() {
        let mut agent = agent();
        agent.use_worker_exe(worker_script("dead.sh", "exit 0\n"));
        let fake = Fake::default();
        assert!(agent
            .respond(&fake, &Session::channel("x"), "hola", Vec::new())
            .is_err());
        let failures = fake.failures.lock().unwrap();
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("sin responder"), "{failures:?}");
    }

    #[test]
    fn final_answer_is_the_last_assistant_without_tool_calls() {
        let history = vec![message("user", "hola"), message("assistant", "listo")];
        assert_eq!(final_answer(&history), Some("listo"));

        let mut calling = message("assistant", "pensando");
        calling.tool_calls = vec![call()];
        assert_eq!(final_answer(&[message("user", "x"), calling]), None);
        assert_eq!(final_answer(&[message("user", "x")]), None);
        assert_eq!(final_answer(&[]), None);
    }

    #[test]
    fn resume_delivers_a_finished_but_unsent_answer() {
        let dir = resume_dir("deliver");
        write_transcript(
            &dir,
            &[
                Entry::Message {
                    message: message("user", "hola"),
                },
                Entry::Message {
                    message: message("assistant", "listo"),
                },
            ],
        );
        std::fs::write(dir.join("inflight"), b"").unwrap();
        let fake = Fake::default();
        agent()
            .local_resume(&fake, &Session::channel("x"), &dir)
            .unwrap();
        assert_eq!(fake.answers.lock().unwrap().as_slice(), ["listo"]);
        assert!(!dir.join("inflight").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resume_clears_the_marker_when_there_is_nothing_to_do() {
        let dir = resume_dir("empty");
        std::fs::write(dir.join("inflight"), b"").unwrap();
        let fake = Fake::default();
        agent()
            .local_resume(&fake, &Session::channel("x"), &dir)
            .unwrap();
        assert!(fake.answers.lock().unwrap().is_empty());
        assert!(!dir.join("inflight").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_sink_appends_each_message_to_the_transcript() {
        let dir = resume_dir("sink");
        let mut sink = EventSink {
            threshold: None,
            events: None,
            transcript: Some(dir.join("transcript.jsonl")),
            pipe: None,
        };
        sink.assistant(0, &assistant("uno", vec![call()]), axe::Usage::default());
        sink.tool(0, &message("tool", "dos"));
        let text = std::fs::read_to_string(dir.join("transcript.jsonl")).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        for line in lines {
            serde_json::from_str::<Entry>(line).unwrap();
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
