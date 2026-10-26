//! OAuth 2.1 para servidores MCP: discovery, registro dinámico del cliente y
//! PKCE. Los endpoints no se hardcodean: salen del challenge `WWW-Authenticate`
//! del servidor, de su metadata de recurso protegido (RFC 9728) y de la del
//! servidor de autorización (RFC 8414).

use serde_json::Value;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

const PENDING_TTL: Duration = Duration::from_secs(15 * 60);
const USER_AGENT: &str = "jimmy (+https://github.com/3-lines-studio/jimmy)";

#[derive(Clone, Debug)]
pub struct Metadata {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: Option<String>,
    pub methods: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Registration {
    pub client_id: String,
    pub client_secret: Option<String>,
}

pub struct Tokens {
    pub access: String,
    pub refresh: Option<String>,
    pub expires_at: i64,
}

pub struct Discovery {
    pub resource: String,
    pub scopes: Vec<String>,
    pub metadata: Metadata,
}

pub fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_read(Duration::from_secs(20))
        .timeout_write(Duration::from_secs(20))
        .timeout_connect(Duration::from_secs(10))
        .user_agent(USER_AGENT)
        .build()
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default()
}

/// Descubre el recurso protegido y su servidor de autorización. Primero golpea
/// el servidor MCP sin credenciales: un 401 con `resource_metadata` es la vía
/// que manda la spec. Si no hay challenge, se cae al well-known del origen.
pub fn discover(server_url: &str) -> Result<Discovery, String> {
    let http = agent();
    let challenge = match http
        .post(server_url)
        .set("Content-Type", "application/json")
        .set("Accept", "application/json, text/event-stream")
        .send_string(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#)
    {
        Ok(_) => None,
        Err(ureq::Error::Status(_, response)) => response
            .header("www-authenticate")
            .map(String::from)
            .or_else(|| Some(String::new())),
        Err(error) => return Err(format!("{server_url}: {error}")),
    };

    let parsed = challenge
        .as_deref()
        .map(parse_challenge)
        .unwrap_or_default();
    let metadata_url = parsed
        .resource_metadata
        .clone()
        .unwrap_or_else(|| well_known(server_url, "oauth-protected-resource"));
    let resource_doc = get_json(&metadata_url)?;
    let resource = string(&resource_doc, "resource").unwrap_or_else(|| server_url.to_string());
    let issuer = resource_doc
        .get("authorization_servers")
        .and_then(Value::as_array)
        .and_then(|list| list.first())
        .and_then(Value::as_str)
        .map(String::from)
        .unwrap_or_else(|| origin(server_url));
    let metadata = metadata(&issuer)?;
    let mut scopes: Vec<String> = parsed.scope.into_iter().collect();
    for scope in strings(&resource_doc, "scopes_supported") {
        if !scopes.contains(&scope) {
            scopes.push(scope);
        }
    }
    Ok(Discovery {
        resource,
        scopes,
        metadata,
    })
}

/// La metadata del servidor de autorización (RFC 8414). El issuer puede traer
/// path, así que el well-known se inserta después del host, como manda la RFC.
pub fn metadata(issuer: &str) -> Result<Metadata, String> {
    let url = format!(
        "{}/.well-known/oauth-authorization-server{}",
        origin(issuer),
        path(issuer)
    );
    let doc = get_json(&url)?;
    let authorization_endpoint = string(&doc, "authorization_endpoint")
        .ok_or("la metadata no trae authorization_endpoint")?;
    let token_endpoint =
        string(&doc, "token_endpoint").ok_or("la metadata no trae token_endpoint")?;
    Ok(Metadata {
        issuer: string(&doc, "issuer").unwrap_or_else(|| issuer.to_string()),
        authorization_endpoint,
        token_endpoint,
        registration_endpoint: string(&doc, "registration_endpoint"),
        methods: strings(&doc, "token_endpoint_auth_methods_supported"),
    })
}

/// Registra el cliente en el servidor de autorización (RFC 7591). Sin esto,
/// conectar un servidor nuevo pediría una app creada a mano.
pub fn register(metadata: &Metadata, redirect: &str, name: &str) -> Result<Registration, String> {
    let endpoint = metadata
        .registration_endpoint
        .as_ref()
        .ok_or("este servidor no acepta registro dinámico")?;
    let method = method(metadata);
    let body = serde_json::json!({
        "client_name": name,
        "redirect_uris": [redirect],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": method,
    });
    let http = agent();
    let response = http
        .post(endpoint)
        .set("Content-Type", "application/json")
        .set("Accept", "application/json")
        .send_string(&body.to_string());
    let doc = match response {
        Ok(response) => response
            .into_string()
            .map_err(|error| format!("registro: {error}"))?,
        Err(ureq::Error::Status(code, response)) => {
            let text = response.into_string().unwrap_or_default();
            return Err(format!("registro http {code}: {text}"));
        }
        Err(error) => return Err(format!("registro: {error}")),
    };
    let doc: Value = serde_json::from_str(&doc)
        .map_err(|error| format!("registro no contestó JSON: {error}"))?;
    let client_id = string(&doc, "client_id").ok_or("el registro no trajo client_id")?;
    let client_secret = if method == "none" {
        None
    } else {
        string(&doc, "client_secret")
    };
    Ok(Registration {
        client_id,
        client_secret,
    })
}

/// `token_endpoint_auth_method` para el registro: público si el servidor acepta
/// `none`, que es lo que corresponde a un cliente sin secreto compartido.
fn method(metadata: &Metadata) -> &'static str {
    if metadata.methods.is_empty() || metadata.methods.iter().any(|m| m == "none") {
        return "none";
    }
    if metadata.methods.iter().any(|m| m == "client_secret_basic") {
        return "client_secret_basic";
    }
    "client_secret_post"
}

