use crate::bus::Bus;
use crate::conversations;
use crate::log::Log;
use crate::media;
use crate::pool::{Pool, Turn};
use crate::protocol::{self, Event};
use crate::store;
use crate::transport::{Msg, Session, Transport};
use crate::worker::Pipe;
use axe::durable::Journal;
use axe::run::{self, Outcome, RunOptions, Sink};
use axe::session::{self, ContextOptions, Entry};
use axe::{Image, Message, OpenAI, ToolCall, ToolOutput, Usage};
use std::collections::HashMap;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const OUTPUT_RESERVE: usize = 64 * 1024;
const HELP: &str = "Comandos:\n/status — contexto usado y versión\n/compact — compactar el contexto ahora\n/clear — borrar el contexto de este chat\n/help — esto";
const CONTEXT_OPTIONS: ContextOptions = ContextOptions {
    original_task: false,
    workspace_state: true,
};

#[derive(Clone)]
pub struct Agent {
    base: String,
    model: String,
    api_key: String,
    effort: String,
    max_tokens: usize,
    context_window: Option<usize>,
    root: PathBuf,
    workspace: String,
    cwd: String,
    system: String,
    context: String,
    pool: Arc<Pool>,
    bus: Arc<Bus>,
    turns: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
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
        system: String,
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
            effort: env("AXE_REASONING_EFFORT").unwrap_or_else(|| "max".into()),
            max_tokens: env("AXE_MAX_TOKENS")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0),
            context_window,
            root,
            cwd: workspace.clone(),
            workspace,
            system,
            context,
            pool,
            bus: Bus::new(),
            turns: Arc::new(Mutex::new(HashMap::new())),
            pipe: None,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    fn provider(&self, key: &str) -> OpenAI {
        OpenAI::new(self.base.clone(), self.api_key.clone())
            .effort(&self.effort)
            .max_tokens(self.max_tokens)
            .user_id(&user_id(key))
    }

    pub(crate) fn set_cancel(&mut self, cancel: Arc<AtomicBool>) {
        self.cancel = cancel;
    }

    pub fn cancel(&self, session: &Session, author: &str) {
        if !author.is_empty() {
            self.say(
                session,
                &Event::Stopped {
                    author: author.to_string(),
                },
            );
        }
        self.pool.cancel(&session.key());
    }

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

    pub fn search(&self, needle: &str, limit: usize) -> Vec<Hit> {
        let needle = needle.trim().to_lowercase();
        if needle.chars().count() < 2 {
            return Vec::new();
        }
        let mut hits = Vec::new();
        for project in conversations::projects(&self.root, Path::new(&self.workspace)) {
            for conversation in project.conversations {
                for (role, content) in store::texts(&conversation.dir) {
                    let Some(snippet) = snippet(&content, &needle) else {
                        continue;
                    };
                    hits.push(Hit {
                        conversation: conversation.key.clone(),
                        title: conversation
                            .title
                            .clone()
                            .unwrap_or_else(|| conversation.key.clone()),
                        project: project.name.clone(),
                        role,
                        snippet,
                    });
                    if hits.len() >= limit {
                        return hits;
                    }
                }
            }
        }
        hits
    }

    pub fn bus(&self) -> Arc<Bus> {
        self.bus.clone()
    }

    pub fn running(&self, key: &str) -> bool {
        self.pool.running(key)
    }

    pub(crate) fn set_cwd(&mut self, cwd: &str) {
        self.cwd = cwd.to_string();
        self.context = runtime_context(&self.model, &self.base, &self.root, &self.workspace, cwd);
    }

    pub(crate) fn set_pipe(&mut self, pipe: Arc<Pipe>) {
        self.pipe = Some(pipe);
    }

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

    pub fn respond(
        &self,
        transport: &dyn Transport,
        session: &Session,
        text: &str,
        images: Vec<Image>,
        author: &str,
    ) -> Result<(), String> {
        self.announce(session, text, &images, author);
        let turn = self.wait_turn(session);
        let _guard = turn.lock().unwrap();
        self.relay(
            transport,
            session,
            protocol::Command::Prompt {
                text: text.to_string(),
                images,
                author: author.to_string(),
            },
        )
    }

    pub fn resume(&self, transport: &dyn Transport, session: &Session) -> Result<(), String> {
        let turn = self.wait_turn(session);
        let _guard = turn.lock().unwrap();
        self.relay(transport, session, protocol::Command::Resume)
    }

    pub fn compact(&self, transport: &dyn Transport, session: &Session) -> Result<(), String> {
        let turn = self.wait_turn(session);
        let _guard = turn.lock().unwrap();
        self.relay(transport, session, protocol::Command::Compact)
    }

    pub fn announce(&self, session: &Session, text: &str, images: &[Image], author: &str) {
        self.say(
            session,
            &Event::User {
                text: text.to_string(),
                author: author.to_string(),
                images: attachments(&self.conversation(session).dir, images),
            },
        );
    }

    fn say(&self, session: &Session, event: &Event) {
        let conversation = self.conversation(session);
        let _ = std::fs::create_dir_all(&conversation.dir);
        let log = Log::in_dir(&conversation.dir);
        self.bus.publish(&conversation.key, &log, event);
    }

    fn flush_media(&self, session: &Session) {
        let conversation = self.conversation(session);
        for event in media::drain(&conversation) {
            self.say(session, &event);
        }
    }

    fn wait_turn(&self, session: &Session) -> Arc<Mutex<()>> {
        self.turns
            .lock()
            .unwrap()
            .entry(session.key())
            .or_default()
            .clone()
    }

    fn relay(
        &self,
        transport: &dyn Transport,
        session: &Session,
        command: protocol::Command,
    ) -> Result<(), String> {
        let status = transport.progress(session);
        let mut live = Live::new(transport, session, status);
        let conversation = self.conversation(session);
        let _ = std::fs::create_dir_all(&conversation.dir);
        let log = Log::in_dir(&conversation.dir);
        let turn = self
            .pool
            .turn(session, &conversation, command, &mut |event| {
                live.on(event);
                self.bus.publish(&conversation.key, &log, event)
            });
        self.flush_media(session);
        match turn {
            Ok(Turn::Answer(text)) => {
                transport.answer(session, live.take(), &text);
                Ok(())
            }
            Ok(Turn::Failed(message)) => {
                transport.fail(session, live.take(), &message);
                Err(message)
            }
            Err(error) => {
                transport.fail(session, live.take(), &format!("⚠️ {error}"));
                Err(error)
            }
        }
    }

    pub(crate) fn local_prompt(
        &self,
        transport: &dyn Transport,
        session: &Session,
        text: &str,
        images: Vec<Image>,
        author: &str,
    ) -> Result<(), String> {
        let dir = self.conversation(session).dir;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

        let mut journal = store::open(&dir)?;
        let mut entries = store::entries(&mut journal)?;
        let mut history = session::context_messages_with(&entries, CONTEXT_OPTIONS);
        session::drop_incomplete_tool_calls(&mut history);

        let user = Message {
            role: "user".into(),
            content: signed(text, author),
            tool_calls: Vec::new(),
            tool_call_id: String::new(),
            reasoning: String::new(),
            images,
        };
        history.push(user.clone());
        let entry = Entry::Message { message: user };
        journal
            .append(vec![entry.clone()])
            .map_err(|e| e.to_string())?;
        entries.push(entry);

        self.execute(transport, session, history, entries, Some(journal), false)
            .map(|_| ())
    }

    pub(crate) fn local_compact(
        &self,
        transport: &dyn Transport,
        session: &Session,
    ) -> Result<(), String> {
        let dir = self.conversation(session).dir;
        let mut journal = store::open(&dir)?;
        let entries: Vec<Entry> = journal
            .entries()
            .iter()
            .map(|record| record.entry.clone())
            .collect();
        let provider = self.provider(&session.key());
        match compact(&provider, &self.model, &entries) {
            Ok(entry) => {
                let before = match &entry {
                    Entry::Compaction { tokens_before, .. } => *tokens_before,
                    _ => 0,
                };
                journal.append(vec![entry]).map_err(|e| e.to_string())?;
                let _ = journal.maybe_snapshot();
                let report = format!("🧹 compactado: {}K tokens", before / 1000);
                transport.answer(session, None, &report);
            }
            Err(e) if e == "nothing to summarize" => {
                transport.answer(session, None, "🧹 no había nada que compactar");
            }
            Err(e) => transport.fail(session, None, &format!("⚠️ no pude compactar: {e}")),
        }
        Ok(())
    }

    pub(crate) fn local_resume(
        &self,
        transport: &dyn Transport,
        session: &Session,
        dir: &Path,
    ) -> Result<(), String> {
        let mut journal = store::open(dir)?;
        let entries = store::entries(&mut journal)?;
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
        self.execute(transport, session, history, entries, Some(journal), false)
            .map(|_| ())
    }

    pub fn run_task(
        &self,
        transport: &dyn Transport,
        session: &Session,
        prompt: &str,
        silent: bool,
    ) -> Result<String, String> {
        let turn = self.wait_turn(session);
        let _guard = turn.lock().unwrap();
        let user = Message {
            role: "user".into(),
            content: prompt.to_string(),
            tool_calls: Vec::new(),
            tool_call_id: String::new(),
            reasoning: String::new(),
            images: Vec::new(),
        };
        self.execute(transport, session, vec![user], Vec::new(), None, silent)
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
        mut journal: Option<Journal>,
        silent: bool,
    ) -> Result<String, String> {
        let dir = journal.as_ref().map(|journal| journal.dir().to_path_buf());
        let mut tools = axe::tools::build_tools_with(
            Arc::new(axe::machine::Local::new(&self.cwd)),
            Some(axe::web::Web::at(
                search_base(&self.base),
                self.api_key.clone(),
            )),
        );
        tools.extend(crate::tools::all());
        let mut system = axe::system_prompt(&tools);
        system.push_str("\n\n");
        system.push_str(&self.system);
        system.push('\n');
        system.push_str(&self.context);
        system.push_str(&format!(
            "Chat actual: {}\nHistorial: {}/journal.jsonl\n",
            session.key(),
            self.conversation(session).dir.display()
        ));
        system.push_str(
            "Si un mensaje empieza con un nombre entre corchetes, es quien lo escribió.\n",
        );
        let provider = self.provider(&session.key());
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

        let mut status = if silent {
            None
        } else {
            transport.progress(session)
        };

        let mut overflow_retried = false;
        loop {
            let started = Instant::now();
            let mut sink = EventSink {
                threshold,
                events: dir.clone(),
                journal: journal.take(),
                pipe: self.pipe.clone(),
                committed: 0,
                error: None,
            };
            let end = run::run_stream(&provider, &opts, &history, &cancel, &mut sink);
            journal = sink.journal.take();
            if let Some(error) = sink.error.take() {
                transport.fail(session, status.take(), &format!("⚠️ {error}"));
                return Err(error);
            }
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
            let fresh = end.messages.len().saturating_sub(history.len());
            let committed = sink.committed.min(fresh);
            for message in &end.messages[history.len()..history.len() + committed] {
                entries.push(Entry::Message {
                    message: message.clone(),
                });
            }
            if end.usage.input > 0 || end.usage.output > 0 {
                let usage = Entry::Usage {
                    input: end.usage.input,
                    output: end.usage.output,
                    cached_input: end.context.cached_input,
                    context_input: end.context.input,
                    context_output: end.context.output,
                };
                commit(&mut journal, usage.clone()).map_err(|error| {
                    transport.fail(session, status.take(), &format!("⚠️ {error}"));
                    error
                })?;
                entries.push(usage);
            }
            match end.outcome {
                Outcome::Done | Outcome::MaxTurns => {
                    snapshot(&mut journal);
                    let fallback = if silent { None } else { Some("✅ listo") };
                    let reply = answer(&end.messages[history.len()..], fallback);
                    transport.answer(session, status.take(), &reply);
                    return Ok(reply);
                }
                Outcome::Cancelled => {
                    snapshot(&mut journal);
                    transport.fail(session, status.take(), "⚠️ interrumpido");
                    return Err("interrumpido".into());
                }
                Outcome::Compact => {
                    history = match compacted(&mut journal, &mut entries, &provider, &self.model) {
                        Ok(history) => history,
                        Err(e) => {
                            snapshot(&mut journal);
                            transport.fail(session, status.take(), &format!("⚠️ {e}"));
                            return Err(e);
                        }
                    };
                }
                Outcome::Failed(e) => {
                    if !overflow_retried && session::is_overflow_error(&e) {
                        overflow_retried = true;
                        history =
                            match compacted(&mut journal, &mut entries, &provider, &self.model) {
                                Ok(history) => history,
                                Err(e) => {
                                    snapshot(&mut journal);
                                    transport.fail(session, status.take(), &format!("⚠️ {e}"));
                                    return Err(e);
                                }
                            };
                        continue;
                    }
                    snapshot(&mut journal);
                    transport.fail(session, status.take(), &format!("⚠️ error: {e}"));
                    return Err(e);
                }
            }
        }
    }
    pub fn command(&self, session: &Session, text: &str) -> Option<String> {
        let turn = self.wait_turn(session);
        let _guard = turn.lock().unwrap();
        match text.split_whitespace().next()? {
            "/start" | "/help" => Some(HELP.into()),
            "/status" => Some(self.status(session)),
            "/clear" => Some(self.clear(session)),
            _ => None,
        }
    }

    fn status(&self, session: &Session) -> String {
        let dir = self.conversation(session).dir;
        let entries = store::load(&dir);
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
        let mut journal = match store::open(&dir) {
            Ok(journal) => journal,
            Err(e) => return format!("⚠️ no pude borrar: {e}"),
        };
        let entries: Vec<Entry> = journal
            .entries()
            .iter()
            .map(|record| record.entry.clone())
            .collect();
        if session::context_messages(&entries).is_empty() {
            return "🧹 no había nada que borrar".into();
        }
        match journal.append(vec![Entry::Reset]) {
            Ok(_) => {
                let _ = journal.maybe_snapshot();
                "🧹 contexto borrado".into()
            }
            Err(e) => format!("⚠️ no pude borrar: {e}"),
        }
    }
}

