use crate::telegram::Telegram;
use axe::run::{self, Outcome, RunOptions, Sink};
use axe::session::{self, Entry};
use axe::{Message, OpenAI, ToolCall, Usage};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

const STATUS_CHARS: usize = 3900;
const MESSAGE_CHARS: usize = 4000;
const EDIT_EVERY: Duration = Duration::from_millis(1200);

#[derive(Clone)]
pub struct Agent {
    base: String,
    model: String,
    api_key: String,
    context_window: Option<usize>,
    root: PathBuf,
    workspace: String,
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
        Self {
            base,
            model,
            api_key,
            context_window,
            root,
            workspace,
        }
    }

    pub fn respond(&self, tg: &Telegram, chat_id: i64, text: &str) -> Result<(), String> {
        let dir = self.root.join("chats").join(chat_id.to_string());
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

        let mut entries = load_entries(&dir);
        let mut history = session::context_messages(&entries);
        session::drop_incomplete_tool_calls(&mut history);

        let user = Message {
            role: "user".into(),
            content: text.to_string(),
            tool_calls: Vec::new(),
            tool_call_id: String::new(),
            reasoning: String::new(),
            images: Vec::new(),
        };
        history.push(user.clone());
        entries.push(Entry::Message { message: user });

        let tools = axe::tui::build_tools(&self.workspace);
        let system = axe::system_prompt(&tools, &self.workspace);
        let provider = OpenAI::new(self.base.clone(), self.api_key.clone());
        let opts = RunOptions {
            model: &self.model,
            system: &system,
            tools: &tools,
            max_turns: usize::MAX,
        };
        let threshold = self.context_window.map(|w| w.saturating_sub(16384));
        let cancel = Arc::new(AtomicBool::new(false));

        let status = tg.send_message(chat_id, "⚙️ pensando…").ok();
        let mut sink = TgSink {
            tg: tg.clone(),
            chat_id,
            status,
            sent: "⚙️ pensando…".into(),
            body: String::new(),
            last_edit: Instant::now(),
            threshold,
        };

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
                Outcome::Done => {
                    save_entries(&dir, &entries)?;
                    finalize(tg, chat_id, sink.take_status(), &answer(&end.messages));
                    return Ok(());
                }
                Outcome::MaxTurns => {
                    save_entries(&dir, &entries)?;
                    finalize(tg, chat_id, sink.take_status(), &answer(&end.messages));
                    return Ok(());
                }
                Outcome::Cancelled => {
                    save_entries(&dir, &entries)?;
                    finalize(tg, chat_id, sink.take_status(), "⚠️ interrumpido");
                    return Err("interrumpido".into());
                }
                Outcome::Compact => {
                    history = match compact(&provider, &self.model, &mut entries) {
                        Ok(history) => history,
                        Err(e) => {
                            save_entries(&dir, &entries)?;
                            finalize(tg, chat_id, sink.take_status(), &format!("⚠️ {e}"));
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
                                save_entries(&dir, &entries)?;
                                finalize(tg, chat_id, sink.take_status(), &format!("⚠️ {e}"));
                                return Err(e);
                            }
                        };
                        continue;
                    }
                    save_entries(&dir, &entries)?;
                    finalize(tg, chat_id, sink.take_status(), &format!("⚠️ error: {e}"));
                    return Err(e);
                }
            }
        }
    }
}

struct TgSink {
    tg: Telegram,
    chat_id: i64,
    status: Option<i64>,
    sent: String,
    body: String,
    last_edit: Instant,
    threshold: Option<usize>,
}

impl TgSink {
    fn flush(&mut self, force: bool) {
        let text = clamp(self.body.trim_end(), STATUS_CHARS);
        if text.is_empty() || text == self.sent {
            return;
        }
        if !force && self.last_edit.elapsed() < EDIT_EVERY {
            return;
        }
        if let Some(id) = self.status {
            let _ = self.tg.edit_message(self.chat_id, id, &text);
        } else {
            self.status = self.tg.send_message(self.chat_id, &text).ok();
        }
        self.sent = text;
        self.last_edit = Instant::now();
    }

    fn line(&mut self, text: &str) {
        if !self.body.is_empty() && !self.body.ends_with('\n') {
            self.body.push('\n');
        }
        self.body.push_str(text);
    }

    fn take_status(&mut self) -> Option<i64> {
        self.status.take()
    }
}

impl Sink for TgSink {
    fn assistant_delta(&mut self, text: &str) {
        self.body.push_str(text);
        self.flush(false);
    }

    fn assistant_done(&mut self) {
        self.flush(true);
    }

    fn tool_start(&mut self, call: &ToolCall) {
        self.line(&format!("▸ {}", render_call(call)));
        self.flush(false);
    }

    fn should_compact(&mut self, input: usize, output: usize) -> bool {
        self.threshold
            .is_some_and(|threshold| input.saturating_add(output) > threshold)
    }

    fn tokens(&mut self, _input: usize, _output: usize, _cached_input: usize) {}

    fn tool_result(&mut self, _call: &ToolCall) {}

    fn tool_delta(&mut self, _call: &ToolCall, _text: &str) {}

    fn assistant(&mut self, _turn: usize, _msg: &Message, _usage: Usage) {}

    fn tool(&mut self, _turn: usize, _msg: &Message) {}
}

fn compact(
    provider: &OpenAI,
    model: &str,
    entries: &mut Vec<Entry>,
) -> Result<Vec<Message>, String> {
    let (summary, tokens_before, retained) = session::compact(provider, model, entries)?;
    entries.push(Entry::Compaction {
        summary,
        tokens_before,
        timestamp: session::now_ms(),
        retained,
    });
    let mut out = session::context_messages(entries);
    session::drop_incomplete_tool_calls(&mut out);
    Ok(out)
}

fn finalize(tg: &Telegram, chat_id: i64, status: Option<i64>, text: &str) {
    if let Some(id) = status {
        tg.delete_message(chat_id, id);
    }
    for chunk in chunks(text, MESSAGE_CHARS) {
        let _ = tg.send_message(chat_id, &chunk);
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

fn render_call(call: &ToolCall) -> String {
    #[derive(serde::Deserialize)]
    struct Args {
        path: Option<String>,
        command: Option<String>,
    }
    if let Ok(args) = serde_json::from_str::<Args>(&call.arguments) {
        if let Some(command) = args.command.filter(|c| !c.is_empty()) {
            return format!("{}: {}", call.name, clamp(&command, 200));
        }
        if let Some(path) = args.path.filter(|p| !p.is_empty()) {
            return format!("{}: {}", call.name, path);
        }
    }
    call.name.clone()
}

fn clamp(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
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