pub fn authorize_url(
    metadata: &Metadata,
    registration: &Registration,
    redirect: &str,
    state: &str,
    challenge: &str,
    scopes: &[String],
    resource: &str,
) -> String {
    let mut pairs = vec![
        ("response_type", "code".to_string()),
        ("client_id", registration.client_id.clone()),
        ("redirect_uri", redirect.to_string()),
        ("state", state.to_string()),
        ("code_challenge", challenge.to_string()),
        ("code_challenge_method", "S256".to_string()),
        ("resource", resource.to_string()),
    ];
    if !scopes.is_empty() {
        pairs.push(("scope", scopes.join(" ")));
    }
    format!(
        "{}?{}",
        metadata.authorization_endpoint,
        form(
            &pairs
                .iter()
                .map(|(k, v)| (*k, v.as_str()))
                .collect::<Vec<_>>()
        )
    )
}

pub fn exchange(
    metadata: &Metadata,
    registration: &Registration,
    code: &str,
    verifier: &str,
    redirect: &str,
    resource: &str,
) -> Result<Tokens, String> {
    let mut pairs = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect),
        ("code_verifier", verifier),
        ("resource", resource),
        ("client_id", registration.client_id.as_str()),
    ];
    if let Some(secret) = &registration.client_secret {
        pairs.push(("client_secret", secret.as_str()));
    }
    token_request(&metadata.token_endpoint, &pairs)
}

pub fn refresh(
    metadata: &Metadata,
    registration: &Registration,
    refresh_token: &str,
    resource: &str,
) -> Result<Tokens, String> {
    let mut pairs = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("resource", resource),
        ("client_id", registration.client_id.as_str()),
    ];
    if let Some(secret) = &registration.client_secret {
        pairs.push(("client_secret", secret.as_str()));
    }
    token_request(&metadata.token_endpoint, &pairs)
}

fn token_request(endpoint: &str, pairs: &[(&str, &str)]) -> Result<Tokens, String> {
    let http = agent();
    let response = http
        .post(endpoint)
        .set("Content-Type", "application/x-www-form-urlencoded")
        .set("Accept", "application/json")
        .send_string(&form(pairs));
    let text = match response {
        Ok(response) => response.into_string().unwrap_or_default(),
        Err(ureq::Error::Status(code, response)) => {
            let text = response.into_string().unwrap_or_default();
            return Err(format!("token http {code}: {text}"));
        }
        Err(error) => return Err(format!("token: {error}")),
    };
    let doc: Value =
        serde_json::from_str(&text).map_err(|error| format!("token no contestó JSON: {error}"))?;
    let access = string(&doc, "access_token").ok_or("la respuesta no trae access_token")?;
    let expires_at = doc
        .get("expires_in")
        .and_then(Value::as_i64)
        .map(|seconds| now() + seconds)
        .unwrap_or_default();
    Ok(Tokens {
        access,
        refresh: string(&doc, "refresh_token"),
        expires_at,
    })
}

#[derive(Default)]
struct Challenge {
    resource_metadata: Option<String>,
    scope: Option<String>,
}

