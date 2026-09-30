//! Los secretos de una org, sellados en la base.
//!
//! El control plane guarda las credenciales que necesita para trabajar por una
//! org —las de sus transports, por ejemplo— y no las deja legibles en el disco:
//! cada valor va sellado con una clave que sólo tiene el despliegue, y atado a
//! su nombre, así que mover una fila a otra org no devuelve el secreto, falla.
//! El esquema es el de heimdall, que es un binario: si algún día expone su
//! crypto como librería, esto se va.

use crate::store::Store;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key as CipherKey, XChaCha20Poly1305, XNonce};
use std::sync::Arc;

const NONCE: usize = 24;

/// Nombres que no pueden ser de un secreto: los del turno. Un secreto llamado
/// `OPENAI_API_KEY` se pondría delante del pase del modelo y el worker hablaría
/// con el proveedor como si fuera el control plane.
const RESERVADOS: [&str; 6] = [
    "OPENAI_API_KEY",
    "AXE_BASE",
    "AXE_MODEL",
    "JIMMY_ROOT",
    "JIMMY_WORKSPACE",
    "PATH",
];

#[derive(Clone)]
pub struct Key([u8; 32]);

impl Key {
    pub fn from_hex(text: &str) -> Result<Key, String> {
        let bytes = decode_hex(text)?;
        if bytes.len() != 32 {
            return Err(format!("la clave tiene {} bytes, espera 32", bytes.len()));
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&bytes);
        Ok(Key(key))
    }

    pub fn seal(&self, plain: &[u8], aad: &[u8]) -> Result<Vec<u8>, String> {
        let cipher = XChaCha20Poly1305::new(CipherKey::from_slice(&self.0));
        let mut nonce = [0u8; NONCE];
        getrandom::getrandom(&mut nonce).map_err(|e| format!("no hay azar: {e}"))?;
        let sealed = cipher
            .encrypt(XNonce::from_slice(&nonce), Payload { msg: plain, aad })
            .map_err(|_| "no pude cifrar".to_string())?;
        let mut blob = Vec::with_capacity(NONCE + sealed.len());
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&sealed);
        Ok(blob)
    }

    pub fn open(&self, blob: &[u8], aad: &[u8]) -> Result<Vec<u8>, String> {
        if blob.len() <= NONCE {
            return Err("el secreto está cortado".to_string());
        }
        let cipher = XChaCha20Poly1305::new(CipherKey::from_slice(&self.0));
        cipher
            .decrypt(
                XNonce::from_slice(&blob[..NONCE]),
                Payload {
                    msg: &blob[NONCE..],
                    aad,
                },
            )
            .map_err(|_| "no pude descifrar: ¿cambió la clave?".to_string())
    }
}

/// Los secretos de una org, con la clave del despliegue.
pub struct Secretos {
    key: Key,
    store: Arc<Store>,
}

impl Secretos {
    #[cfg(test)]
    pub fn new(key: Key, store: Arc<Store>) -> Secretos {
        Secretos { key, store }
    }

    /// Con la clave de la instancia. Sin `JIMMY_SECRETS_KEY` no hay secretos:
    /// es una función que se prende cuando el despliegue la configura.
    pub fn from_env(store: Arc<Store>) -> Option<Secretos> {
        let hex = crate::env("JIMMY_SECRETS_KEY")?;
        match Key::from_hex(&hex) {
            Ok(key) => Some(Secretos { key, store }),
            Err(error) => {
                eprintln!("jimmy: JIMMY_SECRETS_KEY no sirve: {error}");
                None
            }
        }
    }

    pub fn set(&self, org: &str, name: &str, value: &str) -> Result<(), String> {
        let name = nombre_valido(name)?;
        let blob = self.key.seal(value.as_bytes(), &etiqueta(org, &name))?;
        self.store.guardar_secreto(org, &name, &blob)
    }

    pub fn get(&self, org: &str, name: &str) -> Result<Option<String>, String> {
        let name = nombre_valido(name)?;
        let Some(blob) = self.store.leer_secreto(org, &name)? else {
            return Ok(None);
        };
        let abierto = self.key.open(&blob, &etiqueta(org, &name))?;
        String::from_utf8(abierto)
            .map(Some)
            .map_err(|_| "ese secreto no es texto".to_string())
    }

    pub fn borrar(&self, org: &str, name: &str) -> Result<(), String> {
        let name = nombre_valido(name)?;
        self.store.borrar_secreto(org, &name)
    }

    /// Los nombres, nunca los valores.
    pub fn nombres(&self, org: &str) -> Result<Vec<String>, String> {
        self.store.nombres_de_secretos(org)
    }

    /// Lo que el turno le pasa al sandbox: los secretos de esa org como
    /// variables de entorno. Los nombres reservados no entran, y si un secreto
    /// no abre se avisa y se sigue con el resto.
    pub fn entorno(&self, org: &str) -> Vec<(String, String)> {
        let Ok(nombres) = self.nombres(org) else {
            return Vec::new();
        };
        let mut entorno = Vec::new();
        for nombre in nombres {
            if RESERVADOS.contains(&nombre.as_str()) {
                eprintln!("jimmy: el secreto {nombre} no puede ir al turno: es del turno");
                continue;
            }
            match self.get(org, &nombre) {
                Ok(Some(valor)) => entorno.push((nombre, valor)),
                Ok(None) => {}
                Err(error) => eprintln!("jimmy: no pude abrir {nombre} de {org}: {error}"),
            }
        }
        entorno.sort();
        entorno
    }
}

