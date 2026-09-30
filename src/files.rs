//! Los archivos de un proyecto, para mirarlos desde la web. Es de sólo lectura:
//! el que escribe es el agente.

use crate::media;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Lo más grande que se muestra de un archivo que no es imagen. Un `Cargo.lock`
/// entra cómodo; un log de decenas de megas no, y en pantalla no se iba a leer
/// igual.
pub const MAX_READ: u64 = 512 * 1024;

/// Salidas de build: no se listan ni se abren. Lo que empieza con punto queda
/// afuera por su cuenta, y ahí van los metadatos y los `.env`.
pub const HIDDEN: [&str; 4] = ["target", "node_modules", "dist", "__pycache__"];

pub struct Entry {
    pub name: String,
    pub dir: bool,
    pub size: u64,
    pub kind: &'static str,
}

/// Un nivel del árbol, con las carpetas primero. Cada carpeta se pide cuando se
/// abre: el árbol entero de un repo es un JSON enorme para algo que se mira de
/// a una carpeta por vez.
pub fn list(root: &Path, sub: &str) -> Option<Vec<Entry>> {
    let dir = resolve(root, sub)?;
    if !dir.is_dir() {
        return None;
    }
    let mut entries: Vec<Entry> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(entry)
        .collect();
    ordenar(&mut entries);
    Some(entries)
}

/// Los bytes de un archivo, cortados en el tope que le pidan. Las imágenes van
/// enteras porque el navegador las muestra; el texto va cortado porque nadie
/// lee medio megabyte en pantalla.
/// Cómo se muestra una entrada, para que el mismo nombre diga lo mismo venga de
/// donde venga.
pub fn kind(name: &str, dir: bool) -> &'static str {
    if dir {
        return "dir";
    }
    if media::is_image(name) {
        return "image";
    }
    match extension(name) == "md" {
        true => "markdown",
        false => "text",
    }
}

/// Las carpetas primero y después por nombre, sin mayúsculas: el árbol de la
/// web y el que llega del sandbox se ordenan igual.
pub fn ordenar(entries: &mut [Entry]) {
    entries.sort_by(|one, other| {
        other
            .dir
            .cmp(&one.dir)
            .then_with(|| one.name.to_lowercase().cmp(&other.name.to_lowercase()))
    });
}

pub fn read(path: &Path, limit: Option<u64>) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    match limit {
        Some(limit) => file.take(limit),
        None => file.take(u64::MAX),
    }
    .read_to_end(&mut bytes)
    .map_err(|error| error.to_string())?;
    Ok(bytes)
}

/// Un texto no tiene bytes nulos. Es el mismo criterio que usa el `read` del
/// agente, y sirve para no mostrar un binario como si fuera una página.
pub fn is_text(bytes: &[u8]) -> bool {
    !bytes.iter().take(8000).any(|byte| *byte == 0)
}

fn entry(entry: std::fs::DirEntry) -> Option<Entry> {
    let name = entry.file_name().into_string().ok()?;
    if !visible(&name) {
        return None;
    }
    let metadata = entry.metadata().ok()?;
    let dir = metadata.is_dir();
    let kind = kind(&name, dir);
    Some(Entry {
        name,
        dir,
        size: if dir { 0 } else { metadata.len() },
        kind,
    })
}

/// El camino que manda el cliente, adentro del proyecto y nada más: cada parte
/// tiene que ser un nombre que se vería en el árbol, y el resultado —ya con los
/// enlaces simbólicos resueltos por el sistema— tiene que seguir cayendo
/// adentro.
pub fn resolve(root: &Path, sub: &str) -> Option<PathBuf> {
    let root = root.canonicalize().ok()?;
    let mut path = root.clone();
    for part in sub.split('/').filter(|part| !part.is_empty()) {
        if !visible(part) {
            return None;
        }
        path.push(part);
    }
    let path = path.canonicalize().ok()?;
    path.starts_with(&root).then_some(path)
}

pub fn visible(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !HIDDEN.contains(&name)
}

fn extension(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default()
}
