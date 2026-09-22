//! Mandar el mail con el link, por Resend.

use std::time::Duration;

const DEFAULT_BASE: &str = "https://api.resend.com";

pub struct Mail {
    key: String,
    from: String,
    endpoint: String,
    http: ureq::Agent,
}

impl Mail {
    /// Sin clave no hay proveedor: en ese caso el link se muestra en pantalla,
    /// que es como se usa en desarrollo.
    pub fn from_env() -> Option<Mail> {
        let key = crate::env("RESEND_API_KEY")?;
        let from = crate::env("JIMMY_WEB_FROM")?;
        let base = crate::env("RESEND_API_BASE").unwrap_or_else(|| DEFAULT_BASE.to_string());
        Some(Mail::new(key, from, &base))
    }

    pub fn new(key: String, from: String, base: &str) -> Mail {
        let http = ureq::AgentBuilder::new()
            .timeout_read(Duration::from_secs(20))
            .timeout_write(Duration::from_secs(20))
            .timeout_connect(Duration::from_secs(10))
            .build();
        Mail {
            key,
            from,
            endpoint: format!("{}/emails", base.trim_end_matches('/')),
            http,
        }
    }

    pub fn send_link(&self, to: &str, link: &str) -> Result<(), String> {
        let html = format!(
            "<p>Entrá a Jimmy con este link:</p>\
             <p><a href=\"{link}\">{link}</a></p>\
             <p>Vence en unos minutos y sirve una sola vez. Si no lo pediste vos, ignoralo.</p>"
        );
        let body = serde_json::json!({
            "from": self.from,
            "to": [to],
            "subject": "Tu link para entrar a Jimmy",
            "html": html,
        });
        let response = self
            .http
            .post(&self.endpoint)
            .set("Authorization", &format!("Bearer {}", self.key))
            .set("Content-Type", "application/json")
            .set(
                "User-Agent",
                "jimmy (+https://github.com/3-lines-studio/jimmy)",
            )
            .send_string(&body.to_string());
        match response {
            Ok(_) => Ok(()),
            Err(ureq::Error::Status(code, response)) => {
                let text = response.into_string().unwrap_or_default();
                Err(format!("resend http {code}: {text}"))
            }
            Err(e) => Err(format!("resend: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// Un Resend de mentira: junta el pedido y contesta que sí.
    fn fake_resend() -> (String, std::sync::mpsc::Receiver<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            while let Ok(read) = stream.read(&mut buffer) {
                request.extend_from_slice(&buffer[..read]);
                let text = String::from_utf8_lossy(&request);
                if text.contains("\r\n\r\n") && text.len() >= body_start(&text).unwrap_or(0) {
                    break;
                }
                if read == 0 {
                    break;
                }
            }
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}");
            let _ = sender.send(String::from_utf8_lossy(&request).to_string());
        });
        (base, receiver)
    }

    fn body_start(request: &str) -> Option<usize> {
        let end = request.find("\r\n\r\n")? + 4;
        let length: usize = request
            .lines()
            .find(|line| line.to_ascii_lowercase().starts_with("content-length:"))
            .and_then(|line| line.split(':').nth(1))
            .and_then(|value| value.trim().parse().ok())?;
        Some(end + length)
    }

    #[test]
    fn the_link_goes_out_as_a_mail() {
        let (base, received) = fake_resend();
        let mail = Mail::new("clave".into(), "Jimmy <jimmy@ejemplo.com>".into(), &base);

        mail.send_link("berti@ejemplo.com", "https://jimmy.ejemplo/auth?token=abc")
            .unwrap();
        let request = received.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(request.starts_with("POST /emails "), "{request}");
        assert!(request.contains("Authorization: Bearer clave"), "{request}");
        assert!(
            request.contains("\"to\":[\"berti@ejemplo.com\"]"),
            "{request}"
        );
        assert!(
            request.contains("https://jimmy.ejemplo/auth?token=abc"),
            "{request}"
        );
    }
}
