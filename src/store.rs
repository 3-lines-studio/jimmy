//! El estado del control plane: quién entra, a qué org pertenece y qué
//! sesiones están abiertas.
//!
//! Es SQLite en el volumen, con el esquema armado al abrir y sin migraciones,
//! igual que heimdall. Un solo proceso escribe, así que un mutex alcanza.

use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const SCHEMA: &str = "
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS users (
    id INTEGER PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS orgs (
    id INTEGER PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS memberships (
    org_id INTEGER NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role TEXT NOT NULL DEFAULT 'member',
    created_at INTEGER NOT NULL,
    PRIMARY KEY (org_id, user_id)
);
CREATE TABLE IF NOT EXISTS sessions (
    token TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS sessions_by_user ON sessions (user_id);
";

#[derive(Debug, Clone, PartialEq)]
pub struct User {
    pub id: i64,
    pub email: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Org {
    pub id: i64,
    pub slug: String,
    pub name: String,
}

pub struct Store {
    db: Mutex<Connection>,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// El slug de la org personal sale del mail entero, sin lo que no sea letra o
/// número. Es único porque el mail lo es, así que no hay que contar colisiones.
pub fn slug_from_email(email: &str) -> String {
    let mut slug = String::new();
    for c in email.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    slug
}

impl Store {
    pub fn open(path: &Path) -> Result<Store, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("no pude crear {parent:?}: {e}"))?;
        }
        let db = Connection::open(path).map_err(|e| format!("no pude abrir {path:?}: {e}"))?;
        db.query_row("PRAGMA journal_mode=WAL", [], |_| Ok(()))
            .map_err(|e| format!("no pude poner {path:?} en WAL: {e}"))?;
        db.execute_batch(SCHEMA)
            .map_err(|e| format!("no pude armar el esquema: {e}"))?;
        Ok(Store { db: Mutex::new(db) })
    }

    /// El usuario de ese mail, con su org personal, que se crea la primera vez
    /// que entra. Volver a entrar no duplica nada.
    pub fn register(&self, email: &str) -> Result<(User, Org), String> {
        let email = email.trim().to_lowercase();
        let db = self.db.lock().unwrap();
        let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT OR IGNORE INTO users (email, created_at) VALUES (?1, ?2)",
            params![email, now()],
        )
        .map_err(|e| e.to_string())?;
        let user = user_by_email(&tx, &email)?
            .ok_or_else(|| format!("no pude crear el usuario {email}"))?;
        let slug = slug_from_email(&email);
        tx.execute(
            "INSERT OR IGNORE INTO orgs (slug, name, created_at) VALUES (?1, ?2, ?3)",
            params![slug, slug, now()],
        )
        .map_err(|e| e.to_string())?;
        let org = org_by_slug(&tx, &slug)?.ok_or_else(|| format!("no pude crear la org {slug}"))?;
        tx.execute(
            "INSERT OR IGNORE INTO memberships (org_id, user_id, role, created_at)
             VALUES (?1, ?2, 'owner', ?3)",
            params![org.id, user.id, now()],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok((user, org))
    }

    pub fn open_session(&self, token: &str, user_id: i64, expires_at: i64) -> Result<(), String> {
        let db = self.db.lock().unwrap();
        db.execute(
            "INSERT OR REPLACE INTO sessions (token, user_id, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![token, user_id, now(), expires_at],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Quién es el dueño de esa sesión, si sigue viva.
    pub fn session_user(&self, token: &str) -> Result<Option<User>, String> {
        let db = self.db.lock().unwrap();
        user_by_session(&db, token)
    }

    pub fn close_session(&self, token: &str) -> Result<(), String> {
        let db = self.db.lock().unwrap();
        db.execute("DELETE FROM sessions WHERE token = ?1", params![token])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Las sesiones vencidas no resuelven, así que se limpian cuando se puede:
    /// nadie las mira.
    pub fn forget_expired_sessions(&self) -> Result<usize, String> {
        let db = self.db.lock().unwrap();
        db.execute(
            "DELETE FROM sessions WHERE expires_at <= ?1",
            params![now()],
        )
        .map_err(|e| e.to_string())
    }
}

fn read_user(row: &rusqlite::Row) -> rusqlite::Result<User> {
    Ok(User {
        id: row.get(0)?,
        email: row.get(1)?,
        name: row.get(2)?,
    })
}

fn read_org(row: &rusqlite::Row) -> rusqlite::Result<Org> {
    Ok(Org {
        id: row.get(0)?,
        slug: row.get(1)?,
        name: row.get(2)?,
    })
}

fn user_by_email(db: &Connection, email: &str) -> Result<Option<User>, String> {
    db.query_row(
        "SELECT id, email, name FROM users WHERE email = ?1",
        params![email],
        read_user,
    )
    .optional()
    .map_err(|e| e.to_string())
}

fn user_by_session(db: &Connection, token: &str) -> Result<Option<User>, String> {
    db.query_row(
        "SELECT users.id, users.email, users.name FROM users
         JOIN sessions ON sessions.user_id = users.id
         WHERE sessions.token = ?1 AND sessions.expires_at > ?2",
        params![token, now()],
        read_user,
    )
    .optional()
    .map_err(|e| e.to_string())
}

fn org_by_slug(db: &Connection, slug: &str) -> Result<Option<Org>, String> {
    db.query_row(
        "SELECT id, slug, name FROM orgs WHERE slug = ?1",
        params![slug],
        read_org,
    )
    .optional()
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(tag: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("jimmy-store-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Store::open(&dir.join("jimmy.db")).unwrap()
    }

    fn memberships(store: &Store) -> Vec<(i64, i64, String)> {
        let db = store.db.lock().unwrap();
        let mut statement = db
            .prepare("SELECT org_id, user_id, role FROM memberships ORDER BY org_id")
            .unwrap();
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap();
        rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
    }

    #[test]
    fn entrar_dos_veces_no_duplica_ni_al_usuario_ni_a_su_org() {
        let store = store("register");
        let (first, org) = store.register("Don@Berti.sh").unwrap();
        let (again, same_org) = store.register("don@berti.sh").unwrap();
        assert_eq!(first, again);
        assert_eq!(org, same_org);
        assert_eq!(first.email, "don@berti.sh");
        assert_eq!(org.slug, "don-berti-sh");
        assert_eq!(
            memberships(&store),
            vec![(org.id, first.id, "owner".into())]
        );
    }

    #[test]
    fn una_sesion_viva_dice_quien_es_y_una_vencida_no() {
        let store = store("sessions");
        let (user, _) = store.register("don@berti.sh").unwrap();
        store.open_session("viva", user.id, now() + 60).unwrap();
        store.open_session("vieja", user.id, now() - 1).unwrap();
        assert_eq!(store.session_user("viva").unwrap(), Some(user.clone()));
        assert_eq!(store.session_user("vieja").unwrap(), None);
        assert_eq!(store.session_user("inventada").unwrap(), None);
        store.close_session("viva").unwrap();
        assert_eq!(store.session_user("viva").unwrap(), None);
    }

    #[test]
    fn las_sesiones_vencidas_se_limpian() {
        let store = store("expired");
        let (user, _) = store.register("don@berti.sh").unwrap();
        store.open_session("viva", user.id, now() + 60).unwrap();
        store.open_session("vieja", user.id, now() - 1).unwrap();
        assert_eq!(store.forget_expired_sessions().unwrap(), 1);
        assert_eq!(store.session_user("viva").unwrap(), Some(user));
    }

    #[test]
    fn lo_que_ya_esta_en_la_base_sigue_ahi_despues_de_reabrirla() {
        let dir = std::env::temp_dir().join(format!("jimmy-store-{}-reopen", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("jimmy.db");
        let (user, org) = Store::open(&path)
            .unwrap()
            .register("don@berti.sh")
            .unwrap();
        let reopened = Store::open(&path).unwrap();
        let (again, same_org) = reopened.register("don@berti.sh").unwrap();
        assert_eq!(again, user);
        assert_eq!(same_org, org);
        assert_eq!(memberships(&reopened).len(), 1);
    }

    #[test]
    fn el_slug_sale_del_mail_entero() {
        assert_eq!(slug_from_email("don@berti.sh"), "don-berti-sh");
        assert_eq!(slug_from_email("a.b+c@x.com"), "a-b-c-x-com");
        assert_eq!(slug_from_email("don"), "don");
        assert_eq!(slug_from_email("@"), "");
    }
}
