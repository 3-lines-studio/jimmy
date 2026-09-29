//! La máquina de una org: el volumen y el shell del sandbox, del otro lado.
//!
//! Es lo que le hace falta a las herramientas de axe para trabajar en el
//! workspace de una org sin saber que está lejos. Los archivos y los comandos
//! van por el mismo cliente, así que el path que una herramienta lee es el
//! mismo que ve un comando.

use crate::store::Store;
use crate::tensorlake::{SandboxInfo, Tensorlake, MOUNT};
use axe::machine::{resolve, Entry, Machine};
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub struct Remote {
    cliente: Arc<Tensorlake>,
    sandbox: String,
    dir: String,
}

impl Remote {
    pub fn new(cliente: Arc<Tensorlake>, sandbox: &str, dir: &str) -> Remote {
        Remote {
            cliente,
            sandbox: sandbox.to_string(),
            dir: dir.to_string(),
        }
    }
}

/// La imagen del sandbox: la del despliegue, no la de una org. Va sin jimmy
/// adentro, así no hay que reconstruirla por un cambio de código.
fn image() -> String {
    crate::env("TENSORLAKE_IMAGE").unwrap_or_else(|| "jimmy-min".into())
}

/// La huella del binario: es la versión del agente que corre adentro del
/// sandbox. Cambia si y sólo si cambia el binario, así que no hay forma de que
/// un turno corra con el de antes.
fn huella(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

/// El path de este lado, del otro: la raíz de la org es donde el sandbox monta
/// su volumen, así que adentro todo cuelga de la misma forma. El path que una
/// herramienta escribe y el que ve un comando siguen siendo el mismo.
pub fn al_sandbox(root: &Path, path: &Path) -> String {
    match path.strip_prefix(root) {
        Ok(resto) if resto.as_os_str().is_empty() => MOUNT.to_string(),
        Ok(resto) => format!("{MOUNT}/{}", resto.display()),
        Err(_) => path.display().to_string(),
    }
}

/// Las versiones que ya están en el volumen de ese sandbox, para no preguntar
/// por la API en cada turno. La subida es idempotente, así que una instancia
/// que no se acuerde no rompe nada: sube de nuevo.
static SUBIDO: Mutex<Option<HashSet<String>>> = Mutex::new(None);

fn ya_esta(sandbox: &str, huella: &str) -> bool {
    let mut subido = SUBIDO.lock().unwrap();
    subido
        .get_or_insert_with(HashSet::new)
        .contains(&format!("{sandbox}/{huella}"))
}

fn recordar(sandbox: &str, huella: &str) {
    let mut subido = SUBIDO.lock().unwrap();
    subido
        .get_or_insert_with(HashSet::new)
        .insert(format!("{sandbox}/{huella}"));
}

/// El agente adentro del sandbox: el binario y lo que lee del disco —los
/// prompts y las skills—, en el volumen de la org. Se sube una vez por versión
/// y el turno corre de ahí, así que el sandbox es el mismo agente y no una
/// parte: los archivos que toca son los del volumen y no hay viajes por HTTP.
pub fn publicar(cliente: &Tensorlake, sandbox: &str, exe: &Path) -> Result<String, String> {
    let bytes = std::fs::read(exe).map_err(|e| format!("no pude leer {exe:?}: {e}"))?;
    let huella = huella(&bytes);
    let dir = format!("{MOUNT}/.jimmy/{huella}");
    let path = format!("{dir}/jimmy");
    if ya_esta(sandbox, &huella) {
        return Ok(path);
    }
    if cliente.list_files(sandbox, &dir).is_err() {
        cliente.write_file(sandbox, &path, &bytes)?;
        cliente.run(
            sandbox,
            MOUNT,
            &format!("chmod 0755 {path}"),
            60,
            &mut |_| {},
        )?;
        for (local, destino) in contenido()? {
            let archivo = std::fs::read(&local).map_err(|e| format!("{local:?}: {e}"))?;
            cliente.write_file(sandbox, &format!("{MOUNT}/{destino}"), &archivo)?;
        }
    }
    recordar(sandbox, &huella);
    Ok(path)
}

/// Los archivos que el agente lee del disco y no están adentro del binario: los
/// prompts y las skills, que de este lado viven en la imagen y del otro tienen
/// que estar en el volumen.
fn contenido() -> Result<Vec<(std::path::PathBuf, String)>, String> {
    let mut archivos = Vec::new();
    for (dir, nombre) in [
        (crate::prompt::BUILTIN, "prompts"),
        (crate::skill::BUILTIN, "skills"),
    ] {
        let raiz = Path::new(dir);
        if !raiz.is_dir() {
            continue;
        }
        juntar(raiz, &mut |relativo| {
            archivos.push((raiz.join(&relativo), format!("{nombre}/{relativo}")));
        })?;
    }
    Ok(archivos)
}

fn juntar(dir: &Path, con: &mut dyn FnMut(String)) -> Result<(), String> {
    let entradas = std::fs::read_dir(dir).map_err(|e| format!("{dir:?}: {e}"))?;
    for entrada in entradas {
        let entrada = entrada.map_err(|e| e.to_string())?;
        let nombre = entrada.file_name().to_string_lossy().to_string();
        let path = entrada.path();
        if path.is_dir() {
            juntar(&path, &mut |relativo| con(format!("{nombre}/{relativo}")))?;
        } else {
            con(nombre);
        }
    }
    Ok(())
}

/// La máquina de una org lista para el turno: sin fila en `machines` el trabajo
/// corre acá y no hay nada que despertar; con fila, el sandbox se crea si no
/// está y se despierta si está dormido.
pub fn ensure(org: &str, store: &Store) -> Result<Option<SandboxInfo>, String> {
    let Some(row) = store.machine(org)? else {
        return Ok(None);
    };
    if row.provider != "tensorlake" {
        return Err(format!("no sé hablar con el proveedor {}", row.provider));
    }
    let cliente = Tensorlake::from_env().ok_or("falta TENSORLAKE_API_KEY")?;
    let mut listo = cliente.ensure(&row.sandbox, &image(), &row.file_system)?;
    if listo.name.is_empty() {
        listo.name = row.sandbox;
    }
    Ok(Some(listo))
}

/// La máquina de una org, que ya está despierta: sus archivos y sus comandos
/// van por el mismo cliente, así que las herramientas no saben que está lejos.
/// Hoy el turno no la usa —el worker corre adentro del sandbox, con su volumen
/// montado—: es lo que la web necesita para leer y escribir lo de una org sin
/// despertarla.
pub fn de_la_org(sandbox: &str, dir: &str) -> Result<Arc<dyn Machine>, String> {
    let cliente = Tensorlake::from_env().ok_or("falta TENSORLAKE_API_KEY")?;
    Ok(Arc::new(Remote::new(Arc::new(cliente), sandbox, dir)))
}

impl Machine for Remote {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        self.cliente
            .read_file(&self.sandbox, &resolve(&self.dir, path))
    }

    fn write(&self, path: &str, bytes: &[u8]) -> Result<(), String> {
        self.cliente
            .write_file(&self.sandbox, &resolve(&self.dir, path), bytes)
    }

    fn list(&self, path: &str) -> Result<Vec<Entry>, String> {
        self.cliente
            .list_files(&self.sandbox, &resolve(&self.dir, path))
    }

    fn remove(&self, path: &str) -> Result<(), String> {
        self.cliente
            .remove_file(&self.sandbox, &resolve(&self.dir, path))
    }

    fn run(&self, command: &str, timeout: u64, progress: &mut dyn FnMut(&str)) -> String {
        match self
            .cliente
            .run(&self.sandbox, &self.dir, command, timeout, progress)
        {
            Ok(texto) => texto,
            Err(error) => format!("error: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axe::machine::Local;

    fn store(nombre: &str) -> Store {
        let dir =
            std::env::temp_dir().join(format!("jimmy-remote-{nombre}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let _ = std::fs::remove_file(dir.join("jimmy.db"));
        Store::open(&dir.join("jimmy.db")).unwrap()
    }

    /// Una org sin fila en `machines` trabaja acá: no hay nada que despertar ni
    /// a quién pedirle el filesystem.
    #[test]
    fn una_org_sin_maquina_no_tiene_sandbox() {
        let store = store("sin-maquina");
        let (_, org) = store.register("don@ejemplo.com").unwrap();
        assert_eq!(ensure(&org.id, &store).unwrap(), None);
    }

    /// El path de una herramienta del otro lado: la raíz de la org es donde el
    /// sandbox monta su volumen.
    #[test]
    fn los_paths_de_la_org_se_traducen_al_volumen() {
        let raiz = Path::new("/data/orgs/01ABC");
        assert_eq!(al_sandbox(raiz, raiz), "/work");
        assert_eq!(
            al_sandbox(raiz, Path::new("/data/orgs/01ABC/workspace")),
            "/work/workspace"
        );
        assert_eq!(
            al_sandbox(
                raiz,
                Path::new("/data/orgs/01ABC/chats/uno/transcript.jsonl")
            ),
            "/work/chats/uno/transcript.jsonl"
        );
        assert_eq!(
            al_sandbox(Path::new("/data"), Path::new("/data/workspace")),
            "/work/workspace"
        );
    }

    #[test]
    fn la_huella_cambia_con_un_byte() {
        assert_eq!(huella(b"uno"), huella(b"uno"));
        assert_ne!(huella(b"uno"), huella(b"dos"));
    }

    /// El proveedor que no conozco se avisa antes de tocar la red: preferimos
    /// que el turno falle a que trabaje en un lugar que no es el suyo.
    #[test]
    fn un_proveedor_desconocido_es_un_error() {
        let store = store("proveedor");
        let (_, org) = store.register("don@ejemplo.com").unwrap();
        store.set_machine(&org.id, "otro", "caja", "fs").unwrap();
        assert!(ensure(&org.id, &store).unwrap_err().contains("otro"));
    }

    /// El camino entero contra la API de verdad: la fila de `machines` despierta
    /// la máquina de la org y las herramientas trabajan del otro lado.
    ///
    ///     heimdall run -p jimmy -c dev -- cargo test --bin jimmy -- --ignored la_maquina_de_la_org
    #[test]
    #[ignore]
    fn la_maquina_de_la_org_se_despierta_y_trabaja() {
        let store = store("despierta");
        let (_, org) = store.register("don@ejemplo.com").unwrap();
        store
            .set_machine(&org.id, "tensorlake", "turno-remoto", "jimmy-org")
            .unwrap();

        let sandbox = ensure(&org.id, &store).unwrap().unwrap();
        assert_eq!(sandbox.name, "turno-remoto");
        let cliente = Arc::new(Tensorlake::from_env().unwrap());
        let _guardado = Guardado(cliente.clone(), sandbox.id.clone());

        let maquina = de_la_org(&sandbox.name, "/work").unwrap();
        maquina.write("de-la-org.txt", b"hola\n").unwrap();
        assert_eq!(maquina.read("de-la-org.txt").unwrap(), b"hola\n");
        assert!(maquina
            .list("/work")
            .unwrap()
            .iter()
            .any(|entry| entry.name == "de-la-org.txt"));
        let salida = maquina.run("cat de-la-org.txt", 30, &mut |_| {});
        assert!(salida.contains("hola"), "{salida}");
        maquina.remove("de-la-org.txt").unwrap();
        assert!(maquina.read("de-la-org.txt").is_err());
    }

    /// Baja el sandbox aunque el test falle a mitad de camino: el plan de
    /// prueba deja uno solo a la vez.
    struct Guardado(Arc<Tensorlake>, String);

    impl Drop for Guardado {
        fn drop(&mut self) {
            let _ = self.0.terminate(&self.1);
        }
    }

    /// La compuerta: el mismo comando por la máquina local y por la remota, y
    /// el mismo texto. Crea recursos, así que corre a mano con la clave:
    ///
    ///     heimdall run -p jimmy -c dev -- cargo test --bin jimmy -- --ignored el_bash_remoto_se_comporta_igual
    #[test]
    #[ignore]
    fn el_bash_remoto_se_comporta_igual() {
        let Some(cliente) = Tensorlake::from_env() else {
            panic!("falta TENSORLAKE_API_KEY");
        };
        let cliente = Arc::new(cliente);
        let name = format!("igual-{}", crate::random::hex(4));
        let image = std::env::var("TENSORLAKE_IMAGE").unwrap_or_else(|_| "jimmy-min".into());
        let creado = cliente.create(&name, &image, "/work", "jimmy-org").unwrap();
        assert_eq!(creado.status, "running", "{creado:?}");
        let _guardado = Guardado(cliente.clone(), creado.id.clone());

        let dir = std::env::temp_dir().join(format!("axe-igual-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let escrito = dir.join("nota.txt");
        std::fs::write(&escrito, "hola\n").unwrap();
        cliente
            .write_file(&name, "/work/nota.txt", b"hola\n")
            .unwrap();

        let local = Local::new(&dir.display().to_string());
        let remoto = Remote::new(cliente.clone(), &name, "/work");

        remoto.write("relativo.txt", b"hola\n").unwrap();
        assert_eq!(remoto.read("relativo.txt").unwrap(), b"hola\n");
        let en_la_raiz = remoto.list("/").unwrap();
        assert!(
            !en_la_raiz.iter().any(|entry| entry.name == "relativo.txt"),
            "un path relativo cayó en la raíz: {en_la_raiz:?}"
        );
        assert_eq!(remoto.read("/work/relativo.txt").unwrap(), b"hola\n");
        remoto.remove("relativo.txt").unwrap();
        assert!(remoto.read("relativo.txt").is_err());

        for (comando, timeout) in [
            ("echo hola", 30),
            ("echo hola; echo al error >&2", 30),
            ("cat nota.txt", 30),
            ("exit 7", 30),
            ("sleep 30", 2),
        ] {
            let de_local = local.run(comando, timeout, &mut |_| {});
            let de_remoto = remoto.run(comando, timeout, &mut |_| {});
            assert_eq!(de_remoto, de_local, "«{comando}» contestó distinto");
        }

        let grande = "head -c 20000 /dev/zero | tr '\\0' 'x'";
        let de_local = local.run(grande, 30, &mut |_| {});
        let de_remoto = remoto.run(grande, 30, &mut |_| {});
        assert!(de_local.contains("Output truncated"), "{de_local}");
        assert!(de_remoto.contains("Output truncated"), "{de_remoto}");
        let cuerpo = |texto: &str| texto.chars().take(16000).collect::<String>();
        assert_eq!(
            cuerpo(&de_remoto),
            cuerpo(&de_local),
            "el cuerpo del truncado"
        );

        let lento = "for i in 1 2 3; do echo $i; sleep 0.4; done";
        let mut del_local = Vec::new();
        let mut del_remoto = Vec::new();
        let de_local = local.run(lento, 30, &mut |p| del_local.push(p.to_string()));
        let de_remoto = remoto.run(lento, 30, &mut |p| del_remoto.push(p.to_string()));
        assert_eq!(de_remoto, de_local);
        assert!(del_local.len() > 1, "el local no progresó: {del_local:?}");
        assert!(
            del_remoto.len() > 1,
            "el remoto no progresó: {del_remoto:?}"
        );

        remoto.run("sleep 3; echo tarde > /work/tarde.txt", 1, &mut |_| {});
        std::thread::sleep(std::time::Duration::from_secs(5));
        assert!(
            remoto.read("/work/tarde.txt").is_err(),
            "un hijo del comando cortado siguió vivo"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
