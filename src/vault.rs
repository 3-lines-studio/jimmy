use heimdall::cli;
use heimdall::store::{self, Store};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

const DIR: &str = "heimdall";
const VAR: &str = "HEIMDALL_DIR";

static VAULT: OnceLock<Result<Mutex<Store>, String>> = OnceLock::new();
static INJECTED: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn dir(root: &Path) -> std::path::PathBuf {
    root.join(DIR)
}

pub fn open(root: &Path) {
    let dir = dir(root);
    let vault = start(&dir);
    if vault.is_ok() {
        std::env::set_var(VAR, &dir);
        eprintln!("jimmy: vault en {}", dir.display());
    } else if let Err(error) = &vault {
        eprintln!("jimmy: el vault no abrió: {error}");
    }
    let _ = VAULT.set(vault);
}

pub fn store() -> Option<&'static Mutex<Store>> {
    VAULT.get()?.as_ref().ok()
}

pub fn message() -> &'static str {
    match VAULT.get() {
        Some(Err(error)) => error,
        _ => "el vault no arrancó",
    }
}

fn start(dir: &Path) -> Result<Mutex<Store>, String> {
    Store::open_dir(dir).map(Mutex::new)
}

/// Deja los secretos en el entorno del proceso, como los dejaría un shell: el
/// vault general y encima el del proyecto del directorio, sin pisar lo que ya
/// estaba definido. Devuelve cuántas variables puso.
pub fn inject(cwd: &Path) -> usize {
    let Some(held) = store() else {
        return 0;
    };
    apply(held, cwd)
}

fn apply(held: &Mutex<Store>, cwd: &Path) -> usize {
    let (project, env) = scope(cwd);
    let secrets = {
        let store = held.lock().unwrap_or_else(|e| e.into_inner());
        match store.resolve(&project, &env) {
            Ok(secrets) => secrets,
            Err(error) => {
                eprintln!("jimmy: no pude leer el vault: {}", text(error));
                return 0;
            }
        }
    };
    let count = put(&secrets);
    if count > 0 {
        eprintln!("jimmy: {count} variables del vault {project}/{env}");
    }
    count
}

/// Lo que estaba antes gana: solo se toca lo que puso la vuelta anterior.
fn put(secrets: &BTreeMap<String, String>) -> usize {
    let mut injected = INJECTED.lock().unwrap_or_else(|e| e.into_inner());
    let previous = std::mem::take(&mut *injected);
    for name in previous {
        std::env::remove_var(name);
    }
    for (name, value) in secrets {
        if std::env::var_os(name).is_some() {
            continue;
        }
        axe::sentinel::add(value);
        std::env::set_var(name, value);
        injected.push(name.clone());
    }
    injected.len()
}

fn scope(cwd: &Path) -> (String, String) {
    let (project, env) = cli::setup_at(cwd).unwrap_or_default();
    let project = crate::env("HEIMDALL_PROJECT").unwrap_or(project);
    let env = crate::env("HEIMDALL_ENV").unwrap_or_else(|| {
        if env.is_empty() {
            "dev".to_string()
        } else {
            env
        }
    });
    (project, env)
}

pub fn text(error: store::Error) -> String {
    match error {
        store::Error::Bad(message) | store::Error::Internal(message) => message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static ENV: Mutex<()> = Mutex::new(());

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("jimmy-vault-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn el_vault_guarda_un_secreto() {
        let _env = ENV.lock().unwrap();
        let dir = temp("guarda");

        let vault = start(&dir).unwrap();
        vault
            .lock()
            .unwrap()
            .set("prueba", "dev", "CLAVE", "valor", "test")
            .unwrap();

        assert_eq!(
            vault.lock().unwrap().secrets("prueba", "dev").unwrap()["CLAVE"],
            "valor"
        );
        assert!(dir.join("master.key").is_file());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn repo(dir: &Path, project: &str) -> std::path::PathBuf {
        let repo = dir.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(
            repo.join("heimdall.yaml"),
            format!("setup:\n  - project: {project}\n    config: dev\n"),
        )
        .unwrap();
        repo
    }

    #[test]
    fn los_secretos_quedan_en_el_entorno_con_el_proyecto_encima() {
        let _env = ENV.lock().unwrap();
        let dir = temp("inyecta");
        let store = start(&dir).unwrap();
        {
            let store = store.lock().unwrap();
            store
                .set("general", "dev", "JIMMY_TEST_A", "general", "test")
                .unwrap();
            store
                .set("demo", "dev", "JIMMY_TEST_A", "proyecto", "test")
                .unwrap();
            store
                .set("demo", "dev", "JIMMY_TEST_B", "solo acá", "test")
                .unwrap();
        }
        let repo = repo(&dir, "demo");

        assert_eq!(apply(&store, &repo), 2);
        assert_eq!(std::env::var("JIMMY_TEST_A").unwrap(), "proyecto");
        assert_eq!(std::env::var("JIMMY_TEST_B").unwrap(), "solo acá");

        let child = std::process::Command::new("sh")
            .args(["-c", "printf %s \"$JIMMY_TEST_B\""])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&child.stdout), "solo acá");

        let seen = axe::sentinel::redact("LINEA=solo acá");
        assert!(!seen.contains("solo acá"), "{seen}");

        put(&BTreeMap::new());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn una_variable_que_ya_estaba_no_se_pisa() {
        let _env = ENV.lock().unwrap();
        let dir = temp("respeta");
        let store = start(&dir).unwrap();
        store
            .lock()
            .unwrap()
            .set("demo", "dev", "JIMMY_TEST_C", "del vault", "test")
            .unwrap();
        let repo = repo(&dir, "demo");
        std::env::set_var("JIMMY_TEST_C", "de afuera");

        apply(&store, &repo);

        assert_eq!(std::env::var("JIMMY_TEST_C").unwrap(), "de afuera");
        std::env::remove_var("JIMMY_TEST_C");
        put(&BTreeMap::new());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn lo_que_sale_del_vault_sale_del_entorno() {
        let _env = ENV.lock().unwrap();
        let dir = temp("saca");
        let store = start(&dir).unwrap();
        store
            .lock()
            .unwrap()
            .set("demo", "dev", "JIMMY_TEST_D", "1", "test")
            .unwrap();
        let repo = repo(&dir, "demo");
        apply(&store, &repo);
        assert_eq!(std::env::var("JIMMY_TEST_D").unwrap(), "1");

        store
            .lock()
            .unwrap()
            .unset("demo", "dev", "JIMMY_TEST_D", "test")
            .unwrap();
        apply(&store, &repo);

        assert!(std::env::var_os("JIMMY_TEST_D").is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn un_vault_que_no_abre_avisa_por_qué() {
        let _env = ENV.lock().unwrap();
        let dir = temp("roto");
        std::fs::create_dir_all(dir.parent().unwrap()).unwrap();
        std::fs::write(&dir, "no soy un directorio").unwrap();

        let error = start(&dir).err().unwrap();

        assert!(error.contains("no pude crear"), "{error}");
        std::fs::remove_file(&dir).unwrap();
    }
}