/// Parsea `Bearer resource_metadata="..."`, `realm="OAuth", scope="..."` y
/// cualquier combinación: el scheme puede venir pegado al primer parámetro.
fn parse_challenge(header: &str) -> Challenge {
    let mut challenge = Challenge::default();
    for part in header.split(',') {
        let Some((name, value)) = part.trim().split_once('=') else {
            continue;
        };
        let name = name.split_whitespace().last().unwrap_or("");
        let value = value.trim().trim_matches('"').to_string();
        match name.to_ascii_lowercase().as_str() {
            "resource_metadata" if !value.is_empty() => challenge.resource_metadata = Some(value),
            "scope" if !value.is_empty() => challenge.scope = Some(value),
            _ => {}
        }
    }
    challenge
}

fn get_json(url: &str) -> Result<Value, String> {
    let http = agent();
    let response = http.get(url).set("Accept", "application/json").call();
    let text = match response {
        Ok(response) => response.into_string().unwrap_or_default(),
        Err(ureq::Error::Status(code, response)) => {
            let text = response.into_string().unwrap_or_default();
            return Err(format!("{url} http {code}: {text}"));
        }
        Err(error) => return Err(format!("{url}: {error}")),
    };
    serde_json::from_str(&text).map_err(|error| format!("{url} no contestó JSON: {error}"))
}

fn well_known(url: &str, name: &str) -> String {
    format!("{}/.well-known/{}{}", origin(url), name, path(url))
}

fn origin(url: &str) -> String {
    match url.split_once("://") {
        Some((scheme, rest)) => {
            let host = rest.split(['/', '?', '#']).next().unwrap_or("");
            format!("{scheme}://{host}")
        }
        None => url.to_string(),
    }
}

fn path(url: &str) -> String {
    match url.split_once("://") {
        Some((_, rest)) => match rest.find(['/', '?', '#']) {
            Some(index) => {
                let path = &rest[index..];
                let path = path.split(['?', '#']).next().unwrap_or("");
                path.trim_end_matches('/').to_string()
            }
            None => String::new(),
        },
        None => String::new(),
    }
}

fn string(doc: &Value, key: &str) -> Option<String> {
    doc.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(String::from)
}

