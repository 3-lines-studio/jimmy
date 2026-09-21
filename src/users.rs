//! Who can come in, and who is in.
//!
//! Passwords are argon2 hashes in `users.json`; a session is a token in a
//! cookie that also lives on disk, so a redeploy does not log anybody out.

use argon2::password_hash::{PasswordHash, SaltString};
use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const COOKIE: &str = "jimmy_session";

#[derive(Serialize, Deserialize)]
struct User {
    name: String,
    hash: String,
}

fn users_path(root: &Path) -> PathBuf {
    root.join("users.json")
}

fn load(root: &Path) -> Vec<User> {
    std::fs::read_to_string(users_path(root))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn count(root: &Path) -> usize {
    load(root).len()
}

pub fn verify(root: &Path, name: &str, password: &str) -> bool {
    let Some(user) = load(root).into_iter().find(|user| user.name == name) else {
        return false;
    };
    let Ok(parsed) = PasswordHash::new(&user.hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

pub fn add(root: &Path, name: &str, password: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("el nombre está vacío".into());
    }
    if password.is_empty() {
        return Err("la contraseña está vacía".into());
    }
    let mut users = load(root);
    if users.iter().any(|user| user.name == name) {
        return Err(format!("ya existe el usuario {name}"));
    }
    let salt = SaltString::encode_b64(&crate::random::bytes(16)).map_err(|e| e.to_string())?;
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| e.to_string())?;
    users.push(User {
        name: name.to_string(),
        hash,
    });
    let text = serde_json::to_string_pretty(&users).map_err(|e| e.to_string())?;
    axe::atomic_write(&users_path(root), text.as_bytes()).map_err(|e| e.to_string())
}

/// `jimmy user add <nombre>` reads the password from stdin, so it does not end
/// up in the shell history nor in `ps`.
pub fn command(args: &[String]) -> Result<String, String> {
    match args.first().map(String::as_str) {
        Some("add") => {
            let name = args.get(1).ok_or("uso: jimmy user add <nombre>")?;
            let mut password = String::new();
            std::io::stdin()
                .lock()
                .read_line(&mut password)
                .map_err(|e| e.to_string())?;
            add(
                &crate::root_from_env(),
                name,
                password.trim_end_matches(['\n', '\r']),
            )?;
            Ok(format!("usuario {name} agregado"))
        }
        _ => Err("uso: jimmy user add <nombre>".into()),
    }
}

pub struct Sessions {
    path: PathBuf,
    tokens: Mutex<HashMap<String, String>>,
}

impl Sessions {
    pub fn load(root: &Path) -> Sessions {
        let path = root.join("sessions.json");
        let tokens = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Sessions {
            path,
            tokens: Mutex::new(tokens),
        }
    }

    pub fn open(&self, name: &str) -> String {
        let token = crate::random::hex(32);
        let mut tokens = self.tokens.lock().unwrap();
        tokens.insert(token.clone(), name.to_string());
        self.save(&tokens);
        token
    }

    pub fn close(&self, token: &str) {
        let mut tokens = self.tokens.lock().unwrap();
        tokens.remove(token);
        self.save(&tokens);
    }

    pub fn user(&self, token: &str) -> Option<String> {
        self.tokens.lock().unwrap().get(token).cloned()
    }

    fn save(&self, tokens: &HashMap<String, String>) {
        if let Ok(text) = serde_json::to_string_pretty(tokens) {
            let _ = axe::atomic_write(&self.path, text.as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("jimmy-users-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn a_password_verifies_only_for_its_user() {
        let root = root("verify");
        add(&root, "berti", "secreto").unwrap();
        assert!(verify(&root, "berti", "secreto"));
        assert!(!verify(&root, "berti", "otro"));
        assert!(!verify(&root, "nadie", "secreto"));
        assert!(
            add(&root, "berti", "otro").is_err(),
            "no se repite el nombre"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_session_survives_a_reload() {
        let root = root("session");
        let sessions = Sessions::load(&root);
        let token = sessions.open("berti");
        assert_eq!(sessions.user(&token).as_deref(), Some("berti"));

        let reloaded = Sessions::load(&root);
        assert_eq!(reloaded.user(&token).as_deref(), Some("berti"));
        reloaded.close(&token);
        assert_eq!(reloaded.user(&token), None);
        assert_eq!(Sessions::load(&root).user(&token), None);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