struct Live<'a> {
    transport: &'a dyn Transport,
    session: &'a Session,
    placeholder: Option<Msg>,
    started: Instant,
    written: Option<Instant>,
    step: String,
}

const STATUS_AFTER: Duration = Duration::from_secs(120);
const STATUS_EVERY: Duration = Duration::from_secs(120);

impl<'a> Live<'a> {
    fn new(transport: &'a dyn Transport, session: &'a Session, placeholder: Option<Msg>) -> Self {
        Self {
            transport,
            session,
            placeholder,
            started: Instant::now(),
            written: None,
            step: String::new(),
        }
    }

    fn on(&mut self, event: &Event) {
        if let Event::ToolStart { name, args, .. } = event {
            self.step = step(name, args);
        }
        self.refresh();
    }

    fn refresh(&mut self) {
        let Some(placeholder) = &self.placeholder else {
            return;
        };
        let elapsed = self.started.elapsed();
        if elapsed < STATUS_AFTER {
            return;
        }
        if self
            .written
            .is_some_and(|last| last.elapsed() < STATUS_EVERY)
        {
            return;
        }
        self.transport
            .status(self.session, placeholder, &self.line(elapsed));
        self.written = Some(Instant::now());
    }

    fn line(&self, elapsed: Duration) -> String {
        let minutes = elapsed.as_secs() / 60;
        if self.step.is_empty() {
            return format!("⏳ sigue · {minutes} min");
        }
        format!("⏳ {} · {minutes} min", self.step)
    }

