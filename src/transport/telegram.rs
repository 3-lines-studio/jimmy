use super::{Event, EventSource, Msg, Session, Transport};
use crate::markdown;
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::Read;
use std::path::Path;
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

/// Where Telegram lives. Only a test harness changes it.
fn api_base() -> String {
    crate::env("TELEGRAM_API_BASE").unwrap_or_else(|| "https://api.telegram.org".into())
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
        format!("{}/bot{}/{method}", api_base(), self.token)
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
        let url = format!("{}/file/bot{}/{}", api_base(), self.token, file_path);
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

    fn send_media(
        &self,
        session: &Session,
        path: &Path,
        caption: Option<&str>,
    ) -> Result<Msg, String> {
        let (method, field) = media_method(path);
        let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let limit = media_limit(method);
        if data.len() > limit {
            return Err(format!(
                "{} bytes supera el límite de {limit} para {method}",
                data.len()
            ));
        }
        let filename = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file");
        let boundary = format!("jimmy{:x}", axe::session::now_ms());
        let mut body = Vec::new();
        push_field(
            &mut body,
            &boundary,
            "chat_id",
            &self.chat_id(session)?.to_string(),
        );
        if let Some(caption) = caption {
            push_field(&mut body, &boundary, "caption", caption);
        }
        push_file(&mut body, &boundary, field, filename, &data);
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        let response = self
            .http
            .post(&self.url(method))
            .set(
                "Content-Type",
                &format!("multipart/form-data; boundary={boundary}"),
            )
            .send_bytes(&body)
            .map_err(|e| format!("telegram {method}: {e}"))?;
        let text = response.into_string().map_err(|e| e.to_string())?;
        let value: Value =
            serde_json::from_str(&text).map_err(|e| format!("telegram {method}: {e}: {text}"))?;
        if value.get("ok").and_then(Value::as_bool) != Some(true) {
            let description = value
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("error");
            return Err(format!("telegram {method}: {description}"));
        }
        Ok(Msg(value
            .pointer("/result/message_id")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            .to_string()))
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

fn media_method(path: &Path) -> (&'static str, &'static str) {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png" | "jpg" | "jpeg" | "webp" | "gif") => ("sendPhoto", "photo"),
        Some("mp4" | "webm" | "mov" | "mkv") => ("sendVideo", "video"),
        _ => ("sendDocument", "document"),
    }
}

fn media_limit(method: &str) -> usize {
    match method {
        "sendPhoto" => 10 * 1024 * 1024,
        _ => 50 * 1024 * 1024,
    }
}

fn push_field(body: &mut Vec<u8>, boundary: &str, name: &str, value: &str) {
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
        )
        .as_bytes(),
    );
}

fn push_file(body: &mut Vec<u8>, boundary: &str, name: &str, filename: &str, data: &[u8]) {
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(data);
    body.extend_from_slice(b"\r\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_method_picks_by_extension() {
        assert_eq!(media_method(Path::new("a.PNG")), ("sendPhoto", "photo"));
        assert_eq!(media_method(Path::new("a.webp")), ("sendPhoto", "photo"));
        assert_eq!(media_method(Path::new("a.mp4")), ("sendVideo", "video"));
        assert_eq!(
            media_method(Path::new("a.pdf")),
            ("sendDocument", "document")
        );
        assert_eq!(
            media_method(Path::new("noext")),
            ("sendDocument", "document")
        );
    }

    #[test]
    fn photo_limit_is_ten_megabytes() {
        assert_eq!(media_limit("sendPhoto"), 10 * 1024 * 1024);
        assert_eq!(media_limit("sendVideo"), 50 * 1024 * 1024);
        assert_eq!(media_limit("sendDocument"), 50 * 1024 * 1024);
    }
}
