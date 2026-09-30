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
    dir TEXT NOT NULL UNIQUE,
    plan TEXT NOT NULL DEFAULT '',
    personal_of_id TEXT UNIQUE REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER
);
CREATE TABLE IF NOT EXISTS machines (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL UNIQUE REFERENCES orgs(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    sandbox TEXT NOT NULL,
    file_system TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER
);
CREATE TABLE IF NOT EXISTS projects (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    size INTEGER NOT NULL DEFAULT 0,
    unversioned INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER,
    UNIQUE (org_id, name)
);
CREATE TABLE IF NOT EXISTS conversations (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    project TEXT NOT NULL,
    title TEXT,
    read_only INTEGER NOT NULL DEFAULT 0,
    last TEXT,
    touched_at INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER,
    UNIQUE (org_id, key)
);
CREATE TABLE IF NOT EXISTS usage (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    model TEXT NOT NULL,
    day INTEGER NOT NULL,
    prompt_tokens INTEGER NOT NULL DEFAULT 0,
    completion_tokens INTEGER NOT NULL DEFAULT 0,
    cached_tokens INTEGER NOT NULL DEFAULT 0,
    calls INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER,
    UNIQUE (org_id, model, day)
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
CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    when_at TEXT,
    at TEXT,
    every TEXT,
    target TEXT,
    prompt TEXT NOT NULL,
    silent INTEGER NOT NULL DEFAULT 0,
    paused INTEGER NOT NULL DEFAULT 0,
    next_run_at INTEGER,
    claimed_at INTEGER,
    last_read_at INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER
);
CREATE UNIQUE INDEX IF NOT EXISTS tasks_by_name ON tasks (org_id, name)
    WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS tasks_by_next_run ON tasks (next_run_at)
    WHERE deleted_at IS NULL AND paused = 0;
CREATE TABLE IF NOT EXISTS task_runs (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    started_at INTEGER NOT NULL,
    ms INTEGER NOT NULL,
    ok INTEGER NOT NULL,
    text TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER
);
CREATE INDEX IF NOT EXISTS task_runs_by_task ON task_runs (task_id, started_at);
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
    /// El plan que pagó la org, y nada más: es lo que decide si tiene máquina.
    /// Sin plan no corre en ningún lado, y la máquina llega con el pago.
    pub plan: Option<String>,
    /// Dónde vive todo lo de esta org, relativo a la raíz del control plane.
    /// Adentro están sus conversaciones y su workspace. La primera org se queda
    /// la raíz, que es donde ya estaba todo.
    pub dir: String,
}

pub struct Store {
    db: Mutex<Connection>,
}

/// Una tarea de la agenda, con su reloj ya resuelto: `next_run_at` dice cuándo
/// le toca, y se recalcula cada vez que corre. Una tarea `when` sin correr
/// nunca no tiene próxima: corrió una sola vez.
#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub id: String,
    pub org_id: String,
    pub name: String,
    pub when_at: Option<String>,
    pub at: Option<String>,
    pub every: Option<String>,
    pub target: Option<String>,
    pub prompt: String,
    pub silent: bool,
    pub paused: bool,
    pub next_run_at: Option<i64>,
    pub last_read_at: i64,
}

/// Lo que hace falta para crear una tarea: el reloj lo pone quien sabe dónde
/// está la hora local.
#[derive(Debug, Clone, PartialEq)]
pub struct NewTask {
    pub name: String,
    pub prompt: String,
    pub when_at: Option<String>,
    pub at: Option<String>,
    pub every: Option<String>,
    pub target: Option<String>,
    pub silent: bool,
    pub next_run_at: Option<i64>,
}

/// Una corrida de una tarea, con lo que contestó.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub id: String,
    pub started_at: i64,
    pub ms: i64,
    pub ok: bool,
    pub text: String,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

const TASK_COLUMNS: &str = "SELECT id, org_id, name, when_at, at, every, target, prompt, silent,
     paused, next_run_at, last_read_at FROM tasks";

/// Una tarea reclamada hace más que esto se considera abandonada: el proceso
/// que la tenía se murió. Un turno largo dura una hora, así que hay margen.
const STALE: i64 = 2 * 3600;