    fn take(self) -> Option<Msg> {
        self.placeholder
    }
}

fn step(name: &str, args: &str) -> String {
    let args: serde_json::Value = serde_json::from_str(args).unwrap_or(serde_json::Value::Null);
    let detail = ["path", "file_path", "command", "query", "url", "prompt"]
        .iter()
        .find_map(|key| args.get(key)?.as_str())
        .unwrap_or_default();
    let verb = match name {
        "read" | "fetch" => "leyendo",
        "write" => "escribiendo",
        "edit" => "editando",
        "bash" => "corriendo",
        "search" => "buscando",
        "browse" => "navegando",
        other => other,
    };
    if detail.is_empty() {
        return verb.to_string();
    }
    format!("{verb} `{}`", short(detail))
}

fn short(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default().trim();
    let cut: String = line.chars().take(60).collect();
    if line.chars().count() > 60 {
        return format!("{cut}…");
    }
    cut
}

pub struct Hit {
    pub conversation: String,
    pub title: String,
    pub project: String,
    pub role: String,
    pub snippet: String,
}

fn snippet(content: &str, needle: &str) -> Option<String> {
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
    Some(snippet)
}

struct EventSink {
    threshold: Option<usize>,
    events: Option<PathBuf>,
    journal: Option<Journal>,
    pipe: Option<Arc<Pipe>>,
    committed: usize,
    error: Option<String>,
}

