use super::{Event, EventSource, Msg, Session, Transport};
use crate::markdown;
use serde_json::{json, Value};
use std::io::Read;
use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message, WebSocket};

const MESSAGE_CHARS: usize = 3500;
const MARKDOWN_CHARS: usize = 3500;

#[derive(Clone)]
pub struct Slack {
    bot_token: String,
    app_token: String,
    http: ureq::Agent,
}

impl Slack {
    pub fn new(bot_token: String, app_token: String) -> Self {
        let http = ureq::AgentBuilder::new()
            .timeout_read(Duration::from_secs(75))
            .timeout_write(Duration::from_secs(30))
            .timeout_connect(Duration::from_secs(20))
            .build();
        Self {
            bot_token,
            app_token,
            http,
        }
    }

    fn api(&self, token: &str, method: &str, body: Option<Value>) -> Result<Value, String> {
        let url = format!("https://slack.com/api/{method}");
        let request = self
            .http
            .post(&url)
            .set("Authorization", &format!("Bearer {token}"));
        let response = match body {
            Some(body) => request
                .set("Content-Type", "application/json; charset=utf-8")
                .send_string(&body.to_string()),
            None => request.send_string(""),
        };
        let value: Value = match response {
            Ok(response) => {
                let text = response.into_string().map_err(|e| e.to_string())?;
                serde_json::from_str(&text).map_err(|e| format!("slack {method}: {e}: {text}"))?
            }
            Err(ureq::Error::Status(code, response)) => {
                let response = response.into_string().unwrap_or_default();
                return Err(format!("slack {method} http {code}: {response}"));
            }
            Err(e) => return Err(format!("slack {method}: {e}")),
        };
        if value.get("ok").and_then(Value::as_bool) != Some(true) {
            let error = value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("error");
            return Err(format!("slack {method}: {error}"));
        }
        Ok(value)
    }

    fn bot_api(&self, method: &str, body: Value) -> Result<Value, String> {
        self.api(&self.bot_token, method, Some(body))
    }

    fn post(&self, session: &Session, text: &str) -> Result<String, String> {
        let mut body = json!({ "channel": session.channel, "text": text });
        if let Some(thread) = &session.thread {
            body["thread_ts"] = json!(thread);
        }
        let value = self.bot_api("chat.postMessage", body)?;
        Ok(value
            .get("ts")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string())
    }

    fn update(&self, session: &Session, ts: &str, text: &str) -> Result<(), String> {
        self.bot_api(
            "chat.update",
            json!({ "channel": session.channel, "ts": ts, "text": text }),
        )
        .map(|_| ())
    }

    fn delete(&self, session: &Session, ts: &str) {
        let _ = self.bot_api(
            "chat.delete",
            json!({ "channel": session.channel, "ts": ts }),
        );
    }
}

impl Transport for Slack {
    fn parse_target(&self, target: &str) -> Result<Session, String> {
        let target = target.trim();
        if target.is_empty() {
            return Err("destino vacío".into());
        }
        let (channel, thread) = match target.split_once('/') {
            Some((channel, thread)) if !channel.is_empty() && !thread.is_empty() => {
                (channel, Some(thread.to_string()))
            }
            Some(_) => return Err(format!("destino inválido: {target}")),
            None => (target, None),
        };
        Ok(Session {
            channel: channel.to_string(),
            thread,
        })
    }

    fn progress(&self, session: &Session) -> Option<Msg> {
        self.post(session, "⚙️ pensando…").ok().map(Msg)
    }

    fn answer(&self, session: &Session, placeholder: Option<Msg>, markdown: &str) {
        let mut parts = markdown::split(markdown, MARKDOWN_CHARS).into_iter();
        if let Some(placeholder) = placeholder {
            match parts.next() {
                Some(first) => {
                    if self
                        .update(session, &placeholder.0, &to_mrkdwn(&first))
                        .is_err()
                    {
                        self.delete(session, &placeholder.0);
                        let _ = self.post(session, &to_mrkdwn(&first));
                    }
                }
                None => self.delete(session, &placeholder.0),
            }
        }
        for part in parts {
            let _ = self.post(session, &to_mrkdwn(&part));
        }
    }

    fn note(&self, session: &Session, text: &str) {
        let _ = self.post(session, &escape(text));
    }

    fn fail(&self, session: &Session, placeholder: Option<Msg>, text: &str) {
        let text = escape(text);
        let mut parts = markdown::split(&text, MESSAGE_CHARS).into_iter();
        if let Some(placeholder) = placeholder {
            match parts.next() {
                Some(first) => {
                    if self.update(session, &placeholder.0, &first).is_err() {
                        self.delete(session, &placeholder.0);
                        let _ = self.post(session, &first);
                    }
                }
                None => self.delete(session, &placeholder.0),
            }
        }
        for part in parts {
            let _ = self.post(session, &part);
        }
    }

