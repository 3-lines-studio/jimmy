//! El estado del control plane: quién entra, a qué org pertenece y qué
//! sesiones están abiertas.
//!
//! Es SQLite en el volumen, con el esquema armado al abrir y sin migraciones,
//! igual que heimdall. Un solo proceso escribe, así que un mutex alcanza.
//!
//! Toda tabla lleva el mismo encabezado: el `ulid` como clave, `created_at`,
//! `updated_at` y `deleted_at`. La baja es lógica: la fila queda con
//! `deleted_at` y las consultas filtran. El ulid no se deriva de nada: el mail
//! de una persona no tiene por qué estar en el identificador de su org.

use crate::ulid;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const SCHEMA: &str = "
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS users (
    ulid TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER
);
CREATE TABLE IF NOT EXISTS orgs (
    ulid TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    personal_of TEXT UNIQUE REFERENCES users(ulid) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER
);
CREATE TABLE IF NOT EXISTS memberships (
    ulid TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES orgs(ulid) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(ulid) ON DELETE CASCADE,
    role TEXT NOT NULL DEFAULT 'member',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER,
    UNIQUE (org_id, user_id)
);
CREATE TABLE IF NOT EXISTS sessions (
    ulid TEXT PRIMARY KEY,
    token TEXT NOT NULL UNIQUE,
    user_id TEXT NOT NULL REFERENCES users(ulid) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER,
    expires_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS sessions_by_user ON sessions (user_id);
";

#[derive(Debug, Clone, PartialEq)]
pub struct User {
    pub ulid: String,
    pub email: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Org {
    pub ulid: String,
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

/// Cómo se llama la org personal de alguien: la parte de su mail antes del
/// arroba. Es sólo el nombre, que se puede cambiar; el identificador es el ulid.
fn personal_name(email: &str) -> String {
    let part = email.split('@').next().unwrap_or(email).trim();
    if part.is_empty() {
        email.to_string()
    } else {
        part.to_string()
    }
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
    /// que entra. Volver a entrar no duplica ni cambia nada.
    pub fn register(&self, email: &str) -> Result<(User, Org), String> {
        let email = email.trim().to_lowercase();
        let db = self.db.lock().unwrap();
        let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT OR IGNORE INTO users (ulid, email, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3)",
            params![ulid::new(), email, now()],
        )
        .map_err(|e| e.to_string())?;
        let user = user_by_email(&tx, &email)?
            .ok_or_else(|| format!("no pude crear el usuario {email}"))?;
        // Volver a entrar revive al que se había ido: no hay dos cuentas con el
        // mismo mail, ni una cuenta que no pueda volver.
        tx.execute(
            "UPDATE users SET deleted_at = NULL, updated_at = ?1
             WHERE ulid = ?2 AND deleted_at IS NOT NULL",
            params![now(), user.ulid],
        )
        .map_err(|e| e.to_string())?;
        let org = match personal_org(&tx, &user.ulid)? {
            Some(org) => org,
            None => {
                let org = Org {
                    ulid: ulid::new(),
                    name: personal_name(&email),
                };
                tx.execute(
                    "INSERT INTO orgs (ulid, name, personal_of, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?4)",
                    params![org.ulid, org.name, user.ulid, now()],
                )
                .map_err(|e| e.to_string())?;
                tx.execute(
                    "INSERT INTO memberships (ulid, org_id, user_id, role, created_at, updated_at)
                     VALUES (?1, ?2, ?3, 'owner', ?4, ?4)",
                    params![ulid::new(), org.ulid, user.ulid, now()],
                )
                .map_err(|e| e.to_string())?;
                org
            }
        };
        tx.commit().map_err(|e| e.to_string())?;
        Ok((user, org))
    }

    pub fn open_session(&self, token: &str, user: &str, expires_at: i64) -> Result<(), String> {
        let db = self.db.lock().unwrap();
        db.execute(
            "INSERT INTO sessions (ulid, token, user_id, created_at, updated_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?4, ?5)",
            params![ulid::new(), token, user, now(), expires_at],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Quién es el dueño de esa sesión, si sigue viva y no la cerraron.
    pub fn session_user(&self, token: &str) -> Result<Option<User>, String> {
        let db = self.db.lock().unwrap();
        user_by_session(&db, token)
    }

    /// Cerrar sesión es una baja, no un borrado: la fila queda como historial
    /// hasta que venza, y ahí la limpia el barrido.
    pub fn close_session(&self, token: &str) -> Result<(), String> {
        let db = self.db.lock().unwrap();
        db.execute(
            "UPDATE sessions SET deleted_at = ?1, updated_at = ?1
             WHERE token = ?2 AND deleted_at IS NULL",
            params![now(), token],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Las vencidas se borran de verdad: ya no son historial de nadie.
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
        ulid: row.get(0)?,
        email: row.get(1)?,
        name: row.get(2)?,
    })
}

fn read_org(row: &rusqlite::Row) -> rusqlite::Result<Org> {
    Ok(Org {
        ulid: row.get(0)?,
        name: row.get(1)?,
    })
}

fn user_by_email(db: &Connection, email: &str) -> Result<Option<User>, String> {
    db.query_row(
        "SELECT ulid, email, name FROM users WHERE email = ?1",
        params![email],
        read_user,
    )
    .optional()
    .map_err(|e| e.to_string())
}

/// La sesión de alguien que ya no está no sirve, aunque siga viva.
fn user_by_session(db: &Connection, token: &str) -> Result<Option<User>, String> {
    db.query_row(
        "SELECT users.ulid, users.email, users.name FROM users
         JOIN sessions ON sessions.user_id = users.ulid
         WHERE sessions.token = ?1 AND sessions.expires_at > ?2
           AND sessions.deleted_at IS NULL
           AND users.deleted_at IS NULL",
        params![token, now()],
        read_user,
    )
    .optional()
    .map_err(|e| e.to_string())
}

fn personal_org(db: &Connection, user: &str) -> Result<Option<Org>, String> {
    db.query_row(
        "SELECT ulid, name FROM orgs WHERE personal_of = ?1",
        params![user],
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

    fn memberships(store: &Store) -> Vec<(String, String, String)> {
        let db = store.db.lock().unwrap();
        let mut statement = db
            .prepare("SELECT org_id, user_id, role FROM memberships ORDER BY org_id")
            .unwrap();
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap();
        rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
    }

    fn orgs(store: &Store) -> Vec<String> {
        let db = store.db.lock().unwrap();
        let mut statement = db.prepare("SELECT ulid FROM orgs ORDER BY ulid").unwrap();
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
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
        assert_eq!(org.name, "don");
        assert_eq!(first.ulid.len(), 26);
        assert_eq!(org.ulid.len(), 26);
        assert_eq!(
            memberships(&store),
            vec![(org.ulid.clone(), first.ulid.clone(), "owner".into())]
        );
    }

    #[test]
    fn cada_uno_tiene_su_ulid_y_no_sale_de_nadie() {
        let store = store("ulids");
        let (ana, org_ana) = store.register("ana@ejemplo.com").unwrap();
        let (beto, org_beto) = store.register("beto@ejemplo.com").unwrap();
        assert_ne!(ana.ulid, beto.ulid);
        assert_ne!(org_ana.ulid, org_beto.ulid);
        assert!(!org_ana.ulid.contains("ana"), "{}", org_ana.ulid);
        assert!(!ana.ulid.contains("ejemplo"), "{}", ana.ulid);
        assert_eq!(orgs(&store).len(), 2);
    }

    #[test]
    fn el_que_se_va_no_usa_su_sesion_y_vuelve_si_entra_de_nuevo() {
        let store = store("deleted");
        let (user, _) = store.register("don@berti.sh").unwrap();
        store.open_session("viva", &user.ulid, now() + 60).unwrap();
        assert!(store.session_user("viva").unwrap().is_some());
        store
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE users SET deleted_at = ?1 WHERE ulid = ?2",
                params![now(), user.ulid],
            )
            .unwrap();
        assert_eq!(
            store.session_user("viva").unwrap(),
            None,
            "el que se fue no entra, aunque la sesión siga viva"
        );
        let (again, _) = store.register("don@berti.sh").unwrap();
        assert_eq!(again, user);
        assert!(
            store.session_user("viva").unwrap().is_some(),
            "volver a entrar lo revive"
        );
    }

    #[test]
    fn al_crear_no_hay_nada_que_actualizar_ni_que_borrar() {
        let store = store("stamps");
        store.register("don@berti.sh").unwrap();
        let db = store.db.lock().unwrap();
        for table in ["users", "orgs"] {
            let (created, updated, deleted): (i64, i64, Option<i64>) = db
                .query_row(
                    &format!("SELECT created_at, updated_at, deleted_at FROM {table}"),
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            assert_eq!(created, updated, "recién creado no se tocó: {table}");
            assert_eq!(deleted, None, "recién creado no se borró: {table}");
        }
    }

    #[test]
    fn una_sesion_viva_dice_quien_es_y_una_vencida_no() {
        let store = store("sessions");
        let (user, _) = store.register("don@berti.sh").unwrap();
        store.open_session("viva", &user.ulid, now() + 60).unwrap();
        store.open_session("vieja", &user.ulid, now() - 1).unwrap();
        assert_eq!(store.session_user("viva").unwrap(), Some(user.clone()));
        assert_eq!(store.session_user("vieja").unwrap(), None);
        assert_eq!(store.session_user("inventada").unwrap(), None);
        store.close_session("viva").unwrap();
        assert_eq!(store.session_user("viva").unwrap(), None);
        let marcadas: i64 = store
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM sessions WHERE token = 'viva' AND deleted_at IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(marcadas, 1, "cerrar la marca, no la borra");
    }

    /// La coherencia de las estructuras es una regla, no una costumbre: toda
    /// tabla lleva las mismas marcas, y este test la sostiene.
    #[test]
    fn todas_las_tablas_tienen_las_mismas_marcas() {
        let store = store("uniform");
        let db = store.db.lock().unwrap();
        for table in ["users", "orgs", "memberships", "sessions"] {
            let mut statement = db.prepare(&format!("PRAGMA table_info({table})")).unwrap();
            let columns: Vec<String> = statement
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            for column in ["ulid", "created_at", "updated_at", "deleted_at"] {
                assert!(
                    columns.contains(&column.to_string()),
                    "{table} no tiene {column}: {columns:?}"
                );
            }
            assert!(
                !columns.contains(&"id".to_string()),
                "{table} todavía tiene un id al lado del ulid: {columns:?}"
            );
        }
    }

    #[test]
    fn las_sesiones_vencidas_se_limpian() {
        let store = store("expired");
        let (user, _) = store.register("don@berti.sh").unwrap();
        store.open_session("viva", &user.ulid, now() + 60).unwrap();
        store.open_session("vieja", &user.ulid, now() - 1).unwrap();
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
    fn el_nombre_de_la_org_personal_sale_del_mail() {
        assert_eq!(personal_name("don@berti.sh"), "don");
        assert_eq!(personal_name("a.b+c@x.com"), "a.b+c");
        assert_eq!(personal_name("don"), "don");
    }
}