impl EventSink {
    fn write(&mut self, entries: Vec<Entry>) -> bool {
        if self.error.is_some() {
            return false;
        }
        match &mut self.journal {
            Some(journal) => match journal.append(entries) {
                Ok(_) => true,
                Err(error) => {
                    self.error = Some(format!("no pude guardar el turno: {error}"));
                    false
                }
            },
            None => true,
        }
    }
}

pub(crate) struct Inflight {
    path: PathBuf,
    _file: std::fs::File,
}

impl Inflight {
    pub(crate) fn acquire(dir: &Path) -> Result<Self, String> {
        let path = dir.join("inflight");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| e.to_string())?;
        let taken = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if taken != 0 {
            return Err("el chat lo está corriendo otra instancia".into());
        }
        Ok(Self { path, _file: file })
    }
}

impl Drop for Inflight {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
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
        let mut entries = vec![Entry::Message {
            message: message.clone(),
        }];
        for call in &message.tool_calls {
            entries.push(Entry::ToolIntent {
                id: call.id.clone(),
                name: call.name.clone(),
                arguments: call.arguments.clone(),
            });
        }
        if self.write(entries) {
            self.committed += 1;
        }
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
        if self.write(vec![Entry::Message {
            message: message.clone(),
        }]) {
            self.committed += 1;
        }
    }
}

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
    std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn search_base(base: &str) -> &str {
    let Some((scheme, rest)) = base.split_once("://") else {
        return base;
    };
    let origin = scheme.len() + 3;
    match rest.find('/') {
        Some(path) => &base[..origin + path],
        None => base,
    }
}

