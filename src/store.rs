//! El estado del control plane: quién entra, a qué org pertenece y qué
//! sesiones están abiertas.
//!
//! Es SQLite en el volumen, con el esquema armado al abrir y sin migraciones,
//! igual que heimdall. Un solo proceso escribe, así que un mutex alcanza.
//!
//! Toda tabla lleva el mismo encabezado: `id`, `created_at`, `updated_at` y
//! `deleted_at`. El id es un ulid: opaco, ordenable por cuándo se creó y sin
//! derivarse de nada, porque el mail de una persona no tiene por qué estar en
//! el identificador de su org. La baja es lógica: la fila queda con
//! `deleted_at` y las consultas filtran.

use crate::ulid;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const SCHEMA: &str = "
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL DEFAULT '',
    active_org_id TEXT REFERENCES orgs(id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER
);
CREATE TABLE IF NOT EXISTS orgs (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    personal_of_id TEXT UNIQUE REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER
);
CREATE TABLE IF NOT EXISTS memberships (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role TEXT NOT NULL DEFAULT 'member',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER,
    UNIQUE (org_id, user_id)
);
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    token TEXT NOT NULL UNIQUE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER,
    expires_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS sessions_by_user ON sessions (user_id);
";

#[derive(Debug, Clone, PartialEq)]
pub struct User {
    pub id: String,
    pub email: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Org {
    pub id: String,
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
/// arroba. Es sólo el nombre, que se puede cambiar; el identificador es el id.
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
            "INSERT OR IGNORE INTO users (id, email, created_at, updated_at)
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
             WHERE id = ?2 AND deleted_at IS NOT NULL",
            params![now(), user.id],
        )
        .map_err(|e| e.to_string())?;
        let org = match personal_org(&tx, &user.id)? {
            Some(org) => org,
            None => {
                let org = Org {
                    id: ulid::new(),
                    name: personal_name(&email),
                };
                tx.execute(
                    "INSERT INTO orgs (id, name, personal_of_id, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?4)",
                    params![org.id, org.name, user.id, now()],
                )
                .map_err(|e| e.to_string())?;
                tx.execute(
                    "INSERT INTO memberships (id, org_id, user_id, role, created_at, updated_at)
                     VALUES (?1, ?2, ?3, 'owner', ?4, ?4)",
                    params![ulid::new(), org.id, user.id, now()],
                )
                .map_err(|e| e.to_string())?;
                // La primera org en la que se entra es la que queda activa: la
                // personal. Si ya tenía una elegida, no se pisa.
                tx.execute(
                    "UPDATE users SET active_org_id = ?1
                     WHERE id = ?2 AND active_org_id IS NULL",
                    params![org.id, user.id],
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
            "INSERT INTO sessions (id, token, user_id, created_at, updated_at, expires_at)
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

    /// Las orgs de ese usuario, en el orden en que se crearon.
    pub fn orgs_of(&self, user: &str) -> Result<Vec<Org>, String> {
        let db = self.db.lock().unwrap();
        orgs_of(&db, user)
    }

    /// Una org nueva, con el que la crea como dueño.
    pub fn create_org(&self, user: &str, name: &str) -> Result<Org, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("la org necesita un nombre".into());
        }
        let db = self.db.lock().unwrap();
        let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
        let org = Org {
            id: ulid::new(),
            name: name.to_string(),
        };
        tx.execute(
            "INSERT INTO orgs (id, name, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
            params![org.id, org.name, now()],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO memberships (id, org_id, user_id, role, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'owner', ?4, ?4)",
            params![ulid::new(), org.id, user, now()],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(org)
    }

    /// Sólo se puede estar en una org de la que se es parte.
    pub fn set_active_org(&self, user: &str, org: &str) -> Result<(), String> {
        let db = self.db.lock().unwrap();
        if !is_member(&db, org, user)? {
            return Err("esa org no es tuya".into());
        }
        db.execute(
            "UPDATE users SET active_org_id = ?1, updated_at = ?2 WHERE id = ?3",
            params![org, now(), user],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// La org en la que está trabajando, o su org personal si la elegida ya no
    /// está a mano.
    pub fn active_org(&self, user: &str) -> Result<Option<Org>, String> {
        let db = self.db.lock().unwrap();
        let elegida = db
            .query_row(
                "SELECT orgs.id, orgs.name FROM orgs
                 JOIN users ON users.active_org_id = orgs.id
                 JOIN memberships ON memberships.org_id = orgs.id
                     AND memberships.user_id = users.id
                 WHERE users.id = ?1
                   AND orgs.deleted_at IS NULL
                   AND memberships.deleted_at IS NULL",
                params![user],
                read_org,
            )
            .optional()
            .map_err(|e| e.to_string())?;
        match elegida {
            Some(org) => Ok(Some(org)),
            None => personal_org(&db, user),
        }
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
        name: row.get(1)?,
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

/// La sesión de alguien que ya no está no sirve, aunque siga viva.
fn user_by_session(db: &Connection, token: &str) -> Result<Option<User>, String> {
    db.query_row(
        "SELECT users.id, users.email, users.name FROM users
         JOIN sessions ON sessions.user_id = users.id
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
        "SELECT id, name FROM orgs WHERE personal_of_id = ?1",
        params![user],
        read_org,
    )
    .optional()
    .map_err(|e| e.to_string())
}

fn orgs_of(db: &Connection, user: &str) -> Result<Vec<Org>, String> {
    let mut statement = db
        .prepare(
            "SELECT orgs.id, orgs.name FROM orgs
             JOIN memberships ON memberships.org_id = orgs.id
             WHERE memberships.user_id = ?1
               AND memberships.deleted_at IS NULL
               AND orgs.deleted_at IS NULL
             ORDER BY orgs.id",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(params![user], read_org)
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<Org>>>()
        .map_err(|e| e.to_string())
}

fn is_member(db: &Connection, org: &str, user: &str) -> Result<bool, String> {
    db.query_row(
        "SELECT 1 FROM memberships
         WHERE org_id = ?1 AND user_id = ?2 AND deleted_at IS NULL",
        params![org, user],
        |_| Ok(()),
    )
    .optional()
    .map(|found| found.is_some())
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
        let mut statement = db.prepare("SELECT id FROM orgs ORDER BY id").unwrap();
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
        assert_eq!(first.id.len(), 26);
        assert_eq!(org.id.len(), 26);
        assert_eq!(
            memberships(&store),
            vec![(org.id.clone(), first.id.clone(), "owner".into())]
        );
    }

    #[test]
    fn cada_uno_tiene_su_id_y_no_sale_de_nadie() {
        let store = store("ids");
        let (ana, org_ana) = store.register("ana@ejemplo.com").unwrap();
        let (beto, org_beto) = store.register("beto@ejemplo.com").unwrap();
        assert_ne!(ana.id, beto.id);
        assert_ne!(org_ana.id, org_beto.id);
        assert!(!org_ana.id.contains("ana"), "{}", org_ana.id);
        assert!(!ana.id.contains("ejemplo"), "{}", ana.id);
        assert_eq!(orgs(&store).len(), 2);
    }

    #[test]
    fn el_que_se_va_no_usa_su_sesion_y_vuelve_si_entra_de_nuevo() {
        let store = store("deleted");
        let (user, _) = store.register("don@berti.sh").unwrap();
        store.open_session("viva", &user.id, now() + 60).unwrap();
        assert!(store.session_user("viva").unwrap().is_some());
        store
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE users SET deleted_at = ?1 WHERE id = ?2",
                params![now(), user.id],
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
        store.open_session("viva", &user.id, now() + 60).unwrap();
        store.open_session("vieja", &user.id, now() - 1).unwrap();
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
    /// tabla lleva las mismas marcas y el id es un ulid, no un entero.
    #[test]
    fn todas_las_tablas_tienen_las_mismas_marcas() {
        let store = store("uniform");
        let db = store.db.lock().unwrap();
        for table in ["users", "orgs", "memberships", "sessions"] {
            let mut statement = db.prepare(&format!("PRAGMA table_info({table})")).unwrap();
            let rows: Vec<(String, String)> = statement
                .query_map([], |row| Ok((row.get(1)?, row.get(2)?)))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            let columns: Vec<&str> = rows.iter().map(|(name, _)| name.as_str()).collect();
            for column in ["id", "created_at", "updated_at", "deleted_at"] {
                assert!(columns.contains(&column), "{table} no tiene {column}");
            }
            let id_type = rows
                .iter()
                .find(|(name, _)| name == "id")
                .map(|(_, kind)| kind.as_str())
                .unwrap_or_default();
            assert_eq!(id_type, "TEXT", "{table}: el id tiene que ser un ulid");
        }
    }

    #[test]
    fn las_sesiones_vencidas_se_limpian() {
        let store = store("expired");
        let (user, _) = store.register("don@berti.sh").unwrap();
        store.open_session("viva", &user.id, now() + 60).unwrap();
        store.open_session("vieja", &user.id, now() - 1).unwrap();
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
    fn la_org_activa_arranca_en_la_personal_y_se_puede_cambiar() {
        let store = store("activa");
        let (user, personal) = store.register("don@berti.sh").unwrap();
        assert_eq!(store.active_org(&user.id).unwrap(), Some(personal.clone()));
        let empresa = store.create_org(&user.id, "La Empresa").unwrap();
        assert_eq!(
            store.orgs_of(&user.id).unwrap(),
            vec![personal.clone(), empresa.clone()]
        );
        store.set_active_org(&user.id, &empresa.id).unwrap();
        assert_eq!(store.active_org(&user.id).unwrap(), Some(empresa));
    }

    #[test]
    fn una_org_de_la_que_no_sos_parte_no_se_activa() {
        let store = store("ajena");
        let (ana, org_ana) = store.register("ana@ejemplo.com").unwrap();
        let (_, org_beto) = store.register("beto@ejemplo.com").unwrap();
        assert!(store.set_active_org(&ana.id, &org_beto.id).is_err());
        assert_eq!(store.active_org(&ana.id).unwrap(), Some(org_ana));
        assert_eq!(store.orgs_of(&ana.id).unwrap().len(), 1);
    }

    #[test]
    fn si_la_org_elegida_desaparece_vuelve_a_la_personal() {
        let store = store("huerfana");
        let (user, personal) = store.register("don@berti.sh").unwrap();
        let empresa = store.create_org(&user.id, "La Empresa").unwrap();
        store.set_active_org(&user.id, &empresa.id).unwrap();
        store
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE orgs SET deleted_at = ?1 WHERE id = ?2",
                params![now(), empresa.id],
            )
            .unwrap();
        assert_eq!(store.active_org(&user.id).unwrap(), Some(personal));
        assert_eq!(store.orgs_of(&user.id).unwrap().len(), 1);
    }

    #[test]
    fn una_org_sin_nombre_no_se_crea() {
        let store = store("sin-nombre");
        let (user, _) = store.register("don@berti.sh").unwrap();
        assert!(store.create_org(&user.id, "   ").is_err());
        assert_eq!(store.orgs_of(&user.id).unwrap().len(), 1);
    }

    #[test]
    fn el_nombre_de_la_org_personal_sale_del_mail() {
        assert_eq!(personal_name("don@berti.sh"), "don");
        assert_eq!(personal_name("a.b+c@x.com"), "a.b+c");
        assert_eq!(personal_name("don"), "don");
    }
}
