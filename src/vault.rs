//! El vault: heimdall adentro del mismo binario.
//!
//! El store vive en el volumen, la master key se lee del entorno (o se genera
//! y queda en el volumen la primera vez) y el server escucha en loopback, sin
//! salir del proceso. `HEIMDALL_URL` y `HEIMDALL_TOKEN` apuntan ahí, así
//! `heimdall` y `doppler` le inyectan variables a un comando sin pasar por
//! ningún servicio de afuera.

use heimdall::crypto::{self, Key};
use heimdall::server::{self, Server};
use heimdall::store::{self, NewToken, Store};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

const DIR: &str = "heimdall";
const DB: &str = "heimdall.db";
const KEY_FILE: &str = "master.key";
const TOKEN: &str = "jimmy";
const PROJECT: &str = "*";
const ENV: &str = "dev";

/// Hay un vault por proceso y arranca una sola vez, con jimmy.
static VAULT: OnceLock<Vault> = OnceLock::new();

pub struct Vault {
    server: Option<Arc<Server>>,
    error: Option<String>,
}

/// Nunca falla: si el store no abre, el error queda guardado y la web lo
/// muestra. Un vault roto no puede dejar a jimmy sin arrancar.
pub fn open(root: &Path) {
    VAULT.get_or_init(|| match start(&root.join(DIR)) {
        Ok(server) => Vault {
            server: Some(server),
            error: None,
        },
        Err(e) => {
            eprintln!("jimmy: el vault no abrió: {e}");
            Vault {
                server: None,
                error: Some(e),
            }
        }
    });
}

/// El server del vault de este proceso, o nada si no abrió.
pub fn server() -> Option<&'static Arc<Server>> {
    VAULT.get()?.server.as_ref()
}

/// Por qué no hay vault, para poder decirlo.
pub fn message() -> &'static str {
    VAULT
        .get()
        .and_then(|vault| vault.error.as_deref())
        .unwrap_or("el vault no arrancó")
}

fn start(dir: &Path) -> Result<Arc<Server>, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("no pude crear {}: {e}", dir.display()))?;
    let store = Store::open(dir.join(DB), master_key(dir)?)?;
    let token = own_token(&store)?;
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|e| format!("no pude escuchar el vault: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("el vault no tiene dirección: {e}"))?
        .port();
    let server = Arc::new(Server {
        store: Mutex::new(store),
        admin: crypto::random_hex(24)?,
        emails: Vec::new(),
        mail: None,
        link_base: String::new(),
        dev: false,
    });
    std::env::set_var("HEIMDALL_URL", format!("http://127.0.0.1:{port}"));
    std::env::set_var("HEIMDALL_TOKEN", &token);
    eprintln!("jimmy: vault en http://127.0.0.1:{port} (alcance {PROJECT}/{ENV})");
    let served = server.clone();
    std::thread::spawn(move || server::serve(served, listener));
    Ok(server)
}

/// La master key sale del entorno, y si no está, del volumen: la primera vez se
/// genera y queda ahí. El entorno manda, porque es el que se puede rotar sin
/// tocar el volumen; que las dos existan y no coincidan es un error y no un
/// vault que no abre los secretos.
fn master_key(dir: &Path) -> Result<Key, String> {
    let path = dir.join(KEY_FILE);
    let from_env = crate::env("HEIMDALL_MASTER_KEY");
    let from_file = std::fs::read_to_string(&path)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty());
    let hex = match (from_env, from_file) {
        (Some(env), Some(file)) if env != file => {
            return Err(format!(
                "HEIMDALL_MASTER_KEY no es la de {}: sacá una de las dos",
                path.display()
            ))
        }
        (Some(env), _) => {
            std::env::remove_var("HEIMDALL_MASTER_KEY");
            env
        }
        (None, Some(file)) => file,
        (None, None) => persist_key(&path)?,
    };
    Key::from_hex(&hex)
}

fn persist_key(path: &Path) -> Result<String, String> {
    let hex = crypto::random_hex(32)?;
    std::fs::write(path, format!("{hex}\n"))
        .map_err(|e| format!("no pude escribir {}: {e}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("no pude proteger {}: {e}", path.display()))?;
    eprintln!("jimmy: master key nueva en {}", path.display());
    Ok(hex)
}

/// Un token nuevo por arranque. El valor en claro no queda en disco, así que el
/// de la corrida anterior se revoca: los comandos que corren ahora son los
/// únicos que lo tienen.
fn own_token(store: &Store) -> Result<String, String> {
    for token in store.tokens().map_err(text)? {
        if token.name == TOKEN {
            store.revoke(&token.id, TOKEN).map_err(text)?;
        }
    }
    let new = NewToken {
        name: TOKEN.to_string(),
        project: PROJECT.to_string(),
        env: ENV.to_string(),
        keys: None,
        admin: false,
        ttl: None,
    };
    store
        .create_token(new, TOKEN)
        .map(|(_, plain)| plain)
        .map_err(text)
}

pub fn text(error: store::Error) -> String {
    match error {
        store::Error::Bad(message) | store::Error::Internal(message) => message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Las dos pruebas tocan HEIMDALL_MASTER_KEY, que es del proceso entero.
    static ENV: Mutex<()> = Mutex::new(());

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("jimmy-vault-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn el_vault_guarda_un_secreto_y_le_da_el_token_a_los_comandos() {
        let _env = ENV.lock().unwrap();
        let dir = temp("arranque");
        std::env::set_var("HEIMDALL_MASTER_KEY", "ab".repeat(32));
        let server = start(&dir).unwrap();

        server
            .store
            .lock()
            .unwrap()
            .set("prueba", "dev", "CLAVE", "valor", "test")
            .unwrap();

        let url = format!(
            "{}/v1/secrets?project=prueba&env=dev",
            crate::env("HEIMDALL_URL").unwrap()
        );
        let answer: serde_json::Value = ureq::get(&url)
            .set(
                "Authorization",
                &format!("Bearer {}", crate::env("HEIMDALL_TOKEN").unwrap()),
            )
            .call()
            .unwrap()
            .into_json()
            .unwrap();
        assert_eq!(answer["CLAVE"], "valor");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn la_master_key_del_entorno_no_queda_para_los_hijos() {
        let _env = ENV.lock().unwrap();
        let dir = temp("entorno");
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("HEIMDALL_MASTER_KEY", "ab".repeat(32));

        master_key(&dir).unwrap();

        assert!(crate::env("HEIMDALL_MASTER_KEY").is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn una_master_key_que_no_es_la_del_volumen_se_avisa() {
        let _env = ENV.lock().unwrap();
        let dir = temp("choque");
        let path = dir.join(DIR);
        std::fs::create_dir_all(&path).unwrap();
        persist_key(&path.join(KEY_FILE)).unwrap();
        std::env::set_var("HEIMDALL_MASTER_KEY", "cd".repeat(32));

        let error = master_key(&path).err().unwrap();
        std::env::remove_var("HEIMDALL_MASTER_KEY");

        assert!(error.contains("no es la de"), "{error}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
