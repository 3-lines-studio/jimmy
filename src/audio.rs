use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MODEL: &str = "whisper-large-v3";
const URL: &str = "https://api.groq.com/openai/v1/audio/transcriptions";
const MAX_SECONDS: u64 = 300;

pub fn transcribe(api_key: &str, path: &Path, duration: u64) -> Result<String, String> {
    if duration > MAX_SECONDS {
        return Err(format!(
            "el audio dura {duration}s y el máximo es {MAX_SECONDS}s"
        ));
    }

    let boundary = format!("----jimmy{}", nonce());
    let body = multipart(
        &boundary,
        path,
        &[("model", MODEL), ("response_format", "json")],
    )?;

    let agent = ureq::AgentBuilder::new()
        .timeout_read(Duration::from_secs(120))
        .timeout_write(Duration::from_secs(120))
        .build();

    let response = agent
        .post(URL)
        .set("Authorization", &format!("Bearer {api_key}"))
        .set(
            "Content-Type",
            &format!("multipart/form-data; boundary={boundary}"),
        )
        .send_bytes(&body)
        .map_err(error_message)?;

    let raw = response.into_string().map_err(|e| e.to_string())?;
    let parsed: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("groq: respuesta ilegible: {e}"))?;
    let text = parsed
        .get("text")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim();

    if text.is_empty() {
        return Err("no se entendió nada".into());
    }

    Ok(text.to_string())
}

fn error_message(error: ureq::Error) -> String {
    match error {
        ureq::Error::Status(code, response) => {
            let retry_after = response
                .header("retry-after")
                .and_then(|value| value.parse::<u64>().ok());
            let body = response.into_string().unwrap_or_default();
            status_message(code, retry_after, &body)
        }
        other => format!("groq: {other}"),
    }
}

fn status_message(code: u16, retry_after: Option<u64>, body: &str) -> String {
    if code == 429 {
        return match retry_after {
            Some(seconds) => format!("me pasé del límite de Groq, esperá {seconds}s"),
            None => "me pasé del límite de Groq, esperá un rato".into(),
        };
    }

    let detail = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|json| {
            json.pointer("/error/message")
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.trim().to_string());

    if detail.is_empty() {
        format!("groq http {code}")
    } else {
        format!("groq http {code}: {detail}")
    }
}

fn nonce() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn multipart(boundary: &str, path: &Path, fields: &[(&str, &str)]) -> Result<Vec<u8>, String> {
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("audio.ogg");
    let audio = std::fs::read(path).map_err(|e| format!("no pude leer el audio: {e}"))?;

    let mut body = Vec::with_capacity(audio.len() + 512);
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(&audio);
    body.extend_from_slice(b"\r\n");
    for (name, value) in fields {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str, data: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, data).unwrap();
        path
    }

    #[test]
    fn multipart_sends_the_file_and_the_fields() {
        let path = temp("jimmy-test-audio.ogg", b"OggS-data");
        let body = multipart("XYZ", &path, &[("model", "whisper-large-v3")]).unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(text.starts_with("--XYZ\r\n"));
        assert!(text.contains("filename=\"jimmy-test-audio.ogg\""));
        assert!(text.contains("OggS-data\r\n"));
        assert!(text.contains("name=\"model\"\r\n\r\nwhisper-large-v3\r\n"));
        assert!(text.ends_with("--XYZ--\r\n"));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn transcribe_rejects_audio_over_the_limit() {
        let path = temp("jimmy-test-audio-long.ogg", b"x");
        let error = transcribe("key", &path, MAX_SECONDS + 1).unwrap_err();
        assert!(error.contains("máximo es 300s"));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn rate_limit_message_tells_how_long_to_wait() {
        assert_eq!(
            status_message(429, Some(42), ""),
            "me pasé del límite de Groq, esperá 42s"
        );
        assert_eq!(
            status_message(429, None, ""),
            "me pasé del límite de Groq, esperá un rato"
        );
    }

    #[test]
    fn other_errors_keep_the_groq_detail() {
        let body = r#"{"error":{"message":"file must be one of the following types"}}"#;
        assert_eq!(
            status_message(400, None, body),
            "groq http 400: file must be one of the following types"
        );
    }
}
