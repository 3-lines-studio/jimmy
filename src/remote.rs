//! La máquina de una org: el volumen y el shell del sandbox, del otro lado.
//!
//! Es lo que le hace falta a las herramientas de axe para trabajar en el
//! workspace de una org sin saber que está lejos. Los archivos y los comandos
//! van por el mismo cliente, así que el path que una herramienta lee es el
//! mismo que ve un comando.

use crate::conversations;
use crate::files;
use crate::log::Log;
use crate::log::Window;
use crate::media;
use crate::store::Store;
use crate::tensorlake::{SandboxInfo, Tensorlake, MOUNT};
use crate::workspace::{Conversation, Place, Project, Workspace};
use axe::machine::{resolve, Entry, Machine};
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Los paths del agente adentro del sandbox: los mismos que usa el control
/// plane, así lo que el modelo corre por bash es lo mismo de los dos lados.
const BIN: &str = "/usr/local/bin";
const SHARE: &str = "/usr/local/share/jimmy";

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

/// La imagen del sandbox: el entorno del agente, no una org. Va sin el agente
/// adentro —el binario llega publicado, en la versión que corresponde— y se
/// reconstruye sólo cuando cambia el entorno, no el código.
pub(crate) fn image() -> String {
    crate::env("TENSORLAKE_IMAGE").unwrap_or_else(|| "jimmy-entorno".into())
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

/// Lo que ya se hizo en un sandbox, para no preguntar por la API en cada
/// turno: lo publicado (una vez por versión, en el volumen, que sobrevive) y lo
/// instalado (una vez por sandbox, en su disco, que no). Las dos cosas son
/// idempotentes: una instancia que no se acuerde las repite y no rompe nada.
static HECHO: Mutex<Option<HashSet<String>>> = Mutex::new(None);

fn ya_hecho(clave: &str) -> bool {
    let mut hecho = HECHO.lock().unwrap();
    hecho.get_or_insert_with(HashSet::new).contains(clave)
}

fn recordar(clave: &str) {
    let mut hecho = HECHO.lock().unwrap();
    hecho
        .get_or_insert_with(HashSet::new)
        .insert(clave.to_string());
}

/// El agente adentro del sandbox: el binario, los prompts, las skills y los
/// CLIs que el modelo corre por bash, en los mismos paths que de este lado, así
/// adentro se usa igual que acá y no hay dos agentes. La copia queda en el
/// volumen —una vez por versión, con la huella del binario como nombre— y cada
/// sandbox nuevo la instala en su disco desde ahí, que es lo que hace que un
/// sandbox recién creado tenga el agente listo sin subir nada de nuevo.
pub fn publicar(cliente: &Tensorlake, sandbox: &SandboxInfo, exe: &Path) -> Result<String, String> {
    let bytes = std::fs::read(exe).map_err(|e| format!("no pude leer {exe:?}: {e}"))?;
    let huella = huella(&bytes);
    let copia = format!("{MOUNT}/.jimmy/{huella}");
    let publicado = format!("publicado:{}:{huella}", sandbox.name);
    if !ya_hecho(&publicado) {
        if !completo(cliente, &sandbox.name, &copia, &bytes)? {
            cliente.write_file(&sandbox.name, &format!("{copia}/bin/jimmy"), &bytes)?;
            for (local, destino) in archivos()? {
                let datos = std::fs::read(&local).map_err(|e| format!("{local:?}: {e}"))?;
                cliente.write_file(&sandbox.name, &format!("{copia}/{destino}"), &datos)?;
            }
        }
        recordar(&publicado);
    }
    let instalado = format!("instalado:{}:{huella}", sandbox.id);
    if !ya_hecho(&instalado) {
        cliente.run(&sandbox.name, MOUNT, &instalar(&copia), 120, &mut |_| {})?;
        recordar(&instalado);
    }
    Ok(format!("{BIN}/jimmy"))
}

/// De la copia del volumen a los paths del sistema, que son los mismos que usa
/// el control plane: `/usr/local/bin/jimmy`, `/usr/local/share/jimmy/{prompts,skills}`.
fn instalar(copia: &str) -> String {
    format!(
        "set -e; mkdir -p {BIN} {SHARE}/prompts {SHARE}/skills; \
         cp -a {copia}/bin/. {BIN}/; \
         cp -a {copia}/share/prompts/. {SHARE}/prompts/; \
         cp -a {copia}/share/skills/. {SHARE}/skills/; \
         chmod 0755 {BIN}/*"
    )
}

/// Lo que el agente lee del disco y no está adentro del binario: los prompts,
/// las skills y los CLIs. De este lado viven en la imagen del control plane y
/// del otro tienen que estar en el sandbox.
/// Lo publicado está entero o no está: el binario con su tamaño y los CLIs que
/// van con él. Una subida que se corta deja la carpeta a medias, y el sandbox
/// arrancaría sin agente; el directorio no alcanza como señal.
fn completo(
    cliente: &Tensorlake,
    sandbox: &str,
    copia: &str,
    binario: &[u8],
) -> Result<bool, String> {
    let entradas = match cliente.list_files(sandbox, &format!("{copia}/bin")) {
        Ok(entradas) => entradas,
        Err(_) => return Ok(false),
    };
    let clis = archivos()?
        .iter()
        .filter(|(_, destino)| destino.starts_with("bin/"))
        .count();
    let servidor = entradas.iter().any(|entrada| {
        !entrada.is_dir && entrada.name == "jimmy" && entrada.size == binario.len() as u64
    });
    Ok(servidor && entradas.len() == clis + 1)
}

fn archivos() -> Result<Vec<(PathBuf, String)>, String> {
    let mut archivos = Vec::new();
    for (dir, destino) in [
        (crate::prompt::BUILTIN, "share/prompts"),
        (crate::skill::BUILTIN, "share/skills"),
    ] {
        let raiz = Path::new(dir);
        if !raiz.is_dir() {
            continue;
        }
        juntar(raiz, &mut |relativo| {
            archivos.push((raiz.join(&relativo), format!("{destino}/{relativo}")));
        })?;
    }
    for cli in ["browse", "recall", "stats", "gen-image"] {
        let path = PathBuf::from(BIN).join(cli);
        if path.is_file() {
            archivos.push((path, format!("bin/{cli}")));
        }
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
        return Err("esta org todavía no tiene máquina: se le da una al pagar el plan".into());
    };
    if row.provider == "local" {
        return Ok(None);
    }
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

/// Copia a la base el índice de lo que hay en el volumen de la org: una sola
/// operación, porque el que sabe leer el layout es el CLI que corre adentro.
/// El control plane no abre el volumen para esto, y si el sandbox no contesta
/// el índice se queda como estaba.
pub fn sincronizar(
    cliente: &Tensorlake,
    sandbox: &str,
    store: &Store,
    org: &str,
) -> Result<(), String> {
    let salida = cliente.run(
        sandbox,
        MOUNT,
        &format!(
            "JIMMY_ROOT={MOUNT} JIMMY_WORKSPACE={MOUNT}/workspace {BIN}/jimmy conversations --json"
        ),
        120,
        &mut |_| {},
    )?;
    let json: serde_json::Value = serde_json::from_str(salida.trim()).map_err(|e| {
        let dicho: String = salida.chars().take(300).collect();
        format!("no entiendo el índice: {e} · dijo: {dicho:?}")
    })?;
    let proyectos: Vec<crate::store::IndexProject> = json["projects"]
        .as_array()
        .ok_or("el índice vino sin proyectos")?
        .iter()
        .map(|proyecto| crate::store::IndexProject {
            name: proyecto["name"].as_str().unwrap_or_default().to_string(),
            size: proyecto["size"].as_u64().unwrap_or(0),
            unversioned: proyecto["unversioned"].as_bool().unwrap_or(false),
            conversations: proyecto["conversations"]
                .as_array()
                .map(|conversaciones| conversaciones.iter().map(charla_del_json).collect())
                .unwrap_or_default(),
        })
        .collect();
    store.sync_index(org, &proyectos)
}

fn charla_del_json(json: &serde_json::Value) -> crate::store::IndexConversation {
    let texto = |campo: &str| {
        json[campo]
            .as_str()
            .filter(|valor| !valor.is_empty())
            .map(str::to_string)
    };
    crate::store::IndexConversation {
        key: json["key"].as_str().unwrap_or_default().to_string(),
        project: json["project"].as_str().unwrap_or_default().to_string(),
        title: texto("title"),
        read_only: json["read_only"].as_bool().unwrap_or(true),
        last: texto("last"),
        touched_at: json["touched_at"].as_i64().unwrap_or(0),
    }
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

/// El workspace de una org que vive en su sandbox. La lista sale del índice que
/// dejó el último turno —eso no despierta a nadie— y los archivos y los adjuntos
/// se le piden al volumen en el momento: si el sandbox está dormido se lo
/// despierta, igual que para usar el browser. Lo que la web escribe lo escriben
/// las mismas funciones que corren adentro, por el CLI del agente, así el layout
/// lo arma uno solo.
pub struct Remoto {
    org: String,
    store: Arc<Store>,
    lugar: Place,
    listo: Mutex<Option<SandboxInfo>>,
}

impl Remoto {
    pub fn new(org: String, store: Arc<Store>, lugar: Place) -> Remoto {
        Remoto {
            org,
            store,
            lugar,
            listo: Mutex::new(None),
        }
    }

    fn cliente(&self) -> Result<Arc<Tensorlake>, String> {
        Ok(Arc::new(
            Tensorlake::from_env().ok_or("esta org necesita TENSORLAKE_API_KEY")?,
        ))
    }

    /// El sandbox, despierto. Se paga la primera vez que alguien mira un
    /// archivo, no al abrir la página.
    fn despierto(&self) -> Result<SandboxInfo, String> {
        let mut listo = self.listo.lock().unwrap();
        if let Some(listo) = listo.as_ref() {
            return Ok(listo.clone());
        }
        let despierto = ensure(&self.org, &self.store)?.ok_or("esta org no tiene sandbox")?;
        *listo = Some(despierto.clone());
        Ok(despierto)
    }

    /// El directorio de un proyecto del lado del control plane, que es el que
    /// después se traduce al volumen.
    fn proyecto(&self, project: &str) -> Result<PathBuf, String> {
        if project.is_empty() || project.contains('/') || project.starts_with('.') {
            return Err("ese nombre no sirve para un proyecto".into());
        }
        Ok(conversations::project_dir(&self.lugar.workspace, project))
    }

    fn del_volumen(&self, path: &Path) -> String {
        al_sandbox(&self.lugar.root, path)
    }

    fn adjunto(&self, key: &str, name: &str) -> String {
        let dir = media::dir(&conversations::get(
            &self.lugar.root,
            &self.lugar.workspace,
            key,
        ));
        format!("{}/{}", self.del_volumen(&dir), name)
    }

    /// El CLI del agente, adentro del sandbox: es el mismo binario y el mismo
    /// código que el de acá, así que el layout —los proyectos, las
    /// conversaciones— lo arma uno solo.
    fn cli(&self, args: &str) -> Result<String, String> {
        let listo = self.despierto()?;
        let cliente = self.cliente()?;
        publicar(&cliente, &listo, &exe()?)?;
        let comando =
            format!("JIMMY_ROOT={MOUNT} JIMMY_WORKSPACE={MOUNT}/workspace {BIN}/jimmy {args}");
        let (salida, codigo) =
            cliente.run_codigo(&listo.name, MOUNT, &comando, 120, &mut |_| {})?;
        match codigo {
            Some(0) => {
                self.refrescar(&cliente, &listo.name);
                Ok(salida)
            }
            _ => Err(limpiar(&salida)),
        }
    }

    /// La copia sigue al volumen: lo que se acaba de escribir ya se ve sin
    /// esperar al próximo turno. Que no se pueda no rompe la escritura.
    fn refrescar(&self, cliente: &Tensorlake, sandbox: &str) {
        if let Err(error) = sincronizar(cliente, sandbox, &self.store, &self.org) {
            eprintln!(
                "jimmy: no pude refrescar el índice de {}: {error}",
                self.org
            );
        }
    }
}

/// Qué binario se publica en el sandbox: el de acá, que es el mismo. La prueba
/// de integración lo apunta a mano, porque ella corre sobre el binario de test.
pub fn exe() -> Result<PathBuf, String> {
    match crate::env("JIMMY_TEST_EXE") {
        Some(exe) => Ok(PathBuf::from(exe)),
        None => std::env::current_exe().map_err(|e| e.to_string()),
    }
}

/// El alta de una org: su filesystem y su fila. El filesystem lo crea el SDK de
/// Tensorlake —el único que sabe hablar con ese servicio— y la fila queda con el
/// mismo nombre, que es el que el sandbox monta. Idempotente: si el filesystem
/// ya está, no se toca.
pub fn alta(org: &str, store: &Store) -> Result<(), String> {
    let nombre = format!("jimmy-{org}");
    filesystems("crear", &nombre)?;
    store.set_machine(org, "tensorlake", &nombre, &nombre)
}

/// Lo que sabe hacer el SDK: crear, listar y borrar el volumen de una org.
fn filesystems(accion: &str, nombre: &str) -> Result<String, String> {
    let script = scripts()
        .into_iter()
        .find(|path| path.is_file())
        .ok_or("no encuentro deploy/filesystems.py")?;
    let salida = std::process::Command::new("uv")
        .args([
            "run",
            "--with",
            "tensorlake",
            "python",
            &script.display().to_string(),
            accion,
            nombre,
        ])
        .output()
        .map_err(|e| format!("no pude correr uv: {e}"))?;
    match salida.status.success() {
        true => Ok(String::from_utf8_lossy(&salida.stdout).trim().to_string()),
        false => Err(format!(
            "{accion} {nombre}: {}",
            String::from_utf8_lossy(&salida.stderr).trim()
        )),
    }
}

fn scripts() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/usr/local/share/jimmy/deploy/filesystems.py"),
        PathBuf::from("deploy/filesystems.py"),
    ]
}

/// El camino de una entrada del volumen, armado sin tocar el disco de acá: el
/// archivo puede no existir de este lado, que es justamente el punto. Cada parte
/// tiene que ser un nombre que se vería en el árbol, así un `..` no sale de la
/// carpeta.
fn camino(base: &Path, sub: &str) -> Result<PathBuf, String> {
    let mut path = base.to_path_buf();
    for parte in sub.split('/').filter(|parte| !parte.is_empty()) {
        if !files::visible(parte) {
            return Err("esa carpeta no está".into());
        }
        path.push(parte);
    }
    Ok(path)
}

/// El texto de un comando que salió mal: el CLI escribe el error y el código lo
/// confirma.
fn limpiar(salida: &str) -> String {
    let texto = salida.lines().last().unwrap_or_default().trim();
    match texto.is_empty() {
        true => "no se pudo".to_string(),
        false => texto.to_string(),
    }
}

fn entre_comillas(texto: &str) -> String {
    format!("'{}'", texto.replace('\'', "'\\''"))
}

impl Workspace for Remoto {
    fn label(&self) -> String {
        MOUNT.to_string()
    }

    fn projects(&self) -> Result<Vec<Project>, String> {
        Ok(self
            .store
            .index(&self.org)?
            .into_iter()
            .map(|proyecto| Project {
                name: proyecto.name,
                size: proyecto.size,
                unversioned: proyecto.unversioned,
                conversations: proyecto
                    .conversations
                    .into_iter()
                    .map(conversacion)
                    .collect(),
            })
            .collect())
    }

    fn conversations(&self, project: &str) -> Result<Vec<Conversation>, String> {
        Ok(self
            .projects()?
            .into_iter()
            .find(|candidato| candidato.name == project)
            .map(|proyecto| proyecto.conversations)
            .unwrap_or_default())
    }

    fn tree(&self, project: &str, path: &str) -> Result<Vec<files::Entry>, String> {
        let dir = camino(&self.proyecto(project)?, path)?;
        let listo = self.despierto()?;
        let cliente = self.cliente()?;
        let mut entradas: Vec<files::Entry> = cliente
            .list_files(&listo.name, &self.del_volumen(&dir))
            .map_err(|_| "esa carpeta no está".to_string())?
            .into_iter()
            .filter(|entrada| files::visible(&entrada.name))
            .map(|entrada| files::Entry {
                kind: files::kind(&entrada.name, entrada.is_dir),
                name: entrada.name,
                dir: entrada.is_dir,
                size: entrada.size,
            })
            .collect();
        files::ordenar(&mut entradas);
        Ok(entradas)
    }

    fn read_file(
        &self,
        project: &str,
        path: &str,
        limit: Option<u64>,
    ) -> Result<(Vec<u8>, u64), String> {
        let dir = camino(&self.proyecto(project)?, path)?;
        let listo = self.despierto()?;
        let cliente = self.cliente()?;
        let bytes = cliente
            .read_file(&listo.name, &self.del_volumen(&dir))
            .map_err(|_| "ese archivo no está".to_string())?;
        let size = bytes.len() as u64;
        let recortado = match limit {
            Some(limit) if size > limit => bytes[..limit as usize].to_vec(),
            _ => bytes,
        };
        Ok((recortado, size))
    }

    fn window(&self, key: &str, end: usize) -> Result<Window, String> {
        let dir = conversations::chat_dir(&self.lugar.root, key);
        let listo = self.despierto()?;
        let cliente = self.cliente()?;
        let log = format!("{}/conversation.jsonl", self.del_volumen(&dir));
        let bytes = cliente.read_file(&listo.name, &log).unwrap_or_default();
        Ok(Log::window_of(&bytes, end))
    }

    fn read_attachment(&self, key: &str, name: &str) -> Result<Vec<u8>, String> {
        let name = media::safe_name(name).ok_or("ese nombre no sirve")?;
        let listo = self.despierto()?;
        let cliente = self.cliente()?;
        cliente.read_file(&listo.name, &self.adjunto(key, name))
    }

    fn read_attachments(&self, key: &str, names: &[String]) -> Result<Vec<axe::Image>, String> {
        names
            .iter()
            .map(|name| {
                let name = media::safe_name(name).ok_or("ese adjunto no sirve")?;
                axe::image::attach(&self.adjunto(key, name))
            })
            .collect()
    }

    fn write_attachment(&self, key: &str, name: &str, data: &[u8]) -> Result<String, String> {
        let listo = self.despierto()?;
        let cliente = self.cliente()?;
        let name = media::unique_name(name);
        cliente.write_file(&listo.name, &self.adjunto(key, &name), data)?;
        Ok(name)
    }

    fn writable(&self, key: &str) -> Result<Conversation, String> {
        let charla = self
            .store
            .conversation_index(&self.org, key)?
            .ok_or("esa conversación no existe")?;
        if charla.read_only {
            return Err("esa conversación no se escribe desde acá".into());
        }
        Ok(conversacion(charla))
    }

    fn create_project(&self, name: &str) -> Result<(), String> {
        self.cli(&format!("projects new {name}")).map(|_| ())
    }

    fn rename_project(&self, from: &str, to: &str) -> Result<(), String> {
        self.cli(&format!("projects rename {from} {to}"))
            .map(|_| ())
    }

    fn duplicate_project(&self, from: &str, to: &str) -> Result<(), String> {
        self.cli(&format!("projects duplicate {from} {to}"))
            .map(|_| ())
    }

    fn delete_project(&self, name: &str, force: bool) -> Result<(), String> {
        let force = match force {
            true => " --force",
            false => "",
        };
        self.cli(&format!("projects delete {name}{force}"))
            .map(|_| ())
    }

    fn create_conversation(&self, project: &str, title: &str) -> Result<String, String> {
        let salida = self.cli(&format!(
            "conversations new {project} {}",
            entre_comillas(title)
        ))?;
        Ok(salida.trim().to_string())
    }

    fn rename_conversation(&self, key: &str, title: &str) -> Result<(), String> {
        self.cli(&format!(
            "conversations rename {key} {}",
            entre_comillas(title)
        ))
        .map(|_| ())
    }

    fn delete_conversation(&self, key: &str) -> Result<(), String> {
        if !conversations::valid_key(key) {
            return Err("esa conversación no existe".into());
        }
        let listo = self.despierto()?;
        let cliente = self.cliente()?;
        let dir = conversations::chat_dir(&self.lugar.root, key);
        let comando = format!("rm -rf -- {}", entre_comillas(&self.del_volumen(&dir)));
        cliente.run(&listo.name, MOUNT, &comando, 60, &mut |_| {})?;
        self.refrescar(&cliente, &listo.name);
        Ok(())
    }
}

fn conversacion(charla: crate::store::IndexConversation) -> Conversation {
    Conversation {
        key: charla.key,
        project: charla.project,
        title: charla.title,
        read_only: charla.read_only,
        last: charla.last,
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
    /// La lista de una org con sandbox sale de su índice —sin despertar a
    /// nadie— y lo que no está en el índice no se escribe desde acá.
    #[test]
    fn la_lista_de_una_org_con_sandbox_sale_del_indice() {
        let store = Arc::new(store("remoto"));
        let (_, org) = store.register("don@ejemplo.com").unwrap();
        store.sync_index(&org.id, &[]).unwrap();
        let base = std::env::temp_dir().join(format!("jimmy-remoto-{}", std::process::id()));
        let lugar = Place {
            root: base.clone(),
            workspace: base.join("workspace"),
            org: Some(org.id.clone()),
        };
        let espacio = Remoto::new(org.id.clone(), store.clone(), lugar);
        assert!(espacio.projects().unwrap().is_empty());
        assert!(espacio.writable("web-1").is_err(), "no está en el índice");

        store
            .sync_index(
                &org.id,
                &[crate::store::IndexProject {
                    name: "general".into(),
                    size: 0,
                    unversioned: false,
                    conversations: vec![crate::store::IndexConversation {
                        key: "web-1".into(),
                        project: "general".into(),
                        title: Some("charla".into()),
                        read_only: false,
                        last: Some("hola".into()),
                        touched_at: 10,
                    }],
                }],
            )
            .unwrap();

        let proyectos = espacio.projects().unwrap();
        assert_eq!(proyectos.len(), 1);
        assert_eq!(proyectos[0].name, "general");
        assert_eq!(proyectos[0].conversations[0].last.as_deref(), Some("hola"));
        assert_eq!(espacio.conversations("general").unwrap().len(), 1);
        assert!(espacio.conversations("ken").unwrap().is_empty());
        assert_eq!(
            espacio.writable("web-1").unwrap().title.as_deref(),
            Some("charla")
        );
        assert_eq!(espacio.label(), MOUNT);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// El alta de verdad: crea el filesystem de la org y le deja su fila.
    /// Ignorada porque habla con el servicio, tarda, y deja y borra un
    /// filesystem de verdad.
    ///
    ///     heimdall run -p jimmy -c dev -- cargo test --bin jimmy -- --ignored el_alta_de_una_org
    #[test]
    #[ignore]
    fn el_alta_de_una_org_le_da_su_volumen() {
        let store = store("alta");
        let (_, org) = store.register("don@ejemplo.com").unwrap();
        assert!(store.machine(&org.id).unwrap().is_none(), "sin sandbox");

        alta(&org.id, &store).unwrap();
        let maquina = store
            .machine(&org.id)
            .unwrap()
            .expect("la org tiene máquina");
        assert_eq!(maquina.provider, "tensorlake");
        let nombre = format!("jimmy-{}", org.id);
        assert_eq!(maquina.file_system, nombre);
        assert_eq!(maquina.sandbox, nombre);
        let listado = filesystems("listar", &nombre).unwrap();
        assert!(
            listado.contains(&nombre),
            "el filesystem no está: {listado}"
        );

        alta(&org.id, &store).unwrap();
        filesystems("borrar", &nombre).unwrap();
    }

    /// Sin máquina no hay dónde correr: la org no cae al disco de acá, que no
    /// es su lugar. La máquina llega con el plan.
    #[test]
    fn una_org_sin_maquina_no_corre_en_ningun_lado() {
        let store = store("sin-maquina");
        let (_, org) = store.register("don@ejemplo.com").unwrap();
        let error = ensure(&org.id, &store).unwrap_err();
        assert!(error.contains("todavía no tiene máquina"), "{error}");

        store.set_machine(&org.id, "local", "", "").unwrap();
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
        let image = image();
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
