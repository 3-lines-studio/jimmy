//! Quién puede entrar: una lista de mails en el entorno, y un link que se manda
//! una sola vez.
//!
//! No hay contraseñas: el mail autorizado es la identidad. Los usuarios y las
//! sesiones viven en la base del control plane, así un redeploy no echa a nadie
//! y una sesión vale en cualquier instancia.

use crate::mail::Mail;
use crate::random;
use crate::store::Store;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
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
    store: Arc<Store>,
    pub mail: Option<Mail>,
    pub dev: bool,
}

impl Auth {
    /// `allowed` es la lista de mails autorizados, separados por coma. La base
    /// del control plane la abre quien arma todo esto, que es el que sabe dónde
    /// vive.
    pub fn new(store: Arc<Store>, allowed: &str, mail: Option<Mail>, dev: bool) -> Auth {
        let allowed = allowed
            .split(',')
            .map(|email| email.trim().to_lowercase())
            .filter(|email| !email.is_empty())
            .collect();
        Auth {
            allowed,
            links: Mutex::new(HashMap::new()),
            store,
            mail,
            dev,
        }
    }

    /// Quién es el dueño de la sesión, con su id, si sigue viva.
    pub fn session_user(&self, token: &str) -> Option<crate::store::User> {
        self.store.session_user(token).ok().flatten()
    }

    /// El control plane, para lo que no es entrar: orgs, conversaciones, agenda.
    pub fn store(&self) -> &Arc<Store> {
        &self.store
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

    /// La sesión del que entra: la primera vez de ese mail también le crea el
    /// usuario y su org personal.
    pub fn open_session(&self, email: &str) -> Option<String> {
        let (user, _) = self.store.register(email).ok()?;
        let token = random::hex(32);
        let expires = (now() + SESSION_TTL) as i64;
        self.store.open_session(&token, &user.id, expires).ok()?;
        Some(token)
    }

    pub fn user(&self, token: &str) -> Option<String> {
        self.store
            .session_user(token)
            .ok()
            .flatten()
            .map(|user| user.email)
    }

    pub fn close_session(&self, token: &str) {
        let _ = self.store.close_session(token);
        let _ = self.store.forget_expired_sessions();
    }
}

/// Un auth sobre una base temporal, que es de lo único que depende.
#[cfg(test)]
fn test_auth(root: &std::path::Path, emails: &str) -> Auth {
    Auth::new(
        Arc::new(Store::open(&root.join("jimmy.db")).unwrap()),
        emails,
        None,
        true,
    )
}

#[test]
fn una_sesion_vencida_no_deja_entrar_a_nadie() {
    let root = std::env::temp_dir().join(format!("jimmy-auth-{}-expired", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let auth = test_auth(&root, "bob@ejemplo.com");
    let (user, _) = auth.store().register("bob@ejemplo.com").unwrap();
    auth.store()
        .open_session("vieja", &user.id, (now() - 1) as i64)
        .unwrap();
    assert!(auth.user("vieja").is_none(), "la vieja ya venció");

    let nueva = auth.open_session("bob@ejemplo.com").unwrap();
    assert_eq!(auth.user(&nueva).as_deref(), Some("bob@ejemplo.com"));
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auth(tag: &str, emails: &str) -> (Auth, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("jimmy-auth-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let auth = test_auth(&root, emails);
        (auth, root)
    }

    #[test]
    fn only_the_listed_mails_get_a_link() {
        let (auth, root) = auth("links", "Bob@Ejemplo.com, ana@ejemplo.com");
        assert!(
            auth.request_link("bob@ejemplo.com").is_some(),
            "sin importar mayúsculas"
        );
        assert!(auth.request_link("otro@ejemplo.com").is_none());
        assert!(auth.request_link("").is_none());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_link_serves_once_and_the_session_survives() {
        let (auth, root) = auth("once", "bob@ejemplo.com");
        let token = auth.request_link("bob@ejemplo.com").unwrap();
        assert_eq!(
            auth.consume_link(&token).as_deref(),
            Some("bob@ejemplo.com")
        );
        assert_eq!(auth.consume_link(&token), None, "no sirve dos veces");

        let session = auth.open_session("bob@ejemplo.com").unwrap();
        assert_eq!(auth.user(&session).as_deref(), Some("bob@ejemplo.com"));
        let reopened = test_auth(&root, "bob@ejemplo.com");
        assert_eq!(
            reopened.user(&session).as_deref(),
            Some("bob@ejemplo.com"),
            "la sesión vive en la base, no en memoria"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn asking_twice_in_a_row_does_not_send_twice() {
        let (auth, root) = auth("cooldown", "bob@ejemplo.com");
        assert!(auth.request_link("bob@ejemplo.com").is_some());
        assert!(
            auth.request_link("bob@ejemplo.com").is_none(),
            "muy seguido"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn cerrar_la_sesion_la_deja_afuera() {
        let (auth, root) = auth("close", "bob@ejemplo.com");
        let session = auth.open_session("bob@ejemplo.com").unwrap();
        assert!(auth.user(&session).is_some());
        auth.close_session(&session);
        assert!(auth.user(&session).is_none());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
