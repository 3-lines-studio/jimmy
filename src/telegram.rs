use serde::Deserialize;
use serde_json::{json, Value};
use std::io::Read;
use std::time::Duration;

#[derive(Clone)]
pub struct Telegram {
    token: String,
    http: ureq::Agent,
}

#[derive(Debug, Deserialize)]
pub struct Update {
    pub update_id: i64,
    pub message: Option<Incoming>,
}

#[derive(Debug, Deserialize)]
pub struct Incoming {
    pub chat: Chat,
    pub from: Option<User>,
    pub text: Option<String>,
    #[serde(default)]
    pub caption: Option<String>,
    #[serde(default)]
    pub photo: Vec<PhotoSize>,
    #[serde(default)]
    pub voice: Option<Voice>,
    #[serde(default)]
    pub document: Option<Document>,
}

#[derive(Debug, Deserialize)]
pub struct Voice {
    pub file_id: String,
    #[serde(default)]
    pub duration: u64,
}

#[derive(Debug, Deserialize)]
pub struct PhotoSize {
    pub file_id: String,
}

#[derive(Debug, Deserialize)]
pub struct Document {
    pub file_id: String,
    #[serde(default)]
    pub mime_type: String,
}

#[derive(Debug, Deserialize)]
pub struct Chat {
    pub id: i64,
}

#[derive(Debug, Deserialize)]
pub struct User {
    pub id: i64,
    #[serde(default)]
    pub is_bot: bool,
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

    pub fn get_updates(&self, offset: i64) -> Result<Vec<Update>, String> {
        let value = self.call(
            "getUpdates",
            json!({ "offset": offset, "timeout": 30, "allowed_updates": ["message"] }),
        )?;
        let result = value.get("result").cloned().unwrap_or_else(|| json!([]));
        serde_json::from_value(result).map_err(|e| e.to_string())
    }

    pub fn get_file(&self, file_id: &str) -> Result<String, String> {
        let value = self.call("getFile", json!({ "file_id": file_id }))?;
        value
            .pointer("/result/file_path")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| "telegram getFile: falta file_path".to_string())
    }

    pub fn download(&self, file_path: &str) -> Result<Vec<u8>, String> {
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

    pub fn send_message(&self, chat_id: i64, text: &str) -> Result<i64, String> {
        self.send(chat_id, text, None)
    }

    pub fn send_html(&self, chat_id: i64, text: &str) -> Result<i64, String> {
        self.send(chat_id, text, Some("HTML"))
    }

    fn send(&self, chat_id: i64, text: &str, parse_mode: Option<&str>) -> Result<i64, String> {
        let mut body = json!({
            "chat_id": chat_id,
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

    pub fn edit_message(&self, chat_id: i64, message_id: i64, text: &str) -> Result<(), String> {
        self.edit(chat_id, message_id, text, None)
    }

    pub fn edit_html(&self, chat_id: i64, message_id: i64, text: &str) -> Result<(), String> {
        self.edit(chat_id, message_id, text, Some("HTML"))
    }

    fn edit(
        &self,
        chat_id: i64,
        message_id: i64,
        text: &str,
        parse_mode: Option<&str>,
    ) -> Result<(), String> {
        let mut body = json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "text": text,
            "disable_web_page_preview": true,
        });
        if let Some(mode) = parse_mode {
            body["parse_mode"] = json!(mode);
        }
        self.call("editMessageText", body).map(|_| ())
    }

    pub fn delete_message(&self, chat_id: i64, message_id: i64) {
        let _ = self.call(
            "deleteMessage",
            json!({ "chat_id": chat_id, "message_id": message_id }),
        );
    }
}
