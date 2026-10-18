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
    declare(held, &project, &env);
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

/// El proyecto que usa un chat queda declarado, aunque todavía no tenga ni una
/// clave: así el panel lo muestra como vault vacío en vez de no mostrarlo.
fn declare(held: &Mutex<Store>, project: &str, env: &str) {
    if project.is_empty() {
        return;
    }
    let key = format!("{project}/{env}");
    let store = held.lock().unwrap_or_else(|e| e.into_inner());
    let known = store
        .names()
        .map(|names| names.iter().any(|name| name == &key))
        .unwrap_or(true);
    if !known {
        let _ = store.create_environment(project, env, "worker");
    }
}

/// Sin yaml ni variable de entorno, el proyecto sale del workspace: lo que está
/// adentro de `projects/<nombre>` es el proyecto `<nombre>`.
fn project_of(cwd: &Path) -> Option<String> {
    let parts: Vec<&str> = cwd.iter().filter_map(|part| part.to_str()).collect();
    let name = parts.get(parts.iter().position(|part| *part == "projects")? + 1)?;
    let valid = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    valid.then(|| name.to_string())
}

fn scope(cwd: &Path) -> (String, String) {
    let (yaml_project, yaml_env) = cli::setup_at(cwd).unwrap_or_default();
    let project = crate::env("HEIMDALL_PROJECT")
        .or_else(|| Some(yaml_project).filter(|project| !project.is_empty()))
        .or_else(|| project_of(cwd))
        .unwrap_or_default();
    let env = crate::env("HEIMDALL_ENV")
        .or_else(|| Some(yaml_env).filter(|env| !env.is_empty()))
        .unwrap_or_else(|| "dev".to_string());
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
    fn el_proyecto_sale_del_workspace_sin_yaml() {
        assert_eq!(
            scope(Path::new("/data/workspace/projects/jimmy")).0,
            "jimmy"
        );
        assert_eq!(
            scope(Path::new("/data/workspace/projects/jimmy/src")).0,
            "jimmy"
        );
        assert_eq!(scope(Path::new("/data/workspace/files")).0, "");
        assert_eq!(scope(Path::new("/data/workspace/projects/Malo")).0, "");
        assert_eq!(scope(Path::new("/tmp/projects/heimdall")).0, "heimdall");
    }

    #[test]
    fn el_yaml_le_gana_al_workspace() {
        let _env = ENV.lock().unwrap();
        let dir = temp("yaml-gana");
        let repo = repo(&dir, "otro");

        assert_eq!(scope(&repo), ("otro".to_string(), "dev".to_string()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn un_proyecto_sin_claves_queda_declarado() {
        let _env = ENV.lock().unwrap();
        let dir = temp("declara");
        let store = start(&dir).unwrap();
        let repo = repo(&dir, "nuevo");

        apply(&store, &repo);

        let names = store.lock().unwrap().names().unwrap();
        assert!(names.iter().any(|name| name == "nuevo/dev"), "{names:?}");
        assert_eq!(
            store.lock().unwrap().secrets("nuevo", "dev").unwrap().len(),
            0
        );
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