    fn download(&self, id: &str) -> Result<(String, Vec<u8>), String> {
        let response = self
            .http
            .get(id)
            .set("Authorization", &format!("Bearer {}", self.bot_token))
            .call()
            .map_err(|e| format!("slack file: {e}"))?;
        let name = id.rsplit('/').next().unwrap_or("file").to_string();
        let mut data = Vec::new();
        response
            .into_reader()
            .read_to_end(&mut data)
            .map_err(|e| e.to_string())?;
        Ok((name, data))
    }

    fn send_media(
        &self,
        session: &Session,
        path: &Path,
        caption: Option<&str>,
    ) -> Result<Msg, String> {
        let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let filename = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file");
        let open = self.bot_api(
            "files.getUploadURLExternal",
            json!({ "filename": filename, "length": data.len() }),
        )?;
        let upload_url = open
            .get("upload_url")
            .and_then(Value::as_str)
            .ok_or("slack files.getUploadURLExternal: falta upload_url")?;
        let file_id = open
            .get("file_id")
            .and_then(Value::as_str)
            .ok_or("slack files.getUploadURLExternal: falta file_id")?;
        self.http
            .post(upload_url)
            .set("Content-Type", "application/octet-stream")
            .send_bytes(&data)
            .map_err(|e| format!("slack upload: {e}"))?;
        let mut body = json!({
            "files": [{ "id": file_id }],
            "channel_id": session.channel,
        });
        if let Some(caption) = caption {
            body["initial_comment"] = json!(caption);
        }
        if let Some(thread) = &session.thread {
            body["thread_ts"] = json!(thread);
        }
        self.bot_api("files.completeUploadExternal", body)?;
        Ok(Msg(file_id.to_string()))
    }
}

pub struct Events {
    slack: Slack,
    socket: Option<WebSocket<MaybeTlsStream<TcpStream>>>,
}

impl Events {
    pub fn new(slack: Slack) -> Self {
        Self {
            slack,
            socket: None,
        }
    }

    fn open(&mut self) -> Result<(), String> {
        let value = self
            .slack
            .api(&self.slack.app_token, "apps.connections.open", None)?;
        let url = value
            .get("url")
            .and_then(Value::as_str)
            .ok_or("slack apps.connections.open: falta url")?;
        let (socket, _) = connect(url).map_err(|e| format!("slack socket: {e}"))?;
        self.socket = Some(socket);
        Ok(())
    }
}

impl EventSource for Events {
    fn recv(&mut self) -> Result<Vec<Event>, String> {
        if self.socket.is_none() {
            self.open()?;
        }
        loop {
            let frame = {
                let socket = self.socket.as_mut().expect("socket abierto");
                match socket.read() {
                    Ok(frame) => frame,
                    Err(e) => {
                        self.socket = None;
                        return Err(format!("slack socket: {e}"));
                    }
                }
            };
            let text = match frame {
                Message::Text(text) => text.as_str().to_string(),
                Message::Close(_) => {
                    self.socket = None;
                    return Err("slack socket: cerrado".into());
                }
                _ => continue,
            };
            let Ok(value) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if let Some(envelope) = value.get("envelope_id").and_then(Value::as_str) {
                if let Some(socket) = self.socket.as_mut() {
                    let _ = socket.send(Message::text(
                        json!({ "envelope_id": envelope }).to_string(),
                    ));
                }
            }
            match value.get("type").and_then(Value::as_str) {
                Some("events_api") => match event_from_payload(&value) {
                    Some(event) => return Ok(vec![event]),
                    None => continue,
                },
                Some("disconnect") => {
                    self.socket = None;
                    return Ok(Vec::new());
                }
                _ => continue,
            }
        }
    }
}

