//! El flujo OAuth 2.0 de authorization code, con los proveedores declarados
//! como datos en vez de como código.
//!
//! El núcleo no sabe de Notion ni de Google: recibe una `Provider` con sus
//! endpoints, sus scopes y cómo se autentica el cliente, corre el baile y deja
//! los tokens en el vault, un `project` por proveedor y un `env` por cuenta.

use heimdall::store::Store;
use serde_json::Value;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PENDING_TTL: Duration = Duration::from_secs(15 * 60);

pub struct Provider {
    pub name: &'static str,
    pub authorize: &'static str,
    pub token: &'static str,
    pub scopes: &'static [&'static str],
    pub extra: &'static [(&'static str, &'static str)],
    pub basic: bool,
    pub headers: &'static [(&'static str, &'static str)],
}

pub const PROVIDERS: &[Provider] = &[Provider {
    name: "notion",
    authorize: "https://api.notion.com/v1/oauth/authorize",
    token: "https://api.notion.com/v1/oauth/token",
    scopes: &[],
    extra: &[("owner", "user")],
    basic: true,
    headers: &[("Notion-Version", "2026-03-11")],
}];

pub fn provider(name: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|provider| provider.name == name)
}

pub struct Tokens {
    pub access: String,
    pub refresh: Option<String>,
    pub expires_at: i64,
    pub metadata: String,
}

struct Pending {
    provider: String,
    account: String,
    at: Instant,
}

static PENDING: OnceLock<Mutex<Vec<(String, Pending)>>> = OnceLock::new();

fn pending() -> &'static Mutex<Vec<(String, Pending)>> {
    PENDING.get_or_init(|| Mutex::new(Vec::new()))
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default()
}

fn variable(provider: &str, suffix: &str) -> String {
    format!("{}_{}", provider.to_ascii_uppercase(), suffix)
}

fn credentials(provider: &Provider) -> Result<(String, String), String> {
    let id = crate::env(&variable(provider.name, "CLIENT_ID"))
        .ok_or_else(|| format!("falta {}", variable(provider.name, "CLIENT_ID")))?;
    let secret = crate::env(&variable(provider.name, "CLIENT_SECRET"))
        .ok_or_else(|| format!("falta {}", variable(provider.name, "CLIENT_SECRET")))?;
    Ok((id, secret))
}

fn valid_account(account: &str) -> Result<(), String> {
    let valid = !account.is_empty()
        && account.len() <= 64
        && account
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if valid {
        return Ok(());
    }
    Err(format!(
        "«{account}» sólo acepta minúsculas, números, guiones y guiones bajos"
    ))
}

fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn form(pairs: &[(String, String)]) -> String {
    pairs
        .iter()
        .map(|(name, value)| format!("{}={}", encode(name), encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

pub fn start(provider_name: &str, account: &str, redirect: &str) -> Result<String, String> {
    let provider = provider(provider_name)
        .ok_or_else(|| format!("no conozco el proveedor {provider_name}"))?;
    let (client_id, _) = credentials(provider)?;
    valid_account(account)?;
    let state = crate::random::hex(24);
    {
        let mut pending = pending().lock().unwrap_or_else(|error| error.into_inner());
        pending.retain(|(_, entry)| entry.at.elapsed() < PENDING_TTL);
        pending.push((
            state.clone(),
            Pending {
                provider: provider.name.to_string(),
                account: account.to_string(),
                at: Instant::now(),
            },
        ));
    }
    let mut pairs = vec![
        ("client_id".to_string(), client_id),
        ("redirect_uri".to_string(), redirect.to_string()),
        ("response_type".to_string(), "code".to_string()),
        ("state".to_string(), state),
    ];
    if !provider.scopes.is_empty() {
        pairs.push(("scope".to_string(), provider.scopes.join(" ")));
    }
    for (name, value) in provider.extra {
        pairs.push((name.to_string(), value.to_string()));
    }
    Ok(format!("{}?{}", provider.authorize, form(&pairs)))
}

pub fn callback(code: &str, state: &str, redirect: &str) -> Result<(String, String), String> {
    let (provider_name, account) = take_pending(state)?;
    let provider = provider(&provider_name)
        .ok_or_else(|| format!("no conozco el proveedor {provider_name}"))?;
    let (client_id, client_secret) = credentials(provider)?;
    let tokens = request_token(
        provider,
        &[
            ("grant_type".to_string(), "authorization_code".to_string()),
            ("code".to_string(), code.to_string()),
            ("redirect_uri".to_string(), redirect.to_string()),
        ],
        &client_id,
        &client_secret,
    )?;
    let Some(held) = crate::vault::store() else {
        return Err("el vault no arrancó".to_string());
    };
    let store = held.lock().unwrap_or_else(|error| error.into_inner());
    save(&store, provider.name, &account, &tokens)?;
    Ok((provider.name.to_string(), account))
}

fn take_pending(state: &str) -> Result<(String, String), String> {
    let mut pending = pending().lock().unwrap_or_else(|error| error.into_inner());
    pending.retain(|(_, entry)| entry.at.elapsed() < PENDING_TTL);
    let index = pending
        .iter()
        .position(|(candidate, _)| candidate == state)
        .ok_or("el state no es válido o ya venció")?;
    let entry = pending.remove(index).1;
    Ok((entry.provider, entry.account))
}

fn request_token(
    provider: &Provider,
    params: &[(String, String)],
    client_id: &str,
    client_secret: &str,
) -> Result<Tokens, String> {
    let mut pairs = params.to_vec();
    if !provider.basic {
        pairs.push(("client_id".to_string(), client_id.to_string()));
        pairs.push(("client_secret".to_string(), client_secret.to_string()));
    }
    let body = form(&pairs);
    let http = ureq::AgentBuilder::new()
        .timeout_read(Duration::from_secs(20))
        .timeout_write(Duration::from_secs(20))
        .timeout_connect(Duration::from_secs(10))
        .build();
    let mut request = http
        .post(provider.token)
        .set("Content-Type", "application/x-www-form-urlencoded")
        .set("Accept", "application/json")
        .set(
            "User-Agent",
            "jimmy (+https://github.com/3-lines-studio/jimmy)",
        );
    if provider.basic {
        request = request.set(
            "Authorization",
            &format!("Basic {}", basic(&format!("{client_id}:{client_secret}"))),
        );
    }
    for (name, value) in provider.headers {
        request = request.set(name, value);
    }
    let response = match request.send_string(&body) {
        Ok(response) => response,
        Err(ureq::Error::Status(code, response)) => {
            let text = response.into_string().unwrap_or_default();
            return Err(format!("{} http {code}: {text}", provider.name));
        }
        Err(error) => return Err(format!("{}: {error}", provider.name)),
    };
    let text = response
        .into_string()
        .map_err(|error| format!("{}: {error}", provider.name))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|error| format!("{} no contestó JSON: {error}", provider.name))?;
    tokens(&value)
}

fn tokens(value: &Value) -> Result<Tokens, String> {
    let access = value
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or("la respuesta no trae access_token")?
        .to_string();
    let refresh = value
        .get("refresh_token")
        .and_then(Value::as_str)
        .map(String::from);
    let expires_at = value
        .get("expires_in")
        .and_then(Value::as_i64)
        .map(|seconds| now() + seconds)
        .unwrap_or_default();
    let mut object = value.as_object().cloned().unwrap_or_default();
    for key in ["access_token", "refresh_token", "expires_in", "token_type"] {
        object.remove(key);
    }
    Ok(Tokens {
        access,
        refresh,
        expires_at,
        metadata: Value::Object(object).to_string(),
    })
}

pub fn save(store: &Store, provider: &str, account: &str, tokens: &Tokens) -> Result<(), String> {
    const ACTOR: &str = "oauth";
    store
        .create_environment(provider, account, ACTOR)
        .map_err(crate::vault::text)?;
    let mut pairs = vec![
        ("OAUTH_ACCESS_TOKEN", tokens.access.clone()),
        ("OAUTH_EXPIRES_AT", tokens.expires_at.to_string()),
        ("OAUTH_METADATA", tokens.metadata.clone()),
    ];
    if let Some(refresh) = &tokens.refresh {
        pairs.push(("OAUTH_REFRESH_TOKEN", refresh.clone()));
    }
    for (name, value) in pairs {
        store
            .set(provider, account, name, &value, ACTOR)
            .map_err(crate::vault::text)?;
    }
    Ok(())
}

fn basic(input: &str) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0] as u32;
        let second = chunk.get(1).copied().unwrap_or(0) as u32;
        let third = chunk.get(2).copied().unwrap_or(0) as u32;
        let combined = (first << 16) | (second << 8) | third;
        out.push(ALPHABET[(combined >> 18) as usize & 63] as char);
        out.push(ALPHABET[(combined >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[(combined >> 6) as usize & 63] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[combined as usize & 63] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    static ENV: Mutex<()> = Mutex::new(());

    fn fake_token(reply: &'static str) -> (String, std::sync::mpsc::Receiver<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/token", listener.local_addr().unwrap());
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            while let Ok(read) = stream.read(&mut buffer) {
                request.extend_from_slice(&buffer[..read]);
                let text = String::from_utf8_lossy(&request);
                if text.contains("\r\n\r\n") && has_full_body(&text) {
                    break;
                }
                if read == 0 {
                    break;
                }
            }
            let body = reply.as_bytes();
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(body);
            let _ = sender.send(String::from_utf8_lossy(&request).to_string());
        });
        (url, receiver)
    }

    fn has_full_body(request: &str) -> bool {
        let Some(end) = request.find("\r\n\r\n") else {
            return false;
        };
        let length: usize = request
            .lines()
            .find(|line| line.to_ascii_lowercase().starts_with("content-length:"))
            .and_then(|line| line.split(':').nth(1))
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or(0);
        request.len() >= end + 4 + length
    }

    fn local(name: &'static str, token: String) -> Provider {
        Provider {
            name,
            authorize: "https://example.test/authorize",
            token: Box::leak(token.into_boxed_str()),
            scopes: &["read", "write"],
            extra: &[("owner", "user")],
            basic: true,
            headers: &[],
        }
    }

    #[test]
    fn encode_escapes_everything_but_the_unreserved() {
        assert_eq!(encode("a b"), "a%20b");
        assert_eq!(encode("a/b:c"), "a%2Fb%3Ac");
        assert_eq!(encode("abc-_.~"), "abc-_.~");
    }

    #[test]
    fn basic_pads_and_encodes() {
        assert_eq!(basic("user:pass"), "dXNlcjpwYXNz");
        assert_eq!(basic("a"), "YQ==");
        assert_eq!(basic("ab"), "YWI=");
        assert_eq!(basic("abc"), "YWJj");
    }

    #[test]
    fn the_authorize_url_carries_the_state_the_scopes_and_the_extra() {
        let _env = ENV.lock().unwrap();
        std::env::set_var("NOTION_CLIENT_ID", "abc");
        std::env::set_var("NOTION_CLIENT_SECRET", "shh");

        let url = start("notion", "manu", "https://jimmy.test/oauth/notion/callback").unwrap();

        assert!(
            url.starts_with("https://api.notion.com/v1/oauth/authorize?"),
            "{url}"
        );
        assert!(url.contains("client_id=abc"), "{url}");
        assert!(url.contains("response_type=code"), "{url}");
        assert!(url.contains("owner=user"), "{url}");
        assert!(
            url.contains("redirect_uri=https%3A%2F%2Fjimmy.test%2Foauth%2Fnotion%2Fcallback"),
            "{url}"
        );

        let state = url
            .split("state=")
            .nth(1)
            .and_then(|rest| rest.split('&').next())
            .unwrap();
        let (name, account) = take_pending(state).unwrap();
        assert_eq!(name, "notion");
        assert_eq!(account, "manu");
        assert!(take_pending(state).is_err());

        std::env::remove_var("NOTION_CLIENT_ID");
        std::env::remove_var("NOTION_CLIENT_SECRET");
    }

    #[test]
    fn an_account_with_letters_outside_the_slug_is_rejected() {
        let _env = ENV.lock().unwrap();
        std::env::set_var("NOTION_CLIENT_ID", "abc");
        std::env::set_var("NOTION_CLIENT_SECRET", "shh");

        assert!(start("notion", "Manu Ponsa", "https://jimmy.test/cb").is_err());

        std::env::remove_var("NOTION_CLIENT_ID");
        std::env::remove_var("NOTION_CLIENT_SECRET");
    }

    #[test]
    fn the_token_request_uses_basic_auth_and_keeps_the_metadata() {
        let (url, received) = fake_token(
            r#"{"access_token":"tok","refresh_token":"ref","workspace_name":"3lines","bot_id":"b1"}"#,
        );
        let provider = local("local", url);

        let tokens = request_token(
            &provider,
            &[
                ("grant_type".to_string(), "authorization_code".to_string()),
                ("code".to_string(), "abc".to_string()),
            ],
            "id",
            "secret",
        )
        .unwrap();

        assert_eq!(tokens.access, "tok");
        assert_eq!(tokens.refresh.as_deref(), Some("ref"));
        assert!(tokens.metadata.contains("3lines"), "{}", tokens.metadata);
        assert!(!tokens.metadata.contains("tok"), "{}", tokens.metadata);

        let request = received.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(
            request.contains("Authorization: Basic aWQ6c2VjcmV0"),
            "{request}"
        );
        assert!(
            request.contains("grant_type=authorization_code"),
            "{request}"
        );
        assert!(request.contains("code=abc"), "{request}");
        assert!(!request.contains("client_secret="), "{request}");
    }

    #[test]
    fn a_body_authenticated_provider_sends_the_secret_in_the_form() {
        let (url, received) = fake_token(r#"{"access_token":"tok"}"#);
        let mut provider = local("local", url);
        provider.basic = false;

        request_token(&provider, &[], "id", "secret").unwrap();

        let request = received.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(!request.contains("Authorization: Basic"), "{request}");
        assert!(request.contains("client_id=id"), "{request}");
        assert!(request.contains("client_secret=secret"), "{request}");
    }

    #[test]
    fn a_response_without_access_token_is_an_error() {
        let value: Value = serde_json::from_str(r#"{"error":"invalid_grant"}"#).unwrap();
        assert!(tokens(&value).is_err());
    }
}
