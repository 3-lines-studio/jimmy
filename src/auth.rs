//! Quién puede entrar: una lista de mails en el entorno, y un link que se manda
//! una sola vez.
//!
//! No hay contraseñas: el mail autorizado es la identidad. La sesión sí queda
//! en disco, así un redeploy no echa a nadie.

use crate::mail::Mail;
use crate::random;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const COOKIE: &str = "jimmy_session";
pub const SESSION_TTL: u64 = 30 * 24 * 60 * 60;
const TTL: Duration = Duration::from_secs(15 * 60);
const COOLDOWN: Duration = Duration::from_secs(60);

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

struct Link {
    email: String,
    expires: Instant,
    sent: Instant,
}

pub struct Auth {
    allowed: Vec<String>,
    links: Mutex<HashMap<String, Link>>,
    sessions: Sessions,
    pub mail: Option<Mail>,
    pub dev: bool,
}

impl Auth {
    /// `allowed` es la lista de mails autorizados, separados por coma.
    pub fn new(allowed: &str, root: &Path, mail: Option<Mail>, dev: bool) -> Auth {
        let allowed = allowed
            .split(',')
            .map(|email| email.trim().to_lowercase())
            .filter(|email| !email.is_empty())
            .collect();
        Auth {
            allowed,
            links: Mutex::new(HashMap::new()),
            sessions: Sessions::load(root),
            mail,
            dev,
        }
    }

    pub fn allowed(&self) -> &[String] {
        &self.allowed
    }

    fn is_allowed(&self, email: &str) -> bool {
        let email = email.trim().to_lowercase();
        self.allowed.contains(&email)
    }

    /// Un link nuevo para ese mail, si está autorizado y no se pidió otro hace
    /// un segundo. Devolver el token no dice si el mail existe: eso lo decide
    /// quien llama, que siempre contesta lo mismo.
    pub fn request_link(&self, email: &str) -> Option<String> {
        let email = email.trim().to_lowercase();
        if !self.is_allowed(&email) {
            return None;
        }
        let now = Instant::now();
        let mut links = self.links.lock().unwrap();
        links.retain(|_, link| link.expires > now);
        if links
            .values()
            .any(|link| link.email == email && now.duration_since(link.sent) < COOLDOWN)
        {
            return None;
        }
        let token = random::hex(32);
        links.insert(
            token.clone(),
            Link {
                email,
                expires: now + TTL,
                sent: now,
            },
        );
        Some(token)
    }

    /// El token sirve una vez y solo dentro del plazo.
    pub fn consume_link(&self, token: &str) -> Option<String> {
        let mut links = self.links.lock().unwrap();
        let link = links.remove(token)?;
        (link.expires > Instant::now()).then_some(link.email)
    }

    pub fn open_session(&self, email: &str) -> String {
        self.sessions.open(email)
    }

    pub fn user(&self, token: &str) -> Option<String> {
        self.sessions.user(token)
    }

    pub fn close_session(&self, token: &str) {
        self.sessions.close(token);
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Session {
    email: String,
    expires: u64,
}

#[derive(Serialize, Deserialize, Default)]
struct Tokens(HashMap<String, Session>);

struct Sessions {
    path: PathBuf,
    tokens: Mutex<HashMap<String, Session>>,
}

impl Sessions {
    fn load(root: &Path) -> Sessions {
        let path = root.join("sessions.json");
        let tokens = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<Tokens>(&text).ok())
            .map(|tokens| tokens.0)
            .unwrap_or_default();
        let tokens = live(tokens);
        Sessions {
            path,
            tokens: Mutex::new(tokens),
        }
    }

    fn open(&self, email: &str) -> String {
        let token = random::hex(32);
        let session = Session {
            email: email.to_string(),
            expires: now() + SESSION_TTL,
        };
        let mut tokens = self.tokens.lock().unwrap();
        *tokens = live(std::mem::take(&mut *tokens));
        tokens.insert(token.clone(), session);
        self.save(&tokens);
        token
    }

    fn close(&self, token: &str) {
        let mut tokens = self.tokens.lock().unwrap();
        tokens.remove(token);
        self.save(&tokens);
    }

    fn user(&self, token: &str) -> Option<String> {
        let tokens = self.tokens.lock().unwrap();
        let session = tokens.get(token)?;
        (session.expires > now()).then(|| session.email.clone())
    }

    fn save(&self, tokens: &HashMap<String, Session>) {
        let text = serde_json::to_string_pretty(&Tokens(tokens.clone())).unwrap_or_default();
        let _ = axe::atomic_write(&self.path, text.as_bytes());
    }
}

fn live(tokens: HashMap<String, Session>) -> HashMap<String, Session> {
    let now = now();
    tokens
        .into_iter()
        .filter(|(_, session)| session.expires > now)
        .collect()
}

#[test]
fn a_session_that_expired_does_not_let_anyone_in() {
    let root = std::env::temp_dir().join(format!("jimmy-auth-{}-expired", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let expires = now();
    let old = expires.saturating_sub(1);
    let fresh = expires + 60;
    std::fs::write(
        root.join("sessions.json"),
        format!(
            r#"{{"vieja":{{"email":"berti@ejemplo.com","expires":{old}}},
                 "nueva":{{"email":"berti@ejemplo.com","expires":{fresh}}}}}"#
        ),
    )
    .unwrap();

    let auth = Auth::new("berti@ejemplo.com", &root, None, false);
    assert!(auth.user("vieja").is_none(), "la vieja ya venció");
    assert_eq!(auth.user("nueva").as_deref(), Some("berti@ejemplo.com"));

    let opened = auth.open_session("berti@ejemplo.com");
    assert!(auth.user(&opened).is_some());
    let saved = std::fs::read_to_string(root.join("sessions.json")).unwrap();
    assert!(
        !saved.contains("vieja"),
        "al abrir una sesión se limpian las vencidas"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auth(tag: &str, emails: &str) -> (Auth, PathBuf) {
        let root = std::env::temp_dir().join(format!("jimmy-auth-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let auth = Auth::new(emails, &root, None, true);
        (auth, root)
    }

    #[test]
    fn only_the_listed_mails_get_a_link() {
        let (auth, root) = auth("links", "Berti@Ejemplo.com, ana@ejemplo.com");
        assert!(
            auth.request_link("berti@ejemplo.com").is_some(),
            "sin importar mayúsculas"
        );
        assert!(auth.request_link("otro@ejemplo.com").is_none());
        assert!(auth.request_link("").is_none());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_link_serves_once_and_the_session_survives() {
        let (auth, root) = auth("once", "berti@ejemplo.com");
        let token = auth.request_link("berti@ejemplo.com").unwrap();
        assert_eq!(
            auth.consume_link(&token).as_deref(),
            Some("berti@ejemplo.com")
        );
        assert_eq!(auth.consume_link(&token), None, "no sirve dos veces");

        let session = auth.open_session("berti@ejemplo.com");
        assert_eq!(auth.user(&session).as_deref(), Some("berti@ejemplo.com"));
        let reloaded = Sessions::load(&root);
        assert_eq!(
            reloaded.user(&session).as_deref(),
            Some("berti@ejemplo.com")
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn asking_twice_in_a_row_does_not_send_twice() {
        let (auth, root) = auth("cooldown", "berti@ejemplo.com");
        assert!(auth.request_link("berti@ejemplo.com").is_some());
        assert!(
            auth.request_link("berti@ejemplo.com").is_none(),
            "muy seguido"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