fn user_id(key: &str) -> String {
    key.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .take(512)
        .collect()
}

fn compact(provider: &OpenAI, model: &str, entries: &[Entry]) -> Result<Entry, String> {
    let (summary, tokens_before, retained) =
        session::compact_with(provider, model, entries, CONTEXT_OPTIONS)?;
    Ok(Entry::Compaction {
        summary,
        tokens_before,
        timestamp: session::now_ms(),
        retained,
    })
}

fn compacted(
    journal: &mut Option<Journal>,
    entries: &mut Vec<Entry>,
    provider: &OpenAI,
    model: &str,
) -> Result<Vec<Message>, String> {
    let entry = compact(provider, model, entries)?;
    commit(journal, entry.clone())?;
    entries.push(entry);
    let mut history = session::context_messages_with(entries, CONTEXT_OPTIONS);
    session::drop_incomplete_tool_calls(&mut history);
    Ok(history)
}

fn commit(journal: &mut Option<Journal>, entry: Entry) -> Result<(), String> {
    match journal {
        Some(journal) => journal
            .append(vec![entry])
            .map(|_| ())
            .map_err(|error| error.to_string()),
        None => Ok(()),
    }
}

fn snapshot(journal: &mut Option<Journal>) {
    if let Some(journal) = journal {
        if let Err(error) = journal.maybe_snapshot() {
            eprintln!("jimmy: no pude guardar el snapshot: {error}");
        }
    }
}

