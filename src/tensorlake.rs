//! El sandbox de Tensorlake: dónde corre el trabajo de una org.
//!
//! El control plane no usa el SDK ni los bindings: habla la API con HTTP, que
//! es todo lo que hace falta para crear el sandbox, lanzarle un worker y
//! seguirle los eventos en vivo. El ciclo de vida vive en
//! `api.tensorlake.ai`, y el sandbox de cada org contesta en
//! `<nombre>.sandbox.tensorlake.ai`.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read};
use std::time::Duration;

const API: &str = "https://api.tensorlake.ai";
const PROXY: &str = ".sandbox.tensorlake.ai";
const PREFETCH_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct SandboxInfo {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub status: String,
}

#[derive(Deserialize)]
struct Created {
    sandbox_id: String,
    #[serde(default)]
    name: String,
    status: String,
}

#[derive(Deserialize)]
struct Listed {
    #[serde(default)]
    sandboxes: Vec<SandboxInfo>,
}

#[derive(Deserialize)]
struct Started {
    pid: i64,
}

/// Un evento de la salida de un proceso del sandbox. `line` viene como el
/// proceso la escribió, sin el `\n`.
#[derive(Deserialize)]
struct Output {
    line: String,
}

/// Una entrada de un directorio del filesystem de una org.
#[derive(Deserialize)]
struct FileEntry {
    name: String,
    #[serde(default)]
    is_dir: bool,
    #[serde(default)]
    size: u64,
}

#[derive(Deserialize)]
struct ListedFiles {
    #[serde(default)]
    entries: Vec<FileEntry>,
}