fn event_from_payload(value: &Value) -> Option<Event> {
    let event = value.pointer("/payload/event")?;
    if event.get("bot_id").is_some() {
        return None;
    }
    match event.get("subtype").and_then(Value::as_str) {
        None | Some("file_share") => {}
        Some(_) => return None,
    }
    let kind = event.get("type").and_then(Value::as_str)?;
    let channel = event.get("channel").and_then(Value::as_str)?;
    let ts = event.get("ts").and_then(Value::as_str)?;
    let is_dm = event.get("channel_type").and_then(Value::as_str) == Some("im");
    if !is_dm && kind != "app_mention" {
        return None;
    }
    let text = strip_mentions(
        event
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    let image = event
        .get("files")
        .and_then(Value::as_array)
        .and_then(|files| files.iter().find_map(slack_image));
    if text.is_empty() && image.is_none() {
        return None;
    }
    let thread = if is_dm {
        None
    } else {
        Some(
            event
                .get("thread_ts")
                .and_then(Value::as_str)
                .unwrap_or(ts)
                .to_string(),
        )
    };
    Some(Event {
        session: Session {
            channel: channel.to_string(),
            thread,
        },
        sender: event
            .get("user")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        is_bot: false,
        text,
        image,
        voice: None,
    })
}

fn slack_image(file: &Value) -> Option<String> {
    let mimetype = file
        .get("mimetype")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !mimetype.starts_with("image/") {
        return None;
    }
    file.get("url_private_download")
        .or_else(|| file.get("url_private"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn strip_mentions(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("<@") {
        out.push_str(&rest[..start]);
        match rest[start..].find('>') {
            Some(end) => rest = &rest[start + end + 1..],
            None => return text.trim().to_string(),
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

fn to_mrkdwn(markdown: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            in_code = !in_code;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if in_code {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if let Some(text) = heading(trimmed) {
            out.push('*');
            out.push_str(&inline(text));
            out.push_str("*\n");
            continue;
        }
        out.push_str(&inline(line));
        out.push('\n');
    }
    out.trim_end().to_string()
}

fn heading(line: &str) -> Option<&str> {
    let hashes = line.bytes().take_while(|byte| *byte == b'#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    line[hashes..].strip_prefix(' ').map(str::trim_start)
}

fn inline(line: &str) -> String {
    let escaped = escape(line);
    let linked = links(&escaped);
    linked.replace("**", "*").replace("~~", "~")
}

fn links(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        let Some(close) = rest[start + 1..].find("](") else {
            break;
        };
        let label = &rest[start + 1..start + 1 + close];
        let tail = &rest[start + 1 + close + 2..];
        let Some(end) = tail.find(')') else {
            break;
        };
        out.push_str(&rest[..start]);
        out.push('<');
        out.push_str(&tail[..end]);
        out.push('|');
        out.push_str(label);
        out.push('>');
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_splits_channel_and_thread() {
        let slack = Slack::new("x".into(), "x".into());
        assert_eq!(
            slack.parse_target("C0123").unwrap(),
            Session {
                channel: "C0123".into(),
                thread: None
            }
        );
        assert_eq!(
            slack.parse_target("C0123/1726570000.123456").unwrap(),
            Session {
                channel: "C0123".into(),
                thread: Some("1726570000.123456".into())
            }
        );
        assert!(slack.parse_target("C0123/").is_err());
        assert!(slack.parse_target("").is_err());
    }

    #[test]
    fn mrkdwn_bolds_links_and_escapes() {
        assert_eq!(to_mrkdwn("**hola**"), "*hola*");
        assert_eq!(
            to_mrkdwn("[la web](https://a.com)"),
            "<https://a.com|la web>"
        );
        assert_eq!(to_mrkdwn("a < b & c"), "a &lt; b &amp; c");
        assert_eq!(to_mrkdwn("## Título"), "*Título*");
    }

    #[test]
    fn code_fences_are_not_escaped() {
        assert_eq!(to_mrkdwn("```\na < b\n```"), "```\na < b\n```");
    }

    #[test]
    fn strip_mentions_removes_bot_pings() {
        assert_eq!(strip_mentions("<@U123> hola"), "hola");
        assert_eq!(strip_mentions("hola <@U123|jimmy>"), "hola");
        assert_eq!(strip_mentions("sin menciones"), "sin menciones");
    }

    #[test]
    fn app_mention_opens_a_thread_on_its_own_ts() {
        let value = json!({
            "type": "events_api",
            "payload": { "event": {
                "type": "app_mention",
                "channel": "C1",
                "ts": "10.1",
                "user": "U1",
                "text": "<@U9> hola"
            }}
        });
        let event = event_from_payload(&value).unwrap();
        assert_eq!(event.session.key(), "C1/10.1");
        assert_eq!(event.text, "hola");
        assert_eq!(event.sender, "U1");
    }

    #[test]
    fn replies_keep_the_root_thread() {
        let value = json!({
            "type": "events_api",
            "payload": { "event": {
                "type": "app_mention",
                "channel": "C1",
                "ts": "11.2",
                "thread_ts": "10.1",
                "user": "U1",
                "text": "seguimos"
            }}
        });
        assert_eq!(event_from_payload(&value).unwrap().session.key(), "C1/10.1");
    }

    #[test]
    fn a_dm_is_one_session_without_a_thread() {
        let value = json!({
            "type": "events_api",
            "payload": { "event": {
                "type": "message",
                "channel": "D1",
                "channel_type": "im",
                "ts": "12.3",
                "user": "U1",
                "text": "che"
            }}
        });
        let event = event_from_payload(&value).unwrap();
        assert_eq!(event.session.key(), "D1");
    }

    #[test]
    fn bot_messages_are_ignored() {
        let value = json!({
            "type": "events_api",
            "payload": { "event": {
                "type": "message",
                "channel": "D1",
                "channel_type": "im",
                "ts": "12.3",
                "bot_id": "B1",
                "text": "eco"
            }}
        });
        assert!(event_from_payload(&value).is_none());
    }
}