fn final_answer(history: &[Message]) -> Option<&str> {
    let last = history.last()?;
    (last.role == "assistant" && last.tool_calls.is_empty()).then_some(last.content.as_str())
}

fn answer(messages: &[Message], fallback: Option<&str>) -> String {
    let mut parts = Vec::new();
    for message in messages {
        if message.role == "assistant" && !message.content.is_empty() {
            parts.push(message.content.trim());
        }
    }
    let out = parts.join("\n\n");
    if !out.trim().is_empty() {
        return out;
    }
    fallback.unwrap_or_default().to_string()
}

fn attachments(dir: &Path, images: &[Image]) -> Vec<String> {
    let uploads = dir.join(media::UPLOADS);
    images
        .iter()
        .filter(|image| Path::new(&image.path).parent() == Some(uploads.as_path()))
        .filter_map(|image| Path::new(&image.path).file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .collect()
}

fn signed(text: &str, author: &str) -> String {
    if author.is_empty() {
        return text.to_string();
    }
    format!("[{author}] {text}")
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
    fn the_search_base_drops_every_path() {
        assert_eq!(
            search_base("https://api.deepseek.com"),
            "https://api.deepseek.com"
        );
        assert_eq!(
            search_base("https://api.deepseek.com/"),
            "https://api.deepseek.com"
        );
        assert_eq!(
            search_base("https://api.deepseek.com/beta"),
            "https://api.deepseek.com"
        );
        assert_eq!(
            search_base("http://127.0.0.1:8787"),
            "http://127.0.0.1:8787"
        );
        assert_eq!(search_base("nonsense"), "nonsense");
    }

    #[test]
    fn a_chat_key_becomes_a_spec_compliant_user_id() {
        assert_eq!(user_id("telegram:123456"), "telegram-123456");
        assert_eq!(
            user_id("web-1792285574758-d12bc10c"),
            "web-1792285574758-d12bc10c"
        );
        assert_eq!(user_id("slack/C01 2"), "slack-C01-2");
        assert_eq!(user_id(&"a".repeat(600)).len(), 512);
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

    #[test]
    fn the_log_keeps_the_name_of_the_files_that_live_in_uploads() {
        let dir = Path::new("/data/chats/web-1");
        let uploaded = Image {
            path: "/data/chats/web-1/uploads/17-foto.png".into(),
            url: "data:image/png;base64,QUJD".into(),
        };
        let temporary = Image {
            path: "/tmp/jimmy-image-17.png".into(),
            url: "data:image/png;base64,QUJD".into(),
        };
        assert_eq!(attachments(dir, &[uploaded, temporary]), ["17-foto.png"]);
    }

    #[test]
    fn answer_keeps_text_that_came_with_tool_calls() {
        let messages = vec![
            assistant("primera parte", vec![call()]),
            assistant("segunda parte", Vec::new()),
        ];
        assert_eq!(
            answer(&messages, Some("✅ listo")),
            "primera parte\n\nsegunda parte"
        );
    }

    #[test]
    fn answer_falls_back_when_there_is_only_a_tool_call() {
        let messages = vec![assistant("", vec![call()])];
        assert_eq!(answer(&messages, Some("✅ listo")), "✅ listo");
    }

    #[test]
    fn answer_stays_empty_without_a_fallback() {
        let messages = vec![assistant("", vec![call()])];
        assert_eq!(answer(&messages, None), "");
    }

    #[derive(Default)]
    struct Fake {
        answers: Mutex<Vec<String>>,
        failures: Mutex<Vec<String>>,
        statuses: Mutex<Vec<String>>,
    }

    impl Transport for Fake {
        fn parse_target(&self, _: &str) -> Result<Session, String> {
            Ok(Session::channel("x"))
        }
        fn progress(&self, _: &Session) -> Option<Msg> {
            None
        }
        fn status(&self, _: &Session, _: &Msg, text: &str) {
            self.statuses.lock().unwrap().push(text.to_string());
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
            .respond(&fake, &Session::channel("x"), "hola", Vec::new(), "bob")
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
    fn what_jimmy_send_leaves_in_the_queue_ends_up_in_the_log() {
        let root = resume_dir("media");
        let mut agent = agent_in(&root);
        agent.use_worker_exe(worker_script(
            "media.sh",
            "echo '{\"event\":\"ready\"}'
while read -r line; do
  case \"$line\" in *shutdown*) exit 0 ;; esac
  echo '{\"event\":\"done\",\"text\":\"listo\"}'
done
",
        ));
        let conversation = conversations::get(&root, Path::new("/tmp"), "x");
        media::queue(
            &conversation,
            &Event::Image {
                name: "17-foto.png".into(),
                caption: "mirá".into(),
            },
        )
        .unwrap();

        agent
            .respond(
                &Fake::default(),
                &Session::channel("x"),
                "hola",
                Vec::new(),
                "bob",
            )
            .unwrap();

        let log = std::fs::read_to_string(root.join("chats/x/conversation.jsonl")).unwrap();
        assert!(log.contains("\"event\":\"image\""), "{log}");
        assert!(log.contains("\"name\":\"17-foto.png\""), "{log}");
        assert!(
            !conversation.dir.join("outbox.jsonl").exists(),
            "la cola queda vacía"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_message_shows_up_before_the_turn_ends() {
        let root = resume_dir("announce");
        let go = root.join("go");
        let mut agent = agent_in(&root);
        agent.use_worker_exe(worker_script(
            "slow.sh",
            &format!(
                "echo '{{\"event\":\"ready\"}}'
while read -r line; do
  case \"$line\" in *shutdown*) exit 0 ;; esac
  while [ ! -f {} ]; do sleep 0.05; done
  echo '{{\"event\":\"done\",\"text\":\"eco\"}}'
done
",
                go.display()
            ),
        ));
        let log = root.join("chats/x/conversation.jsonl");
        let running = agent.clone();
        let handle = std::thread::spawn(move || {
            running
                .respond(
                    &Fake::default(),
                    &Session::channel("x"),
                    "hola",
                    Vec::new(),
                    "ana",
                )
                .unwrap();
        });
        let mut text = String::new();
        for _ in 0..400 {
            text = std::fs::read_to_string(&log).unwrap_or_default();
            if text.contains("\"user\"") {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(text.contains("\"author\":\"ana\""), "{text}");
        assert!(!text.contains("\"done\""), "el turno sigue: {text}");

        std::fs::write(&go, "anda").unwrap();
        handle.join().unwrap();
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(text.contains("\"done\""), "{text}");
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
        let (_, live) = agent.bus().attach("x", "bob");
        let fake = Fake::default();
        agent
            .respond(&fake, &Session::channel("x"), "hola", Vec::new(), "bob")
            .unwrap();

        let seen: Vec<String> = live
            .iter()
            .take(5)
            .map(|event| serde_json::to_string(&event).unwrap())
            .filter(|line| !line.contains("\"presence\"") && !line.contains("\"online\""))
            .collect();
        assert!(seen[0].contains("\"user\""), "{seen:?}");
        assert!(seen[0].contains("hola"), "{seen:?}");
        assert!(seen[0].contains("bob"), "{seen:?}");
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
            .respond(&fake, &Session::channel("x"), "hola", Vec::new(), "bob")
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
    fn the_step_says_what_the_tool_is_doing() {
        assert_eq!(
            step("read", r#"{"path":"src/web.rs"}"#),
            "leyendo `src/web.rs`"
        );
        assert_eq!(
            step("bash", r#"{"command":"cargo test"}"#),
            "corriendo `cargo test`"
        );
        assert_eq!(step("read", "no es json"), "leyendo");
        assert_eq!(step("raro", r#"{"path":"x"}"#), "raro `x`");
    }

    #[test]
    fn a_long_command_is_cut_to_one_line() {
        let args = format!(r#"{{"command":"{}"}}"#, "x".repeat(200));
        let line = step("bash", &args);
        assert!(line.contains('…'), "{line}");
        assert!(line.chars().count() < 90, "{line}");
        assert_eq!(step("bash", r#"{"command":"uno\ndos"}"#), "corriendo `uno`");
    }

    fn tool_start(name: &str, args: &str) -> Event {
        Event::ToolStart {
            id: "1".into(),
            name: name.into(),
            args: args.into(),
        }
    }

    #[test]
    fn the_message_stays_quiet_until_the_turn_gets_long() {
        let fake = Fake::default();
        let session = Session::channel("x");
        let mut live = Live::new(&fake, &session, Some(Msg("1".into())));
        live.on(&tool_start("read", r#"{"path":"a.rs"}"#));
        assert!(fake.statuses.lock().unwrap().is_empty());

        live.started = Instant::now() - Duration::from_secs(121);
        live.on(&tool_start("bash", r#"{"command":"make"}"#));
        let said = fake.statuses.lock().unwrap().clone();
        assert_eq!(said.len(), 1);
        assert!(said[0].contains("corriendo `make`"), "{said:?}");
        assert!(said[0].contains("2 min"), "{said:?}");

        live.on(&tool_start("read", r#"{"path":"otro.rs"}"#));
        assert_eq!(fake.statuses.lock().unwrap().len(), 1);
    }

    #[test]
    fn without_a_message_on_screen_nothing_is_said() {
        let fake = Fake::default();
        let session = Session::channel("x");
        let mut live = Live::new(&fake, &session, None);
        live.started = Instant::now() - Duration::from_secs(600);
        live.on(&tool_start("bash", r#"{"command":"make"}"#));
        assert!(fake.statuses.lock().unwrap().is_empty());
    }

    #[test]
    fn clear_appends_a_reset_instead_of_rewriting_the_history() {
        let root = resume_dir("clear");
        let session = Session::channel("x");
        let agent = agent_in(&root);
        let dir = agent.conversation(&session).dir;
        std::fs::create_dir_all(&dir).unwrap();
        let mut journal = store::open(&dir).unwrap();
        journal
            .append(vec![Entry::Message {
                message: assistant("hola", Vec::new()),
            }])
            .unwrap();
        drop(journal);

        let reply = agent.command(&session, "/clear").unwrap();
        assert!(reply.contains("borrado"), "{reply}");

        let records = Journal::load(&dir).unwrap();
        assert!(matches!(records.last().unwrap().entry, Entry::Reset));
        let entries: Vec<Entry> = records.into_iter().map(|record| record.entry).collect();
        assert!(session::context_messages(&entries).is_empty());
        assert!(agent.command(&session, "/clear").unwrap().contains("nada"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_sink_commits_each_message_to_the_journal() {
        let dir = resume_dir("sink");
        let mut sink = EventSink {
            threshold: None,
            events: None,
            journal: Some(store::open(&dir).unwrap()),
            pipe: None,
            committed: 0,
            error: None,
        };
        sink.assistant(0, &assistant("uno", vec![call()]), axe::Usage::default());
        sink.tool(0, &message("tool", "dos"));
        assert_eq!(sink.committed, 2);
        assert!(sink.error.is_none());
        let records = Journal::load(&dir).unwrap();
        assert_eq!(records.len(), 3);
        assert!(matches!(
            records[1].entry,
            Entry::ToolIntent { ref id, .. } if id == "1"
        ));
        assert_eq!(
            records[2].entry,
            Entry::Message {
                message: message("tool", "dos")
            }
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