/// Cuántas corridas se guardan por tarea. El historial es para mirar, no para
/// archivar: con una tarea cada cinco minutos, veinte alcanzaban.
const KEEP_RUNS: i64 = 200;

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
                let id = ulid::new();
                let primera: i64 = tx
                    .query_row("SELECT count(*) FROM orgs", [], |row| row.get(0))
                    .map_err(|e| e.to_string())?;
                let org = Org {
                    dir: if primera == 0 {
                        ".".to_string()
                    } else {
                        format!("orgs/{id}")
                    },
                    id,
                    name: personal_name(&email),
                    plan: None,
                };
                tx.execute(
                    "INSERT INTO orgs (id, name, dir, plan, personal_of_id, created_at, updated_at)
                     VALUES (?1, ?2, ?3, '', ?4, ?5, ?5)",
                    params![org.id, org.name, org.dir, user.id, now()],
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
        let id = ulid::new();
        let org = Org {
            dir: format!("orgs/{id}"),
            id,
            name: name.to_string(),
            plan: None,
        };
        tx.execute(
            "INSERT INTO orgs (id, name, dir, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
            params![org.id, org.name, org.dir, now()],
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
                "SELECT orgs.id, orgs.name, orgs.dir, orgs.plan FROM orgs
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

    /// Una org por su id.
    pub fn org(&self, id: &str) -> Result<Option<Org>, String> {
        let db = self.db.lock().unwrap();
        db.query_row(
            "SELECT id, name, dir, plan FROM orgs WHERE id = ?1 AND deleted_at IS NULL",
            params![id],
            read_org,
        )
        .optional()
        .map_err(|e| e.to_string())
    }

    /// Suma lo que consumió una llamada. Se guarda por org, modelo y día, así
    /// el gasto de un mes es una suma y la tabla no crece con cada llamada.
    pub fn sumar_uso(&self, org: &str, model: &str, uso: &Uso) -> Result<(), String> {
        let db = self.db.lock().unwrap();
        db.execute(
            "INSERT INTO usage (id, org_id, model, day, prompt_tokens, completion_tokens,
                               cached_tokens, calls, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
             ON CONFLICT(org_id, model, day) DO UPDATE SET
                 prompt_tokens = prompt_tokens + excluded.prompt_tokens,
                 completion_tokens = completion_tokens + excluded.completion_tokens,
                 cached_tokens = cached_tokens + excluded.cached_tokens,
                 calls = calls + excluded.calls,
                 updated_at = excluded.updated_at,
                 deleted_at = NULL",
            params![
                ulid::new(),
                org,
                model,
                dia(now()),
                uso.prompt as i64,
                uso.completion as i64,
                uso.cached as i64,
                uso.calls as i64,
                now()
            ],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    }

    /// Lo que consumió una org desde un día (incluido). Sin desde, todo.
    pub fn uso_de(&self, org: &str, desde: Option<i64>) -> Result<Uso, String> {
        let db = self.db.lock().unwrap();
        let mut uso = Uso::default();
        let mut statement = db
            .prepare(
                "SELECT prompt_tokens, completion_tokens, cached_tokens, calls FROM usage
                 WHERE org_id = ?1 AND deleted_at IS NULL AND day >= ?2",
            )
            .map_err(|e| e.to_string())?;
        let filas = statement
            .query_map(params![org, desde.unwrap_or(i64::MIN)], |row| {
                Ok(Uso {
                    prompt: row.get::<_, i64>(0)?.max(0) as u64,
                    completion: row.get::<_, i64>(1)?.max(0) as u64,
                    cached: row.get::<_, i64>(2)?.max(0) as u64,
                    calls: row.get::<_, i64>(3)?.max(0) as u64,
                })
            })
            .map_err(|e| e.to_string())?;
        for fila in filas {
            uso.sumar(&fila.map_err(|e| e.to_string())?);
        }
        Ok(uso)
    }

    /// La org de la instancia es la que se quedó la raíz, que es donde su
    /// trabajo vivió siempre: su máquina es la de acá. Sin fila no correría en
    /// ningún lado, así que se la da al arrancar. No pisa una que ya tenga.
    pub fn marcar_la_org_de_la_instancia(&self) -> Result<(), String> {
        let id = {
            let db = self.db.lock().unwrap();
            db.query_row(
                "SELECT id FROM orgs WHERE dir = '.' AND deleted_at IS NULL",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
        };
        let Some(id) = id else {
            return Ok(());
        };
        if self.machine(&id)?.is_some() {
            return Ok(());
        }
        self.set_machine(&id, "local", "", "")
    }

    /// El plan de una org. Es lo que decide si tiene máquina: sin plan no hay
    /// dónde correr, y la máquina llega con el pago. No hay planes que valgan
    /// por sí mismos: el nombre es el que diga el cobro.
    pub fn set_plan(&self, org: &str, plan: Option<&str>) -> Result<(), String> {
        let db = self.db.lock().unwrap();
        db.execute(
            "UPDATE orgs SET plan = ?1, updated_at = ?2 WHERE id = ?3 AND deleted_at IS NULL",
            params![plan.unwrap_or_default(), now(), org],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    }

    /// La org personal de alguien, por su mail.
    pub fn org_of_email(&self, email: &str) -> Result<Option<Org>, String> {
        let db = self.db.lock().unwrap();
        db.query_row(
            "SELECT orgs.id, orgs.name, orgs.dir, orgs.plan FROM orgs
             JOIN users ON users.id = orgs.personal_of_id
             WHERE users.email = ?1 AND orgs.deleted_at IS NULL AND users.deleted_at IS NULL",
            params![email.trim().to_lowercase()],
            read_org,
        )
        .optional()
        .map_err(|e| e.to_string())
    }

    /// Las tareas de una org, por nombre.
    pub fn tasks_of(&self, org: &str) -> Result<Vec<Task>, String> {
        let db = self.db.lock().unwrap();
        let mut statement = db
            .prepare(&format!(
                "{TASK_COLUMNS} WHERE org_id = ?1 AND deleted_at IS NULL ORDER BY name"
            ))
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map(params![org], read_task)
            .map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<Vec<Task>>>()
            .map_err(|e| e.to_string())
    }

    pub fn task_named(&self, org: &str, name: &str) -> Result<Option<Task>, String> {
        let db = self.db.lock().unwrap();
        db.query_row(
            &format!("{TASK_COLUMNS} WHERE org_id = ?1 AND name = ?2 AND deleted_at IS NULL"),
            params![org, name],
            read_task,
        )
        .optional()
        .map_err(|e| e.to_string())
    }

    pub fn create_task(&self, org: &str, new: NewTask) -> Result<Task, String> {
        let name = new.name.trim();
        if name.is_empty() {
            return Err("la tarea necesita un nombre".into());
        }
        let task = Task {
            id: ulid::new(),
            org_id: org.to_string(),
            name: name.to_string(),
            when_at: new.when_at,
            at: new.at,
            every: new.every,
            target: new.target,
            prompt: new.prompt,
            silent: new.silent,
            paused: false,
            next_run_at: new.next_run_at,
            last_read_at: 0,
        };
        let db = self.db.lock().unwrap();
        db.execute(
            "INSERT INTO tasks (id, org_id, name, when_at, at, every, target, prompt, silent,
                                paused, next_run_at, last_read_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10, 0, ?11, ?11)",
            params![
                task.id,
                task.org_id,
                task.name,
                task.when_at,
                task.at,
                task.every,
                task.target,
                task.prompt,
                task.silent,
                task.next_run_at,
                now()
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(task)
    }

    pub fn set_paused(&self, org: &str, name: &str, paused: bool) -> Result<(), String> {
        let db = self.db.lock().unwrap();
        let changed = db
            .execute(
                "UPDATE tasks SET paused = ?1, updated_at = ?2
                 WHERE org_id = ?3 AND name = ?4 AND deleted_at IS NULL",
                params![paused, now(), org, name],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err(format!("no existe la tarea {name}"));
        }
        Ok(())
    }

    /// La baja es lógica, como todo lo demás: la fila queda con su horario y
    /// sus corridas por si hay que mirarlas.
    pub fn delete_task(&self, org: &str, name: &str) -> Result<(), String> {
        let db = self.db.lock().unwrap();
        let changed = db
            .execute(
                "UPDATE tasks SET deleted_at = ?1, updated_at = ?1
                 WHERE org_id = ?2 AND name = ?3 AND deleted_at IS NULL",
                params![now(), org, name],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err(format!("no existe la tarea {name}"));
        }
        Ok(())
    }

    /// Marca leída la última corrida de una tarea, o la de todas: lo leído se
    /// guarda como el momento, y todo lo que llegó después es nuevo.
    pub fn mark_read(&self, org: &str, name: Option<&str>) -> Result<(), String> {
        let db = self.db.lock().unwrap();
        let stamp = now();
        match name {
            Some(name) => db.execute(
                "UPDATE tasks SET last_read_at = ?1, updated_at = ?1
                 WHERE org_id = ?2 AND name = ?3 AND deleted_at IS NULL",
                params![stamp, org, name],
            ),
            None => db.execute(
                "UPDATE tasks SET last_read_at = ?1, updated_at = ?1
                 WHERE org_id = ?2 AND deleted_at IS NULL",
                params![stamp, org],
            ),
        }
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Cuántas corridas llegaron desde la última mirada.
    pub fn unread_runs(&self, task: &str, since: i64) -> Result<usize, String> {
        let db = self.db.lock().unwrap();
        let count: i64 = db
            .query_row(
                "SELECT count(*) FROM task_runs
                 WHERE task_id = ?1 AND started_at > ?2 AND deleted_at IS NULL",
                params![task, since],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(count as usize)
    }

    /// Le adelanta el reloj para que la tome en la próxima vuelta. Una tarea
    /// pausada no corre ni a mano.
    pub fn run_now(&self, org: &str, name: &str) -> Result<(), String> {
        let db = self.db.lock().unwrap();
        let stamp = now();
        let changed = db
            .execute(
                "UPDATE tasks SET next_run_at = ?1, claimed_at = NULL, updated_at = ?1
                 WHERE org_id = ?2 AND name = ?3 AND deleted_at IS NULL AND paused = 0",
                params![stamp, org, name],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err(format!("la tarea {name} no existe o está pausada"));
        }
        Ok(())
    }

    /// Las últimas corridas de una tarea, de la más nueva a la más vieja.
    pub fn runs_of(&self, task: &str, limit: usize) -> Result<Vec<Run>, String> {
        let db = self.db.lock().unwrap();
        let mut statement = db
            .prepare(
                "SELECT id, started_at, ms, ok, text FROM task_runs
                 WHERE task_id = ?1 AND deleted_at IS NULL
                 ORDER BY started_at DESC, id DESC LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map(params![task, limit as i64], read_run)
            .map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<Vec<Run>>>()
            .map_err(|e| e.to_string())
    }

    /// Las tareas vencidas, reclamadas en la misma transacción: el que las
    /// saca de acá es el único que las va a correr. Una tarea reclamada hace
    /// demasiado se suelta sola, así que un proceso que muere en el medio no
    /// deja la tarea colgada para siempre.
    pub fn claim_due(&self, now: i64) -> Result<Vec<Task>, String> {
        let db = self.db.lock().unwrap();
        let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
        let due: Vec<Task> = {
            let mut statement = tx
                .prepare(&format!(
                    "{TASK_COLUMNS} WHERE deleted_at IS NULL AND paused = 0
                     AND next_run_at IS NOT NULL AND next_run_at <= ?1
                     AND (claimed_at IS NULL OR claimed_at < ?2)
                     ORDER BY next_run_at"
                ))
                .map_err(|e| e.to_string())?;
            let rows = statement
                .query_map(params![now, now - STALE], read_task)
                .map_err(|e| e.to_string())?;
            rows.collect::<rusqlite::Result<Vec<Task>>>()
                .map_err(|e| e.to_string())?
        };
        for task in &due {
            tx.execute(
                "UPDATE tasks SET claimed_at = ?1, updated_at = ?1 WHERE id = ?2",
                params![now, task.id],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(due)
    }

    /// Deja la corrida y reprograma la tarea. `None` en la próxima es una tarea
    /// que ya no vuelve a correr.
    pub fn record_run(
        &self,
        task: &Task,
        started_at: i64,
        next_run_at: Option<i64>,
        ms: i64,
        ok: bool,
        text: &str,
    ) -> Result<Run, String> {
        let run = Run {
            id: ulid::new(),
            started_at,
            ms,
            ok,
            text: text.to_string(),
        };
        let db = self.db.lock().unwrap();
        let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO task_runs (id, task_id, started_at, ms, ok, text, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?3, ?3)",
            params![run.id, task.id, run.started_at, run.ms, run.ok, run.text],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE tasks SET next_run_at = ?1, claimed_at = NULL, updated_at = ?2 WHERE id = ?3",
            params![next_run_at, run.started_at, task.id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM task_runs WHERE task_id = ?1 AND id NOT IN
                 (SELECT id FROM task_runs WHERE task_id = ?1
                  ORDER BY started_at DESC, id DESC LIMIT ?2)",
            params![task.id, KEEP_RUNS],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(run)
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
    let plan: String = row.get(3)?;
    Ok(Org {
        id: row.get(0)?,
        name: row.get(1)?,
        dir: row.get(2)?,
        plan: (!plan.is_empty()).then_some(plan),
    })
}

fn read_task(row: &rusqlite::Row) -> rusqlite::Result<Task> {
    Ok(Task {
        id: row.get(0)?,
        org_id: row.get(1)?,
        name: row.get(2)?,
        when_at: row.get(3)?,
        at: row.get(4)?,
        every: row.get(5)?,
        target: row.get(6)?,
        prompt: row.get(7)?,
        silent: row.get(8)?,
        paused: row.get(9)?,
        next_run_at: row.get(10)?,
        last_read_at: row.get(11)?,
    })
}

fn read_run(row: &rusqlite::Row) -> rusqlite::Result<Run> {
    Ok(Run {
        id: row.get(0)?,
        started_at: row.get(1)?,
        ms: row.get(2)?,
        ok: row.get(3)?,
        text: row.get(4)?,
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
        "SELECT id, name, dir, plan FROM orgs WHERE personal_of_id = ?1",
        params![user],
        read_org,
    )
    .optional()
    .map_err(|e| e.to_string())
}

fn orgs_of(db: &Connection, user: &str) -> Result<Vec<Org>, String> {
    let mut statement = db
        .prepare(
            "SELECT orgs.id, orgs.name, orgs.dir, orgs.plan FROM orgs
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

/// Dónde vive el trabajo de una org: el proveedor, el nombre de su máquina y
/// el filesystem que monta, que es su volumen. Sin fila, el trabajo corre acá.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub struct Machine {
    pub org_id: String,
    pub provider: String,
    pub sandbox: String,
    pub file_system: String,
}

impl Store {
    pub fn machine(&self, org: &str) -> Result<Option<Machine>, String> {
        let db = self.db.lock().unwrap();
        db.query_row(
            "SELECT org_id, provider, sandbox, file_system FROM machines WHERE org_id = ?1 AND deleted_at IS NULL",
            params![org],
            |row| {
                Ok(Machine {
                    org_id: row.get(0)?,
                    provider: row.get(1)?,
                    sandbox: row.get(2)?,
                    file_system: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(|e| e.to_string())
    }

    /// La escribe el alta de una org, que todavía no existe: hoy sólo la
    /// llaman las pruebas.
    pub fn set_machine(
        &self,
        org: &str,
        provider: &str,
        sandbox: &str,
        file_system: &str,
    ) -> Result<(), String> {
        if provider.trim().is_empty() {
            return Err("la máquina necesita un proveedor".into());
        }
        let db = self.db.lock().unwrap();
        db.execute(
            "INSERT INTO machines (id, org_id, provider, sandbox, file_system, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
             ON CONFLICT(org_id) DO UPDATE SET
                 provider = excluded.provider,
                 sandbox = excluded.sandbox,
                 file_system = excluded.file_system,
                 updated_at = excluded.updated_at,
                 deleted_at = NULL",
            params![ulid::new(), org, provider, sandbox, file_system, now()],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    }
}

/// Lo que hay adentro de una org, para poder mostrarlo sin abrir su volumen ni
/// despertarla: el volumen manda, esto es una copia.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexProject {
    pub name: String,
    pub size: u64,
    pub unversioned: bool,
    pub conversations: Vec<IndexConversation>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IndexConversation {
    pub key: String,
    pub project: String,
    pub title: Option<String>,
    pub read_only: bool,
    pub last: Option<String>,
    /// Cuándo se tocó lo de adentro: es lo que ordena la lista.
    pub touched_at: i64,
}

impl Store {
    /// Lo que hay en la org, como quedó en la última sincronización. Lo usa la
    /// web, que es el paso que sigue: mostrar la lista sin despertar el sandbox.
    #[allow(dead_code)]
    pub fn index(&self, org: &str) -> Result<Vec<IndexProject>, String> {
        let db = self.db.lock().unwrap();
        let mut proyectos: Vec<IndexProject> = Vec::new();
        let mut statement = db
            .prepare(
                "SELECT name, size, unversioned FROM projects
                 WHERE org_id = ?1 AND deleted_at IS NULL ORDER BY name",
            )
            .map_err(|e| e.to_string())?;
        let filas = statement
            .query_map(params![org], |row| {
                Ok(IndexProject {
                    name: row.get(0)?,
                    size: row.get::<_, i64>(1)?.max(0) as u64,
                    unversioned: row.get(2)?,
                    conversations: Vec::new(),
                })
            })
            .map_err(|e| e.to_string())?;
        for fila in filas {
            proyectos.push(fila.map_err(|e| e.to_string())?);
        }

        let mut statement = db
            .prepare(
                "SELECT key, project, title, read_only, last, touched_at FROM conversations
                 WHERE org_id = ?1 AND deleted_at IS NULL
                 ORDER BY touched_at DESC, key DESC",
            )
            .map_err(|e| e.to_string())?;
        let filas = statement
            .query_map(params![org], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    IndexConversation {
                        key: row.get(0)?,
                        project: row.get(1)?,
                        title: row.get(2)?,
                        read_only: row.get(3)?,
                        last: row.get(4)?,
                        touched_at: row.get(5)?,
                    },
                ))
            })
            .map_err(|e| e.to_string())?;
        for fila in filas {
            let (project, conversation) = fila.map_err(|e| e.to_string())?;
            if let Some(proyecto) = proyectos.iter_mut().find(|otro| otro.name == project) {
                proyecto.conversations.push(conversation);
            }
        }
        Ok(proyectos)
    }

    /// La conversación, como quedó en la última sincronización.
    #[allow(dead_code)]
    pub fn conversation_index(
        &self,
        org: &str,
        key: &str,
    ) -> Result<Option<IndexConversation>, String> {
        let db = self.db.lock().unwrap();
        db.query_row(
            "SELECT key, project, title, read_only, last, touched_at FROM conversations
             WHERE org_id = ?1 AND key = ?2 AND deleted_at IS NULL",
            params![org, key],
            |row| {
                Ok(IndexConversation {
                    key: row.get(0)?,
                    project: row.get(1)?,
                    title: row.get(2)?,
                    read_only: row.get(3)?,
                    last: row.get(4)?,
                    touched_at: row.get(5)?,
                })
            },
        )
        .optional()
        .map_err(|e| e.to_string())
    }

    /// Reemplaza el índice de la org con lo que se acaba de leer de su volumen.
    /// Lo que ya no está se da de baja: la fila queda, que la baja es lógica.
    pub fn sync_index(&self, org: &str, projects: &[IndexProject]) -> Result<(), String> {
        let db = self.db.lock().unwrap();
        let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
        let ahora = now();
        tx.execute(
            "UPDATE projects SET deleted_at = ?1 WHERE org_id = ?2 AND deleted_at IS NULL",
            params![ahora, org],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE conversations SET deleted_at = ?1 WHERE org_id = ?2 AND deleted_at IS NULL",
            params![ahora, org],
        )
        .map_err(|e| e.to_string())?;
        for proyecto in projects {
            tx.execute(
                "INSERT INTO projects (id, org_id, name, size, unversioned, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
                 ON CONFLICT(org_id, name) DO UPDATE SET
                     size = excluded.size,
                     unversioned = excluded.unversioned,
                     updated_at = excluded.updated_at,
                     deleted_at = NULL",
                params![
                    ulid::new(),
                    org,
                    proyecto.name,
                    proyecto.size as i64,
                    proyecto.unversioned,
                    ahora
                ],
            )
            .map_err(|e| e.to_string())?;
            for conversacion in &proyecto.conversations {
                tx.execute(
                    "INSERT INTO conversations (id, org_id, key, project, title, read_only, last,
                                               touched_at, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
                     ON CONFLICT(org_id, key) DO UPDATE SET
                         project = excluded.project,
                         title = excluded.title,
                         read_only = excluded.read_only,
                         last = excluded.last,
                         touched_at = excluded.touched_at,
                         updated_at = excluded.updated_at,
                         deleted_at = NULL",
                    params![
                        ulid::new(),
                        org,
                        conversacion.key,
                        conversacion.project,
                        conversacion.title,
                        conversacion.read_only,
                        conversacion.last,
                        conversacion.touched_at,
                        ahora
                    ],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        tx.commit().map_err(|e| e.to_string())
    }
}

/// Lo que consumió una org: los tokens que le manda al modelo y los que le
/// contesta, más cuántas llamadas fueron.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Uso {
    pub prompt: u64,
    pub completion: u64,
    pub cached: u64,
    pub calls: u64,
}

impl Uso {
    pub fn sumar(&mut self, otro: &Uso) {
        self.prompt += otro.prompt;
        self.completion += otro.completion;
        self.cached += otro.cached;
        self.calls += otro.calls;
    }

    pub fn total(&self) -> u64 {
        self.prompt + self.completion
    }
}

/// El día de un momento, sin calendario: los segundos enteros divididos por un
/// día. Alcanza para sumar por mes y no depende de ninguna zona horaria.
pub fn dia(de: i64) -> i64 {
    de.div_euclid(86_400)
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
        for table in [
            "users",
            "orgs",
            "memberships",
            "sessions",
            "machines",
            "projects",
            "conversations",
            "usage",
        ] {
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
    fn la_primera_org_se_queda_la_raiz_y_las_demas_tienen_la_suya() {
        let store = store("dirs");
        let (user, personal) = store.register("don@berti.sh").unwrap();
        assert_eq!(personal.dir, ".", "la primera se queda lo que ya había");
        let empresa = store.create_org(&user.id, "La Empresa").unwrap();
        assert_eq!(empresa.dir, format!("orgs/{}", empresa.id));
        let (_, otra) = store.register("ana@ejemplo.com").unwrap();
        assert_eq!(otra.dir, format!("orgs/{}", otra.id));
        assert_ne!(otra.id, personal.id);
    }

    #[test]
    fn el_nombre_de_la_org_personal_sale_del_mail() {
        assert_eq!(personal_name("don@berti.sh"), "don");
        assert_eq!(personal_name("a.b+c@x.com"), "a.b+c");
        assert_eq!(personal_name("don"), "don");
    }

    fn tarea(name: &str, next: Option<i64>) -> NewTask {
        NewTask {
            name: name.into(),
            prompt: "p".into(),
            when_at: None,
            at: Some("05:00".into()),
            every: None,
            target: None,
            silent: false,
            next_run_at: next,
        }
    }

    /// Dos orgs pueden tener una tarea con el mismo nombre: cada una ve la suya.
    #[test]
    fn las_tareas_de_una_org_son_suyas() {
        let store = store("tareas");
        let (user, personal) = store.register("don@berti.sh").unwrap();
        let empresa = store.create_org(&user.id, "La Empresa").unwrap();
        let de_bob = store
            .create_task(&personal.id, tarea("memoria", Some(100)))
            .unwrap();
        store
            .create_task(&empresa.id, tarea("memoria", Some(100)))
            .unwrap();

        assert_eq!(store.tasks_of(&personal.id).unwrap().len(), 1);
        assert_eq!(store.tasks_of(&empresa.id).unwrap().len(), 1);
        assert!(store.task_named(&personal.id, "otra").unwrap().is_none());

        store.set_paused(&personal.id, "memoria", true).unwrap();
        let mia = store.task_named(&personal.id, "memoria").unwrap().unwrap();
        assert!(mia.paused);
        let ajena = store.task_named(&empresa.id, "memoria").unwrap().unwrap();
        assert!(!ajena.paused, "la de la otra org no se toca");
        assert!(store.set_paused(&personal.id, "nada", true).is_err());
        assert_eq!(de_bob.id, mia.id);

        assert_eq!(store.tasks_of(&personal.id).unwrap().len(), 1);
    }

    /// El que reclama una tarea es el único que la corre: dos instancias del
    /// control plane no pueden hacer la misma dos veces.
    #[test]
    fn una_tarea_vencida_se_reclama_una_sola_vez() {
        let store = store("claim");
        let (_, org) = store.register("don@berti.sh").unwrap();
        store
            .create_task(&org.id, tarea("memoria", Some(100)))
            .unwrap();
        store
            .create_task(&org.id, tarea("futura", Some(10_000)))
            .unwrap();
        store
            .create_task(&org.id, tarea("pausada", Some(100)))
            .unwrap();
        store.set_paused(&org.id, "pausada", true).unwrap();

        let due = store.claim_due(1_000).unwrap();
        assert_eq!(due.len(), 1, "sólo la vencida y sin pausar");
        assert_eq!(due[0].name, "memoria");
        assert!(
            store.claim_due(1_000).unwrap().is_empty(),
            "ya está adentro"
        );

        store
            .record_run(&due[0], 1_000, Some(5_000), 5, true, "listo")
            .unwrap();
        assert!(
            store.claim_due(1_000).unwrap().is_empty(),
            "todavía no le toca"
        );
        assert_eq!(
            store.claim_due(6_000).unwrap().len(),
            1,
            "y a su hora vuelve"
        );
    }

    /// Un proceso que muere en el medio deja la tarea reclamada: pasado un
    /// rato, otro la puede volver a tomar.
    #[test]
    fn una_tarea_reclamada_y_abandonada_se_vuelve_a_reclamar() {
        let store = store("abandonada");
        let (_, org) = store.register("don@berti.sh").unwrap();
        store
            .create_task(&org.id, tarea("memoria", Some(100)))
            .unwrap();
        assert_eq!(store.claim_due(1_000).unwrap().len(), 1);
        assert!(
            store.claim_due(1_000 + STALE).unwrap().is_empty(),
            "el reclamo todavía vale"
        );
        assert_eq!(store.claim_due(1_000 + STALE + 1).unwrap().len(), 1);
    }

    #[test]
    fn el_historial_guarda_las_ultimas_corridas() {
        let store = store("historial");
        let (_, org) = store.register("don@berti.sh").unwrap();
        let task = store
            .create_task(&org.id, tarea("memoria", Some(100)))
            .unwrap();
        for vuelta in 0..(KEEP_RUNS + 5) {
            store
                .record_run(
                    &task,
                    vuelta,
                    Some(200 + vuelta),
                    vuelta,
                    true,
                    &format!("vuelta {vuelta}"),
                )
                .unwrap();
        }

        let runs = store.runs_of(&task.id, 5).unwrap();
        assert_eq!(runs.len(), 5, "la vista muestra las últimas");
        let guardadas: i64 = {
            let db = store.db.lock().unwrap();
            db.query_row("SELECT count(*) FROM task_runs", [], |row| row.get(0))
                .unwrap()
        };
        assert_eq!(guardadas, KEEP_RUNS, "y no se acumulan sin fin");
        let textos: Vec<String> = store
            .runs_of(&task.id, KEEP_RUNS as usize)
            .unwrap()
            .into_iter()
            .map(|run| run.text)
            .collect();
        assert!(
            !textos.iter().any(|texto| texto == "vuelta 0"),
            "las viejas se van"
        );
        assert!(
            textos.iter().any(|texto| texto == "vuelta 204"),
            "las nuevas quedan"
        );
    }
    /// El consumo se acumula por día y por modelo: dos llamadas del mismo día
    /// son una fila, y el total de un rango es una suma.
    #[test]
    fn el_consumo_de_una_org_se_acumula_por_dia() {
        let store = store("consumo");
        let (_, org) = store.register("don@ejemplo.com").unwrap();
        assert_eq!(store.uso_de(&org.id, None).unwrap(), Uso::default());

        let uno = Uso {
            prompt: 100,
            completion: 20,
            cached: 10,
            calls: 1,
        };
        store.sumar_uso(&org.id, "deepseek-flash", &uno).unwrap();
        store.sumar_uso(&org.id, "deepseek-flash", &uno).unwrap();
        store
            .sumar_uso(
                &org.id,
                "otro-modelo",
                &Uso {
                    prompt: 5,
                    calls: 1,
                    ..Uso::default()
                },
            )
            .unwrap();

        let total = store.uso_de(&org.id, None).unwrap();
        assert_eq!(total.prompt, 205);
        assert_eq!(total.completion, 40);
        assert_eq!(total.calls, 3);
        assert_eq!(total.total(), 245);
        assert_eq!(
            store.uso_de(&org.id, Some(dia(i64::MAX))).unwrap(),
            Uso::default()
        );
        assert_eq!(store.uso_de(&org.id, Some(0)).unwrap().calls, 3);
    }

    #[test]
    fn el_plan_de_una_org_decide_si_tiene_sandbox() {
        let store = store("plan");
        let (_, org) = store.register("don@ejemplo.com").unwrap();
        assert_eq!(org.plan, None);
        assert_eq!(store.org(&org.id).unwrap().unwrap().plan, None);

        store.set_plan(&org.id, Some("paid")).unwrap();
        assert_eq!(
            store.org(&org.id).unwrap().unwrap().plan.as_deref(),
            Some("paid")
        );
        let por_mail = store.org_of_email("DON@ejemplo.com").unwrap().unwrap();
        assert_eq!(por_mail.id, org.id);
        assert_eq!(por_mail.plan.as_deref(), Some("paid"));

        store.set_plan(&org.id, None).unwrap();
        assert_eq!(store.org(&org.id).unwrap().unwrap().plan, None);
        assert!(
            store.org_of_email("otro@ejemplo.com").unwrap().is_none(),
            "el que no está, no está"
        );
    }

    fn proyecto(
        name: &str,
        size: u64,
        unversioned: bool,
        conversations: Vec<IndexConversation>,
    ) -> IndexProject {
        IndexProject {
            name: name.to_string(),
            size,
            unversioned,
            conversations,
        }
    }

    fn charla(project: &str, key: &str, last: &str, touched_at: i64) -> IndexConversation {
        IndexConversation {
            key: key.to_string(),
            project: project.to_string(),
            title: Some("charla".into()),
            read_only: false,
            last: Some(last.to_string()),
            touched_at,
        }
    }

    /// El índice es una copia del volumen: se reemplaza entero en cada
    /// sincronización, así lo que se fue no queda colgado en la lista.
    #[test]
    fn el_indice_de_una_org_sigue_al_volumen() {
        let store = store("indice");
        let (_, org) = store.register("don@ejemplo.com").unwrap();
        assert!(store.index(&org.id).unwrap().is_empty());

        store
            .sync_index(
                &org.id,
                &[
                    proyecto(
                        "general",
                        0,
                        false,
                        vec![
                            charla("general", "web-vieja", "hola", 10),
                            charla("general", "web-nueva", "chau", 20),
                        ],
                    ),
                    proyecto("ken", 1024, true, vec![]),
                ],
            )
            .unwrap();

        let indice = store.index(&org.id).unwrap();
        assert_eq!(indice.len(), 2);
        assert_eq!(indice[0].name, "general");
        let charlas = &indice[0].conversations;
        assert_eq!(charlas[0].key, "web-nueva", "la última va primero");
        assert_eq!(charlas[1].key, "web-vieja");
        assert_eq!(indice[1].size, 1024);
        assert!(indice[1].unversioned);
        assert_eq!(
            store
                .conversation_index(&org.id, "web-vieja")
                .unwrap()
                .unwrap()
                .last
                .as_deref(),
            Some("hola")
        );

        // El volumen cambió: ken se fue y la charla no tiene último mensaje.
        let general = || {
            vec![proyecto(
                "general",
                0,
                false,
                vec![charla("general", "web-nueva", "chau", 20)],
            )]
        };
        store.sync_index(&org.id, &general()).unwrap();
        let indice = store.index(&org.id).unwrap();
        assert_eq!(indice.len(), 1, "ken ya no está");
        assert!(
            store
                .conversation_index(&org.id, "web-vieja")
                .unwrap()
                .is_none(),
            "la charla que se fue tampoco"
        );

        // Y sincronizar lo mismo no duplica nada.
        store.sync_index(&org.id, &general()).unwrap();
        assert_eq!(store.index(&org.id).unwrap()[0].conversations.len(), 1);
    }

    /// La org de la instancia es la que se quedó la raíz: su máquina es ésta,
    /// y se la da al arrancar. Las demás no corren acá.
    #[test]
    fn la_org_de_la_instancia_corre_aca() {
        let store = store("instancia");
        let (user, primera) = store.register("don@ejemplo.com").unwrap();
        assert_eq!(primera.dir, ".");
        let segunda = store.create_org(&user.id, "La Empresa").unwrap();
        assert_eq!(store.machine(&primera.id).unwrap(), None);

        store.marcar_la_org_de_la_instancia().unwrap();
        assert_eq!(
            store
                .machine(&primera.id)
                .unwrap()
                .unwrap()
                .provider
                .as_str(),
            "local"
        );
        assert_eq!(store.machine(&segunda.id).unwrap(), None);

        store
            .set_machine(&primera.id, "tensorlake", "x", "fs")
            .unwrap();
        store.marcar_la_org_de_la_instancia().unwrap();
        assert_eq!(
            store
                .machine(&primera.id)
                .unwrap()
                .unwrap()
                .provider
                .as_str(),
            "tensorlake",
            "no pisa la máquina que ya tiene"
        );
    }

    #[test]
    fn una_org_puede_tener_su_maquina() {
        let store = store("maquinas");
        let (user, _) = store.register("bob@ejemplo.com").unwrap();
        let org = store.create_org(&user.id, "La Empresa").unwrap();
        assert_eq!(store.machine(&org.id).unwrap(), None);

        store
            .set_machine(&org.id, "tensorlake", "turno-remoto", "jimmy-org")
            .unwrap();
        let guardada = store.machine(&org.id).unwrap().unwrap();
        assert_eq!(guardada.provider, "tensorlake");
        assert_eq!(guardada.sandbox, "turno-remoto");
        assert_eq!(guardada.file_system, "jimmy-org");

        store
            .set_machine(&org.id, "tensorlake", "otra", "otro-fs")
            .unwrap();
        let guardada = store.machine(&org.id).unwrap().unwrap();
        assert_eq!(guardada.sandbox, "otra");
        assert_eq!(guardada.file_system, "otro-fs");

        assert!(store.set_machine(&org.id, "  ", "x", "fs").is_err());
        store.set_machine(&org.id, "local", "", "").unwrap();
        assert_eq!(
            store.machine(&org.id).unwrap().unwrap().provider.as_str(),
            "local"
        );
    }
}
