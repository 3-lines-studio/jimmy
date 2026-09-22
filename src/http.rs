//! Just enough HTTP/1.1 for a web frontend: request parsing and response
//! writing. One request per connection, no keep-alive.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;

/// Lo más grande que aceptamos leer. Con la web expuesta, un cuerpo sin tope es
/// una forma de quedarse sin memoria.
pub const MAX_BODY: usize = 24 * 1024 * 1024;

pub struct Request {
    pub method: String,
    pub path: String,
    pub query: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
    /// El cuerpo se pasó del tope y no lo leímos.
    pub too_large: bool,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    pub fn param(&self, name: &str) -> Option<&str> {
        self.query.get(name).map(String::as_str)
    }

    pub fn cookie(&self, name: &str) -> Option<String> {
        for part in self.header("cookie")?.split(';') {
            if let Some((key, value)) = part.trim().split_once('=') {
                if key == name {
                    return Some(value.to_string());
                }
            }
        }
        None
    }

    pub fn json(&self) -> Option<serde_json::Value> {
        serde_json::from_slice(&self.body).ok()
    }

    pub fn field(&self, name: &str) -> Option<String> {
        self.json()?
            .get(name)?
            .as_str()
            .map(|value| value.to_string())
    }

    pub fn list(&self, name: &str) -> Vec<String> {
        self.json()
            .and_then(|json| json.get(name).and_then(|value| value.as_array()).cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|value| value.as_str().map(|text| text.to_string()))
            .collect()
    }
}

pub fn read(stream: &mut TcpStream) -> std::io::Result<Option<Request>> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.trim_end().split(' ');
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("/").to_string();
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path.to_string(), parse_pairs(query)),
        None => (target, HashMap::new()),
    };
    let mut headers = HashMap::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            break;
        }
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let too_large = length > MAX_BODY;
    let mut body = vec![0u8; if too_large { 0 } else { length }];
    if !too_large && length > 0 {
        reader.read_exact(&mut body)?;
    }
    Ok(Some(Request {
        method,
        path,
        query,
        headers,
        body,
        too_large,
    }))
}

pub fn respond(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    extra: &[(&str, &str)],
    body: &[u8],
) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n",
        reason(status),
        body.len()
    )?;
    for (name, value) in extra {
        write!(stream, "{name}: {value}\r\n")?;
    }
    stream.write_all(b"\r\n")?;
    stream.write_all(body)?;
    stream.flush()
}

pub fn send_json(
    stream: &mut TcpStream,
    status: u16,
    value: &serde_json::Value,
) -> std::io::Result<()> {
    let body = serde_json::to_vec(value).unwrap_or_default();
    respond(stream, status, "application/json", &[], &body)
}

pub fn send_error(stream: &mut TcpStream, status: u16, message: &str) -> std::io::Result<()> {
    send_json(stream, status, &serde_json::json!({ "error": message }))
}

pub fn send_text(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    text: &str,
) -> std::io::Result<()> {
    respond(stream, status, content_type, &[], text.as_bytes())
}

pub fn sse_open(stream: &mut TcpStream) -> std::io::Result<()> {
    stream.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n",
    )?;
    stream.flush()
}

pub fn sse_data(stream: &mut TcpStream, data: &str) -> std::io::Result<()> {
    stream.write_all(b"data: ")?;
    stream.write_all(data.as_bytes())?;
    stream.write_all(b"\n\n")?;
    stream.flush()
}

pub fn sse_ping(stream: &mut TcpStream) -> std::io::Result<()> {
    stream.write_all(b": ping\n\n")?;
    stream.flush()
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        303 => "See Other",
        400 => "Bad Request",
        413 => "Payload Too Large",
        401 => "Unauthorized",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "OK",
    }
}

fn parse_pairs(input: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for pair in input.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        out.insert(percent_decode(key), percent_decode(value));
    }
    out
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok());
            if let Some(byte) = hex {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(cookie: &str) -> Request {
        let mut headers = HashMap::new();
        headers.insert("cookie".to_string(), cookie.to_string());
        Request {
            method: "GET".into(),
            path: "/".into(),
            query: HashMap::new(),
            headers,
            body: Vec::new(),
            too_large: false,
        }
    }

    #[test]
    fn percent_decode_handles_escapes_and_plus() {
        assert_eq!(percent_decode("a%20b+c"), "a b c");
        assert_eq!(percent_decode("%2Ftmp"), "/tmp");
        assert_eq!(percent_decode("100%"), "100%");
    }

    #[test]
    fn parse_pairs_splits_on_ampersand() {
        let pairs = parse_pairs("user=ana&password=a%3Db");
        assert_eq!(pairs.get("user").unwrap(), "ana");
        assert_eq!(pairs.get("password").unwrap(), "a=b");
    }

    #[test]
    fn cookie_reads_one_value() {
        assert_eq!(
            request("a=1; jimmy_session=abc; b=2")
                .cookie("jimmy_session")
                .as_deref(),
            Some("abc")
        );
        assert_eq!(request("a=1").cookie("jimmy_session"), None);
    }
}
