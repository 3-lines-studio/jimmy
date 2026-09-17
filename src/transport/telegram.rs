use super::{Event, EventSource, Msg, Session, Transport};
use crate::markdown;
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::Read;
use std::time::Duration;

const MESSAGE_CHARS: usize = 4000;
const MARKDOWN_CHARS: usize = 3500;

#[derive(Clone)]
pub struct Telegram {
    token: String,
    http: ureq::Agent,
}

#[derive(Debug, Deserialize)]
struct Update {
    update_id: i64,
    message: Option<Incoming>,
}

#[derive(Debug, Deserialize)]
struct Incoming {
    chat: Chat,
    from: Option<User>,
    text: Option<String>,
    #[serde(default)]
    caption: Option<String>,
    #[serde(default)]
    photo: Vec<PhotoSize>,
    #[serde(default)]
    voice: Option<Voice>,
    #[serde(default)]
    document: Option<Document>,
}

#[derive(Debug, Deserialize)]
struct Voice {
    file_id: String,
    #[serde(default)]
    duration: u64,
}

#[derive(Debug, Deserialize)]
struct PhotoSize {
    file_id: String,
}

#[derive(Debug, Deserialize)]
struct Document {
    file_id: String,
    #[serde(default)]
    mime_type: String,
}

#[derive(Debug, Deserialize)]
struct Chat {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct User {
    id: i64,
    #[serde(default)]
    is_bot: bool,
}

impl Telegram {
    pub fn new(token: String) -> Self {
        let http = ureq::AgentBuilder::new()
            .timeout_read(Duration::from_secs(75))
            .timeout_write(Duration::from_secs(30))
            .timeout_connect(Duration::from_secs(20))
            .build();
        Self { token, http }
    }

    fn url(&self, method: &str) -> String {
        format!("https://api.telegram.org/bot{}/{method}", self.token)
    }

    fn call(&self, method: &str, body: Value) -> Result<Value, String> {
        let payload = body.to_string();
        match self
            .http
            .post(&self.url(method))
            .set("Content-Type", "application/json")
            .send_string(&payload)
        {
            Ok(response) => {
                let text = response.into_string().map_err(|e| e.to_string())?;
                serde_json::from_str(&text).map_err(|e| format!("telegram {method}: {e}: {text}"))
            }
            Err(ureq::Error::Status(code, response)) => {
                let body = response.into_string().unwrap_or_default();
                Err(format!("telegram {method} http {code}: {body}"))
            }
            Err(e) => Err(format!("telegram {method}: {e}")),
        }
    }

    fn chat_id(&self, session: &Session) -> Result<i64, String> {
        session
            .channel
            .parse()
            .map_err(|_| format!("chat inválido: {}", session.channel))
    }

    fn get_updates(&self, offset: i64) -> Result<Vec<Update>, String> {
        let value = self.call(
            "getUpdates",
            json!({ "offset": offset, "timeout": 30, "allowed_updates": ["message"] }),
        )?;
        let result = value.get("result").cloned().unwrap_or_else(|| json!([]));
        serde_json::from_value(result).map_err(|e| e.to_string())
    }

    fn get_file(&self, file_id: &str) -> Result<String, String> {
        let value = self.call("getFile", json!({ "file_id": file_id }))?;
        value
            .pointer("/result/file_path")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| "telegram getFile: falta file_path".to_string())
    }

    fn download_file(&self, file_path: &str) -> Result<Vec<u8>, String> {
        let url = format!(
            "https://api.telegram.org/file/bot{}/{}",
            self.token, file_path
        );
        let response = self
            .http
            .get(&url)
            .call()
            .map_err(|e| format!("telegram file: {e}"))?;
        let mut data = Vec::new();
        response
            .into_reader()
            .read_to_end(&mut data)
            .map_err(|e| e.to_string())?;
        Ok(data)
    }

    fn send_message(&self, session: &Session, text: &str) -> Result<i64, String> {
        self.send(session, text, None)
    }

    fn send_html(&self, session: &Session, text: &str) -> Result<i64, String> {
        self.send(session, text, Some("HTML"))
    }

    fn send(&self, session: &Session, text: &str, parse_mode: Option<&str>) -> Result<i64, String> {
        let mut body = json!({
            "chat_id": self.chat_id(session)?,
            "text": text,
            "disable_web_page_preview": true,
        });
        if let Some(mode) = parse_mode {
            body["parse_mode"] = json!(mode);
        }
        let value = self.call("sendMessage", body)?;
        Ok(value
            .pointer("/result/message_id")
            .and_then(Value::as_i64)
            .unwrap_or(0))
    }

