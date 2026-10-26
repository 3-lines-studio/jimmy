//! Cliente MCP mínimo sobre Streamable HTTP: `initialize`, `tools/list` y
//! `tools/call`. El protocolo es JSON-RPC 2.0, y la respuesta llega como JSON
//! o como un stream de eventos; las dos formas se aceptan.
//!
//! Sin daemon: cada llamada es un viaje. La sesión que el servidor pida se
//! guarda en el vault y se reenvía, para que el servidor no tenga que hablar
//! con un proceso que no existe entre llamadas.

use std::time::Duration;

const USER_AGENT: &str = "jimmy (+https://github.com/3-lines-studio/jimmy)";
const PROTOCOL: &str = "2025-06-18";

#[derive(Debug, Clone)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub schema: serde_json::Value,
}

pub struct Session {
    http: ureq::Agent,
    url: String,
    token: String,
    id: Option<String>,
    next: u64,
}

impl Session {
    pub fn new(url: &str, token: &str) -> Session {
        let http = ureq::AgentBuilder::new()
            .timeout_read(Duration::from_secs(30))
            .timeout_write(Duration::from_secs(30))
            .timeout_connect(Duration::from_secs(10))
            .user_agent(USER_AGENT)
            .build();
        Session {
            http,
            url: url.to_string(),
            token: token.to_string(),
            id: None,
            next: 1,
        }
    }

    pub fn connect(url: &str, token: &str) -> Result<Session, String> {
        let mut session = Session::new(url, token);
        session.initialize()?;
        Ok(session)
    }

    fn initialize(&mut self) -> Result<(), String> {
        let result = self.call(
            "initialize",
            serde_json::json!({
                "protocolVersion": PROTOCOL,
                "capabilities": {},
                "clientInfo": { "name": "jimmy", "version": env!("CARGO_PKG_VERSION") },
            }),
        )?;
        let _ = result;
        self.notify("notifications/initialized")?;
        Ok(())
    }

    pub fn tools(&mut self) -> Result<Vec<Tool>, String> {
        let result = self.call("tools/list", serde_json::json!({}))?;
        let list = result
            .get("tools")
            .and_then(serde_json::Value::as_array)
            .ok_or("la respuesta no trae tools")?;
        Ok(list
            .iter()
            .filter_map(|tool| {
                let name = tool.get("name")?.as_str()?.to_string();
                Some(Tool {
                    name,
                    description: tool
                        .get("description")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    schema: tool
                        .get("inputSchema")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                })
            })
            .collect())
    }

    pub fn call_tool(&mut self, name: &str, arguments: &str) -> Result<String, String> {
        let arguments: serde_json::Value = serde_json::from_str(if arguments.trim().is_empty() {
            "{}"
        } else {
            arguments
        })
        .map_err(|error| format!("argumentos inválidos: {error}"))?;
        let result = self.call(
            "tools/call",
            serde_json::json!({ "name": name, "arguments": arguments }),
        )?;
        Ok(text(&result))
    }

    fn call(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let id = self.next;
        self.next += 1;
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let reply = self.post(&body.to_string())?;
        let message = messages(&reply)
            .into_iter()
            .find(|message| {
                message.get("id").and_then(serde_json::Value::as_i64) == Some(id as i64)
            })
            .ok_or_else(|| format!("{method}: el servidor no contestó"))?;
        if let Some(error) = message.get("error") {
            return Err(format!("{method}: {}", error));
        }
        Ok(message
            .get("result")
            .cloned()
            .unwrap_or(serde_json::Value::Null))
    }

    fn notify(&mut self, method: &str) -> Result<(), String> {
        let body = serde_json::json!({ "jsonrpc": "2.0", "method": method });
        self.post(&body.to_string())?;
        Ok(())
    }

    fn post(&self, body: &str) -> Result<String, String> {
        let mut request = self
            .http
            .post(&self.url)
            .set("Content-Type", "application/json")
            .set("Accept", "application/json, text/event-stream")
            .set("Authorization", &format!("Bearer {}", self.token))
            .set("MCP-Protocol-Version", PROTOCOL);
        if let Some(id) = &self.id {
            request = request.set("Mcp-Session-Id", id);
        }
        match request.send_string(body) {
            Ok(response) => {
                if response.status() == 202 {
                    return Ok(String::new());
                }
                response
                    .into_string()
                    .map_err(|error| format!("{}: {error}", self.url))
            }
            Err(ureq::Error::Status(code, response)) => {
                let text = response.into_string().unwrap_or_default();
                Err(format!("{}: http {code}: {text}", self.url))
            }
            Err(error) => Err(format!("{}: {error}", self.url)),
        }
    }
}

/// Las respuestas SSE llegan como `data: {...}` por línea; las de JSON, como un
/// objeto. Las dos se normalizan a una lista de mensajes.
fn messages(body: &str) -> Vec<serde_json::Value> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return serde_json::from_str::<serde_json::Value>(trimmed)
            .map(|value| match value {
                serde_json::Value::Array(list) => list,
                other => vec![other],
            })
            .unwrap_or_default();
    }
    trimmed
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .filter_map(|data| serde_json::from_str(data.trim()).ok())
        .collect()
}

/// El contenido de un `tools/call`: los `TextContent` concatenados, o el JSON
/// estructurado si no hubo texto.
fn text(result: &serde_json::Value) -> String {
    let mut out = String::new();
    if let Some(content) = result.get("content").and_then(serde_json::Value::as_array) {
        for item in content {
            if let Some(value) = item.get("text").and_then(serde_json::Value::as_str) {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(value);
            }
        }
    }
    if out.is_empty() {
        if let Some(structured) = result.get("structuredContent") {
            return structured.to_string();
        }
        return result.to_string();
    }
    if result
        .get("isError")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        return format!("error: {out}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_json_reply_is_one_message() {
        let messages = messages(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["id"], 1);
    }

    #[test]
    fn an_sse_reply_is_split_by_data_lines() {
        let body = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n\n";
        let messages = messages(body);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["id"], 1);
    }

    #[test]
    fn an_empty_reply_is_no_message() {
        assert!(messages("").is_empty());
        assert!(messages("   ").is_empty());
    }

    #[test]
    fn the_tool_text_joins_every_block() {
        let result = serde_json::json!({
            "content": [{ "type": "text", "text": "uno" }, { "type": "text", "text": "dos" }]
        });
        assert_eq!(text(&result), "uno\ndos");
    }

    #[test]
    fn an_error_tool_result_is_flagged() {
        let result = serde_json::json!({
            "isError": true,
            "content": [{ "type": "text", "text": "no pude" }]
        });
        assert_eq!(text(&result), "error: no pude");
    }

    #[test]
    fn structured_content_is_the_fallback() {
        let result = serde_json::json!({ "structuredContent": { "total": 3 } });
        assert_eq!(text(&result), "{\"total\":3}");
    }
}
