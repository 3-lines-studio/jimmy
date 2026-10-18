use heimdall::store::{self, Store};
use std::path::Path;
use std::sync::{Mutex, OnceLock};

const DIR: &str = "heimdall";
const VAR: &str = "HEIMDALL_DIR";

static VAULT: OnceLock<Result<Mutex<Store>, String>> = OnceLock::new();

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
