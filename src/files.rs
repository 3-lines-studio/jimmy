use crate::media;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const MAX_READ: u64 = 512 * 1024;

const HIDDEN: [&str; 4] = ["target", "node_modules", "dist", "__pycache__"];

pub struct Entry {
    pub name: String,
    pub dir: bool,
    pub size: u64,
    pub kind: &'static str,
}

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
    entries.sort_by(|one, other| {
        other
            .dir
            .cmp(&one.dir)
            .then_with(|| one.name.to_lowercase().cmp(&other.name.to_lowercase()))
    });
    Some(entries)
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
    let kind = if dir {
        "dir"
    } else if media::is_image(&name) {
        "image"
    } else if extension(&name) == "md" {
        "markdown"
    } else {
        "text"
    };
    Some(Entry {
        name,
        dir,
        size: if dir { 0 } else { metadata.len() },
        kind,
    })
}

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

fn visible(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !HIDDEN.contains(&name)
}

fn extension(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default()
}