fn strings(doc: &Value, key: &str) -> Vec<String> {
    doc.get(key)
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

pub fn form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(name, value)| format!("{}={}", encode(name), encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

pub fn encode(value: &str) -> String {
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

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0] as u32;
        let second = chunk.get(1).copied().unwrap_or(0) as u32;
        let third = chunk.get(2).copied().unwrap_or(0) as u32;
        let combined = (first << 16) | (second << 8) | third;
        out.push(ALPHABET[(combined >> 18) as usize & 63] as char);
        out.push(ALPHABET[(combined >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(combined >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[combined as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

pub fn base64url(bytes: &[u8]) -> String {
    base64(bytes)
        .trim_end_matches('=')
        .replace('+', "-")
        .replace('/', "_")
}

pub fn challenge(verifier: &str) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, verifier.as_bytes());
    base64url(digest.as_ref())
}

struct Pending {
    server: String,
    account: String,
    verifier: String,
    metadata: Metadata,
    registration: Registration,
    resource: String,
    at: Instant,
}

static PENDING: OnceLock<Mutex<Vec<(String, Pending)>>> = OnceLock::new();

fn pending() -> &'static Mutex<Vec<(String, Pending)>> {
    PENDING.get_or_init(|| Mutex::new(Vec::new()))
}

pub struct Authorized {
    pub server: String,
    pub account: String,
    pub tokens: Tokens,
}

/// Guarda el `state` y su verifier mientras el usuario autoriza.
pub fn remember(
    state: &str,
    server: &str,
    account: &str,
    verifier: &str,
    metadata: Metadata,
    registration: Registration,
    resource: &str,
) {
    let mut pending = pending().lock().unwrap_or_else(|error| error.into_inner());
    pending.retain(|(_, entry)| entry.at.elapsed() < PENDING_TTL);
    pending.push((
        state.to_string(),
        Pending {
            server: server.to_string(),
            account: account.to_string(),
            verifier: verifier.to_string(),
            metadata,
            registration,
            resource: resource.to_string(),
            at: Instant::now(),
        },
    ));
}

/// Cierra el baile: canjea el código con el verifier guardado.
pub fn complete(state: &str, code: &str, redirect: &str) -> Result<Authorized, String> {
    let entry = {
        let mut pending = pending().lock().unwrap_or_else(|error| error.into_inner());
        pending.retain(|(_, entry)| entry.at.elapsed() < PENDING_TTL);
        let index = pending
            .iter()
            .position(|(candidate, _)| candidate == state)
            .ok_or("el state no es válido o ya venció")?;
        pending.remove(index).1
    };
    let tokens = exchange(
        &entry.metadata,
        &entry.registration,
        code,
        &entry.verifier,
        redirect,
        &entry.resource,
    )?;
    Ok(Authorized {
        server: entry.server,
        account: entry.account,
        tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_challenge_yields_the_metadata_url_and_the_scope() {
        let challenge = parse_challenge(
            r#"Bearer realm="OAuth", resource_metadata="https://mcp.notion.com/.well-known/oauth-protected-resource/mcp", scope="default""#,
        );
        assert_eq!(
            challenge.resource_metadata.as_deref(),
            Some("https://mcp.notion.com/.well-known/oauth-protected-resource/mcp")
        );
        assert_eq!(challenge.scope.as_deref(), Some("default"));
    }

    #[test]
    fn a_challenge_without_a_comma_reads_the_scheme_off_the_name() {
        let challenge = parse_challenge(
            r#"Bearer resource_metadata="https://mcp.slack.com/.well-known/oauth-protected-resource""#,
        );
        assert_eq!(
            challenge.resource_metadata.as_deref(),
            Some("https://mcp.slack.com/.well-known/oauth-protected-resource")
        );
    }

    #[test]
    fn a_challenge_without_metadata_stays_empty() {
        let challenge = parse_challenge("Bearer realm=\"OAuth\"");
        assert!(challenge.resource_metadata.is_none());
        assert!(challenge.scope.is_none());
    }

    #[test]
    fn the_well_known_keeps_the_path_after_the_host() {
        assert_eq!(
            well_known("https://mcp.notion.com/mcp", "oauth-protected-resource"),
            "https://mcp.notion.com/.well-known/oauth-protected-resource/mcp"
        );
        assert_eq!(
            well_known("https://mcp.notion.com", "oauth-authorization-server"),
            "https://mcp.notion.com/.well-known/oauth-authorization-server"
        );
    }

    #[test]
    fn origin_and_path_split_the_url() {
        assert_eq!(
            origin("https://mcp.notion.com/mcp"),
            "https://mcp.notion.com"
        );
        assert_eq!(path("https://mcp.notion.com/mcp"), "/mcp");
        assert_eq!(path("https://mcp.notion.com"), "");
    }

    #[test]
    fn pkce_is_s256_of_the_verifier() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn base64url_drops_the_padding_and_the_unsafe_characters() {
        assert_eq!(base64url(b"a"), "YQ");
        assert_eq!(base64url(b"ab"), "YWI");
        assert_eq!(base64url(b"abc"), "YWJj");
    }

    #[test]
    fn the_authorize_url_carries_pkce_state_and_resource() {
        let metadata = Metadata {
            issuer: "https://mcp.notion.com".into(),
            authorization_endpoint: "https://mcp.notion.com/authorize".into(),
            token_endpoint: "https://mcp.notion.com/token".into(),
            registration_endpoint: Some("https://mcp.notion.com/register".into()),
            methods: vec!["none".into()],
        };
        let registration = Registration {
            client_id: "abc".into(),
            client_secret: None,
        };
        let url = authorize_url(
            &metadata,
            &registration,
            "https://jimmy.test/oauth/callback",
            "st4te",
            "ch4llenge",
            &["default".into()],
            "https://mcp.notion.com/mcp",
        );
        assert!(
            url.starts_with("https://mcp.notion.com/authorize?"),
            "{url}"
        );
        assert!(url.contains("code_challenge_method=S256"), "{url}");
        assert!(url.contains("code_challenge=ch4llenge"), "{url}");
        assert!(url.contains("state=st4te"), "{url}");
        assert!(
            url.contains("resource=https%3A%2F%2Fmcp.notion.com%2Fmcp"),
            "{url}"
        );
        assert!(url.contains("scope=default"), "{url}");
        assert!(!url.contains("client_secret"), "{url}");
    }

    #[test]
    fn a_public_registration_leaves_out_the_secret() {
        let metadata = Metadata {
            issuer: String::new(),
            authorization_endpoint: String::new(),
            token_endpoint: String::new(),
            registration_endpoint: None,
            methods: vec!["none".into(), "client_secret_basic".into()],
        };
        assert_eq!(method(&metadata), "none");

        let confidential = Metadata {
            methods: vec!["client_secret_post".into()],
            ..metadata
        };
        assert_eq!(method(&confidential), "client_secret_post");
    }

    #[test]
    fn encode_escapes_everything_the_form_needs() {
        assert_eq!(encode("a b"), "a%20b");
        assert_eq!(encode("a/b"), "a%2Fb");
    }
}