    fn edit_message(&self, session: &Session, message_id: i64, text: &str) -> Result<(), String> {
        self.edit(session, message_id, text, None)
    }

    fn edit_html(&self, session: &Session, message_id: i64, text: &str) -> Result<(), String> {
        self.edit(session, message_id, text, Some("HTML"))
    }

    fn edit(
        &self,
        session: &Session,
        message_id: i64,
        text: &str,
        parse_mode: Option<&str>,
    ) -> Result<(), String> {
        let mut body = json!({
            "chat_id": self.chat_id(session)?,
            "message_id": message_id,
            "text": text,
            "disable_web_page_preview": true,
        });
        if let Some(mode) = parse_mode {
            body["parse_mode"] = json!(mode);
        }
        self.call("editMessageText", body).map(|_| ())
    }

    fn delete_message(&self, session: &Session, message_id: i64) {
        let _ = self.call(
            "deleteMessage",
            json!({ "chat_id": self.chat_id(session).unwrap_or(0), "message_id": message_id }),
        );
    }

    fn send_markdown(&self, session: &Session, text: &str) {
        let html = markdown::to_telegram_html(text);
        if self.send_html(session, &html).is_err() {
            let _ = self.send_message(session, text);
        }
    }
}

impl Transport for Telegram {
    fn parse_target(&self, target: &str) -> Result<Session, String> {
        let id: i64 = target
            .trim()
            .parse()
            .map_err(|_| format!("destino inválido: {target}"))?;
        Ok(Session::channel(id.to_string()))
    }

    fn progress(&self, session: &Session) -> Option<Msg> {
        self.send_message(session, "⚙️ pensando…")
            .ok()
            .map(|id| Msg(id.to_string()))
    }

    fn answer(&self, session: &Session, placeholder: Option<Msg>, markdown: &str) {
        let mut parts = markdown::split(markdown, MARKDOWN_CHARS).into_iter();
        if let Some(placeholder) = placeholder {
            let id = placeholder.0.parse().unwrap_or(0);
            match parts.next() {
                Some(first) => {
                    let html = markdown::to_telegram_html(&first);
                    if self.edit_html(session, id, &html).is_err() {
                        self.delete_message(session, id);
                        self.send_markdown(session, &first);
                    }
                }
                None => self.delete_message(session, id),
            }
        }
        for part in parts {
            self.send_markdown(session, &part);
        }
    }

    fn note(&self, session: &Session, text: &str) {
        let _ = self.send_message(session, text);
    }

    fn fail(&self, session: &Session, placeholder: Option<Msg>, text: &str) {
        let mut parts = chunks(text, MESSAGE_CHARS).into_iter();
        if let Some(placeholder) = placeholder {
            let id = placeholder.0.parse().unwrap_or(0);
            match parts.next() {
                Some(first) => {
                    if self.edit_message(session, id, &first).is_err() {
                        self.delete_message(session, id);
                        let _ = self.send_message(session, &first);
                    }
                }
                None => self.delete_message(session, id),
            }
        }
        for part in parts {
            let _ = self.send_message(session, &part);
        }
    }

    fn download(&self, id: &str) -> Result<(String, Vec<u8>), String> {
        let path = self.get_file(id)?;
        let data = self.download_file(&path)?;
        Ok((path, data))
    }
}

pub struct Updates {
    telegram: Telegram,
    offset: i64,
}

impl Updates {
    pub fn new(telegram: Telegram) -> Self {
        Self {
            telegram,
            offset: 0,
        }
    }
}

impl EventSource for Updates {
    fn recv(&mut self) -> Result<Vec<Event>, String> {
        let updates = self.telegram.get_updates(self.offset)?;
        let mut events = Vec::new();
        for update in updates {
            self.offset = update.update_id + 1;
            let Some(message) = update.message else {
                continue;
            };
            let text = message
                .text
                .clone()
                .or_else(|| message.caption.clone())
                .unwrap_or_default();
            let image = message
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
            if text.is_empty() && image.is_none() && voice.is_none() {
                continue;
            }
            let from = message.from.as_ref();
            events.push(Event {
                session: Session::channel(message.chat.id.to_string()),
                sender: from.map(|from| from.id.to_string()).unwrap_or_default(),
                is_bot: from.is_some_and(|from| from.is_bot),
                text,
                image,
                voice,
            });
        }
        Ok(events)
    }
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