/// El nombre es parte del sello: mover una fila a otra org y a otro nombre no
/// devuelve el secreto.
fn etiqueta(org: &str, name: &str) -> Vec<u8> {
    format!("{org}/{name}").into_bytes()
}

fn nombre_valido(name: &str) -> Result<String, String> {
    let name = name.trim().to_uppercase();
    if name.is_empty() {
        return Err("el secreto necesita un nombre".into());
    }
    let valido = name.starts_with(|ch: char| ch.is_ascii_alphabetic() || ch == '_')
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_');
    if !valido {
        return Err(format!("ese nombre no sirve para una variable: {name}"));
    }
    Ok(name)
}

fn decode_hex(text: &str) -> Result<Vec<u8>, String> {
    let text = text.trim();
    if !text.len().is_multiple_of(2) {
        return Err("el hexadecimal tiene largo impar".into());
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let mut i = 0;
    while i < bytes.len() {
        let par = std::str::from_utf8(&bytes[i..i + 2]).map_err(|_| "no es hexadecimal")?;
        out.push(u8::from_str_radix(par, 16).map_err(|_| "no es hexadecimal")?);
        i += 2;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dos orgs de verdad, una para cada lado: los secretos son de una org que
    /// existe. Cada prueba tiene su base, que si no se pisan entre ellas.
    fn secretos(tag: &str) -> (Secretos, String, String, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("jimmy-secretos-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Arc::new(Store::open(&dir.join("jimmy.db")).unwrap());
        let (_, una) = store.register("una@ejemplo.com").unwrap();
        let (_, otra) = store.register("otra@ejemplo.com").unwrap();
        let key = Key::from_hex(&"ab".repeat(32)).unwrap();
        (Secretos { key, store }, una.id, otra.id, dir)
    }

    #[test]
    fn un_secreto_vuelve_y_no_esta_en_la_base() {
        let (secretos, org, _, dir) = secretos("ida");
        secretos.set(&org, "SLACK_TOKEN", "xoxb-secreto").unwrap();
        assert_eq!(
            secretos.get(&org, "SLACK_TOKEN").unwrap().as_deref(),
            Some("xoxb-secreto")
        );
        assert_eq!(
            secretos.get(&org, "slack_token").unwrap().as_deref(),
            Some("xoxb-secreto"),
            "el nombre no distingue mayúsculas"
        );

        let crudo = std::fs::read(dir.join("jimmy.db")).unwrap();
        assert!(
            !crudo.windows(12).any(|w| w == b"xoxb-secreto"),
            "el valor quedó legible en la base"
        );
        assert_eq!(secretos.nombres(&org).unwrap(), ["SLACK_TOKEN"]);

        secretos.borrar(&org, "SLACK_TOKEN").unwrap();
        assert_eq!(secretos.get(&org, "SLACK_TOKEN").unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn un_secreto_es_de_su_org_y_de_su_nombre() {
        let (secretos, org, otra, dir) = secretos("dueno");
        secretos.set(&org, "CLAVE", "el-valor").unwrap();
        assert_eq!(secretos.get(&otra, "CLAVE").unwrap(), None);
        assert_eq!(secretos.get(&org, "OTRA").unwrap(), None);

        // La misma fila, movida a otra org, no abre: el sello dice dónde va.
        let fila = secretos.store.leer_secreto(&org, "CLAVE").unwrap().unwrap();
        secretos
            .store
            .guardar_secreto(&otra, "CLAVE", &fila)
            .unwrap();
        assert!(secretos.get(&otra, "CLAVE").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn con_otra_clave_no_abre() {
        let (secretos, org, _, dir) = secretos("clave");
        secretos.set(&org, "CLAVE", "el-valor").unwrap();
        let otra = Secretos {
            key: Key::from_hex(&"cd".repeat(32)).unwrap(),
            store: secretos.store.clone(),
        };
        assert!(otra.get(&org, "CLAVE").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn el_entorno_del_turno_no_deja_pisar_lo_del_turno() {
        let (secretos, org, otra, dir) = secretos("entorno");
        secretos.set(&org, "STRIPE_KEY", "sk-1").unwrap();
        secretos.set(&org, "AXE_BASE", "http://otro").unwrap();
        secretos.set(&org, "OPENAI_API_KEY", "la-de-otro").unwrap();
        secretos.set(&otra, "DE_OTRA", "no-va").unwrap();

        assert_eq!(
            secretos.entorno(&org),
            vec![("STRIPE_KEY".to_string(), "sk-1".to_string())],
            "sólo lo suyo, y nada que sea del turno"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn un_nombre_que_no_es_una_variable_no_entra() {
        let (secretos, org, _, dir) = secretos("nombres");
        assert!(secretos.set(&org, "con espacio", "x").is_err());
        assert!(secretos.set(&org, "1NUMERO", "x").is_err());
        assert!(secretos.set(&org, "", "x").is_err());
        assert!(secretos.set(&org, "bien_1", "x").is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn la_clave_es_de_32_bytes_en_hexadecimal() {
        assert!(Key::from_hex("ab").is_err());
        assert!(Key::from_hex(&"zz".repeat(32)).is_err());
        assert!(Key::from_hex(&"ab".repeat(32)).is_ok());
    }

    #[test]
    fn cada_sello_usa_su_propio_azar() {
        let key = Key::from_hex(&"ab".repeat(32)).unwrap();
        let uno = key.seal(b"x", b"donde").unwrap();
        let dos = key.seal(b"x", b"donde").unwrap();
        assert_ne!(uno, dos);
        assert_eq!(key.open(&uno, b"donde").unwrap(), b"x");
        assert!(key.open(&uno, b"otro").is_err());
        assert!(key.open(b"corto", b"donde").is_err());
    }
}