/// El path va en el query string, así que lo que no sea seguro se escapa.
fn escapar(path: &str) -> String {
    let mut out = String::new();
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

pub struct Tensorlake {
    key: String,
    namespace: String,
}

impl Tensorlake {
    pub fn new(key: &str, namespace: &str) -> Tensorlake {
        Tensorlake {
            key: key.to_string(),
            namespace: namespace.to_string(),
        }
    }

    /// Con lo que hay en el entorno. Sin clave no hay sandbox: el control
    /// plane sigue corriendo en local.
    pub fn from_env() -> Option<Tensorlake> {
        let key = crate::env("TENSORLAKE_API_KEY")?;
        let namespace = crate::env("TENSORLAKE_NAMESPACE").unwrap_or_else(|| "default".into());
        Some(Tensorlake::new(&key, &namespace))
    }

    fn sandboxes(&self) -> String {
        format!("{API}/v1/namespaces/{}/sandboxes", self.namespace)
    }

    fn proxy(&self, name: &str) -> String {
        format!("https://{name}{PROXY}/api/v1")
    }

    fn auth(&self, request: ureq::Request) -> ureq::Request {
        request.set("Authorization", &format!("Bearer {}", self.key))
    }

    /// El error de la API viene con el motivo adentro del cuerpo, así que se
    /// devuelve entero: sin eso, un 400 no dice qué campo estaba mal.
    fn finish(
        &self,
        result: Result<ureq::Response, ureq::Error>,
    ) -> Result<ureq::Response, String> {
        match result {
            Ok(response) => Ok(response),
            Err(ureq::Error::Status(code, response)) => {
                let body = response.into_string().unwrap_or_default();
                Err(format!("{code}: {}", body.trim()))
            }
            Err(error) => Err(error.to_string()),
        }
    }

    /// Los sandboxes del proyecto. La lista viene con retraso, así que no es
    /// la fuente de verdad de nada: sirve para ver qué quedó vivo.
    pub fn list(&self) -> Result<Vec<SandboxInfo>, String> {
        let response = self.finish(self.auth(ureq::get(&self.sandboxes())).call())?;
        let listed: Listed = response.into_json().map_err(|e| e.to_string())?;
        Ok(listed.sandboxes)
    }

    pub fn find(&self, name: &str) -> Result<Option<SandboxInfo>, String> {
        Ok(self.list()?.into_iter().find(|box_| box_.name == name))
    }

    /// Crear el sandbox de una org, con su filesystem montado.
    pub fn create(
        &self,
        name: &str,
        image: &str,
        mount: &str,
        fs: &str,
    ) -> Result<SandboxInfo, String> {
        let body = serde_json::json!({
            "name": name,
            "image": image,
            "cpus": 1,
            "memory_mb": 1024,
            "timeout_secs": 60,
            "file_systems": [{
                "file_system_id": fs,
                "mount_path": mount,
                "prefetch": true,
            }],
        });
        let url = self.sandboxes();
        let response = self.finish(self.auth(ureq::post(&url)).send_json(body))?;
        let created: Created = response.into_json().map_err(|e| e.to_string())?;
        Ok(SandboxInfo {
            id: created.sandbox_id,
            name: created.name,
            status: created.status,
        })
    }

    pub fn terminate(&self, id: &str) -> Result<(), String> {
        let url = format!("{}/{id}", self.sandboxes());
        self.finish(self.auth(ureq::delete(&url)).call())?;
        Ok(())
    }

    /// Dormirlo sin bajarlo: el filesystem y los procesos quedan como estaban.
    pub fn suspend(&self, id: &str) -> Result<(), String> {
        let url = format!("{}/{id}/suspend", self.sandboxes());
        self.finish(self.auth(ureq::post(&url)).call())?;
        Ok(())
    }

    pub fn resume(&self, id: &str) -> Result<(), String> {
        let url = format!("{}/{id}/resume", self.sandboxes());
        self.finish(self.auth(ureq::post(&url)).call())?;
        Ok(())
    }

    /// Lanzar un proceso con la entrada por pipe y la salida guardada: es lo
    /// que hace falta para hablarle como si fuera un pipe de la máquina.
    pub fn start(
        &self,
        sandbox: &str,
        command: &str,
        args: &[String],
        env: &BTreeMap<String, String>,
        dir: &str,
    ) -> Result<i64, String> {
        let body = serde_json::json!({
            "command": command,
            "args": args,
            "env": env,
            "working_dir": dir,
            "stdin_mode": "pipe",
            "stdout_mode": "capture",
        });
        let url = format!("{}/processes", self.proxy(sandbox));
        let response = self.finish(self.auth(ureq::post(&url)).send_json(body))?;
        let started: Started = response.into_json().map_err(|e| e.to_string())?;
        Ok(started.pid)
    }

    pub fn write_stdin(&self, sandbox: &str, pid: i64, bytes: &[u8]) -> Result<(), String> {
        let url = format!("{}/processes/{pid}/stdin", self.proxy(sandbox));
        let request = ureq::post(&url).set("Content-Type", "application/octet-stream");
        self.finish(self.auth(request).send_bytes(bytes))?;
        Ok(())
    }

    pub fn close_stdin(&self, sandbox: &str, pid: i64) -> Result<(), String> {
        let url = format!("{}/processes/{pid}/stdin/close", self.proxy(sandbox));
        self.finish(self.auth(ureq::post(&url)).send_string(""))?;
        Ok(())
    }

    /// La salida del proceso a medida que llega, línea por línea, hasta que el
    /// proceso termina.
    pub fn follow(
        &self,
        sandbox: &str,
        pid: i64,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<(), String> {
        let url = format!("{}/processes/{pid}/stdout/follow", self.proxy(sandbox));
        let response = self.finish(self.auth(ureq::get(&url)).call())?;
        let mut reader = BufReader::new(response.into_reader());
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => return Ok(()),
                Ok(_) => {}
                Err(error) => return Err(error.to_string()),
            }
            let line = line.trim_end();
            let Some(data) = line.strip_prefix("data: ") else {
                continue;
            };
            if data == "{}" {
                return Ok(());
            }
            match serde_json::from_str::<Output>(data) {
                Ok(output) => on_line(&output.line),
                Err(_) => return Err(format!("no entiendo la salida del sandbox: {data}")),
            }
        }
    }

    pub fn kill(&self, sandbox: &str, pid: i64) -> Result<(), String> {
        let url = format!("{}/processes/{pid}", self.proxy(sandbox));
        self.finish(self.auth(ureq::delete(&url)).call())?;
        Ok(())
    }

    pub fn signal(&self, sandbox: &str, pid: i64, signal: i32) -> Result<(), String> {
        let url = format!("{}/processes/{pid}/signal", self.proxy(sandbox));
        let body = serde_json::json!({ "signal": signal });
        self.finish(self.auth(ureq::post(&url)).send_json(body))?;
        Ok(())
    }

    /// El archivo entero: la API no sabe de rangos, así que un pedazo lo pide
    /// un comando de adentro o se corta acá.
    pub fn read_file(&self, sandbox: &str, path: &str) -> Result<Vec<u8>, String> {
        let url = format!("{}/files?path={}", self.proxy(sandbox), escapar(path));
        let response = self.finish(self.auth(ureq::get(&url)).call())?;
        let mut bytes = Vec::new();
        response
            .into_reader()
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        Ok(bytes)
    }

    /// Escribe el archivo y sus directorios, si no están.
    pub fn write_file(&self, sandbox: &str, path: &str, bytes: &[u8]) -> Result<(), String> {
        let url = format!("{}/files?path={}", self.proxy(sandbox), escapar(path));
        let request = ureq::put(&url).set("Content-Type", "application/octet-stream");
        self.finish(self.auth(request).send_bytes(bytes))?;
        Ok(())
    }

    pub fn list_files(
        &self,
        sandbox: &str,
        path: &str,
    ) -> Result<Vec<axe::machine::Entry>, String> {
        let url = format!("{}/files/list?path={}", self.proxy(sandbox), escapar(path));
        let response = self.finish(self.auth(ureq::get(&url)).call())?;
        let listed: ListedFiles = response.into_json().map_err(|e| e.to_string())?;
        Ok(listed
            .entries
            .into_iter()
            .map(|entry| axe::machine::Entry {
                name: entry.name,
                is_dir: entry.is_dir,
                size: entry.size,
            })
            .collect())
    }

    pub fn remove_file(&self, sandbox: &str, path: &str) -> Result<(), String> {
        let url = format!("{}/files?path={}", self.proxy(sandbox), escapar(path));
        self.finish(self.auth(ureq::delete(&url)).call())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Contra la API de verdad: crea un sandbox, le habla como si fuera un
    /// pipe de esta máquina y lee la salida en vivo. Crea recursos, así que
    /// corre a mano y con la clave puesta:
    ///
    ///     heimdall run -p jimmy -c dev -- cargo test --bin jimmy -- --ignored el_sandbox_habla
    #[test]
    #[ignore]
    fn el_sandbox_habla_por_pipes() {
        let Some(sandbox) = Tensorlake::from_env() else {
            panic!("falta TENSORLAKE_API_KEY");
        };
        let sandbox = Arc::new(sandbox);
        let name = format!("prueba-{}", crate::random::hex(4));
        let image = std::env::var("TENSORLAKE_IMAGE").unwrap_or_else(|_| "jimmy-min".into());

        let creado = sandbox
            .create(&name, &image, "/work", "poc-workspace")
            .unwrap();
        assert_eq!(creado.status, "running", "{creado:?}");

        let script = "echo listo; while IFS= read -r l; do echo eco:$l; done";
        let pid = sandbox
            .start(
                &name,
                "/bin/sh",
                &["-c".into(), script.into()],
                &BTreeMap::new(),
                "/work",
            )
            .unwrap();

        let visto = Arc::new(Mutex::new(Vec::new()));
        let anotador = visto.clone();
        let leyendo = sandbox.clone();
        let cual = name.clone();
        let hilo = std::thread::spawn(move || {
            leyendo
                .follow(&cual, pid, &mut |line| {
                    anotador.lock().unwrap().push(line.to_string())
                })
                .unwrap();
        });

        std::thread::sleep(Duration::from_secs(2));
        sandbox.write_stdin(&name, pid, b"uno\n").unwrap();
        sandbox.write_stdin(&name, pid, b"dos\n").unwrap();
        sandbox.close_stdin(&name, pid).unwrap();
        hilo.join().unwrap();

        let visto = visto.lock().unwrap().clone();
        assert_eq!(visto, vec!["listo", "eco:uno", "eco:dos"], "{visto:?}");

        sandbox.terminate(&creado.id).unwrap();
    }

    /// Los archivos de una org por la API: escribir, leer, listar y borrar.
    /// Crea recursos, así que corre a mano con la clave puesta:
    ///
    ///     heimdall run -p jimmy -c dev -- cargo test --bin jimmy -- --ignored el_filesystem_de_una_org
    #[test]
    #[ignore]
    fn el_filesystem_de_una_org() {
        let Some(credencial) = Tensorlake::from_env() else {
            panic!("falta TENSORLAKE_API_KEY");
        };
        let name = format!("archivos-{}", crate::random::hex(4));
        let image = std::env::var("TENSORLAKE_IMAGE").unwrap_or_else(|_| "jimmy-min".into());
        let creado = credencial
            .create(&name, &image, "/work", "jimmy-org")
            .unwrap();
        assert_eq!(creado.status, "running", "{creado:?}");

        let path = "/work/notas/prueba.txt";
        credencial.write_file(&name, path, b"hola\n").unwrap();
        assert_eq!(credencial.read_file(&name, path).unwrap(), b"hola\n");

        let entradas = credencial.list_files(&name, "/work/notas").unwrap();
        assert_eq!(entradas.len(), 1, "{entradas:?}");
        assert_eq!(entradas[0].name, "prueba.txt");
        assert_eq!(entradas[0].size, 5);
        assert!(!entradas[0].is_dir);

        credencial.remove_file(&name, path).unwrap();
        assert!(credencial.read_file(&name, path).is_err());

        credencial.terminate(&creado.id).unwrap();
    }
}
