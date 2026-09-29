//! Los adjuntos de una conversación: los archivos que viven en su `uploads/`,
//! cómo se llaman, y la cola de lo que el asistente mandó y todavía no está en
//! el log.

use crate::conversations::Conversation;
use crate::protocol::Event;
use std::path::{Path, PathBuf};

pub const UPLOADS: &str = "uploads";
/// La cola de lo que el asistente manda, al lado del chat: la escribe el CLI,
/// que corre adentro del sandbox, y la lee el padre, que escribe el log.
pub const OUTBOX: &str = "outbox.jsonl";

/// Las que la web sabe mostrar.
pub fn is_image(name: &str) -> bool {
    matches!(
        extension(name).as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp"
    )
}

pub fn content_type(name: &str) -> &'static str {
    match extension(name).as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}

fn extension(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default()
}

pub fn dir(conversation: &Conversation) -> PathBuf {
    conversation.dir.join(UPLOADS)
}

/// Guarda los bytes con un nombre único y devuelve el nombre.
pub fn store(dir: &Path, name: &str, bytes: &[u8]) -> Result<String, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let name = unique_name(name);
    std::fs::write(dir.join(&name), bytes).map_err(|e| e.to_string())?;
    Ok(name)
}

/// Lo que llega por la query o por el nombre del archivo es un nombre y nada
/// más: sin barras, sin `..` y sin nada que `join` pueda leer como un camino.
pub fn safe_name(name: &str) -> Option<&str> {
    let clean = !name.is_empty()
        && name.len() <= 80
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    clean.then_some(name)
}

/// El cliente elige la parte legible y el sello de tiempo la hace única, así dos
/// `foto.png` no se pisan.
pub fn unique_name(name: &str) -> String {
    let base: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .take(60)
        .collect();
    let base = base.trim_matches('.');
    let stamp = axe::session::now_ms();
    if base.is_empty() {
        return format!("{stamp}-adjunto");
    }
    format!("{stamp}-{base}")
}

/// Deja anotado lo que el asistente manda: el CLI es otro proceso y el log lo
/// escribe el padre, así que esto espera a que termine el turno.
pub fn queue(conversation: &Conversation, event: &Event) -> Result<(), String> {
    let dir = conversation.dir.join(UPLOADS);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    append(&conversation.dir.join(OUTBOX), event)
}

/// Lo que quedó en la cola, y la vacía.
pub fn drain(conversation: &Conversation) -> Vec<Event> {
    let path = conversation.dir.join(OUTBOX);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let _ = std::fs::remove_file(&path);
    text.lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn append(path: &Path, event: &Event) -> Result<(), String> {
    let mut line = serde_json::to_string(event).map_err(|e| e.to_string())?;
    line.push('\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    std::io::Write::write_all(&mut file, line.as_bytes()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conversation(dir: PathBuf) -> Conversation {
        Conversation {
            key: "web-1".into(),
            dir,
            cwd: PathBuf::from("."),
            project: "general".into(),
            title: None,
            read_only: false,
        }
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jimmy-media-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_queued_image_waits_there_once() {
        let dir = scratch("queue");
        let conversation = conversation(dir.clone());
        let event = Event::Image {
            name: "17-foto.png".into(),
            caption: "mirá".into(),
        };
        queue(&conversation, &event).unwrap();
        queue(&conversation, &event).unwrap();

        let drained = drain(&conversation);
        assert_eq!(drained.len(), 2);
        assert!(matches!(&drained[0], Event::Image { name, .. } if name == "17-foto.png"));
        assert!(drain(&conversation).is_empty(), "la cola queda vacía");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_name_a_client_sends_cannot_be_a_path() {
        assert_eq!(safe_name("17-foto.png"), Some("17-foto.png"));
        assert_eq!(safe_name("../../meta.json"), None);
        assert_eq!(safe_name(".."), None);
        assert_eq!(safe_name("con/barra.png"), None);
        assert_eq!(safe_name(""), None);
    }

    #[test]
    fn the_extension_decides_what_a_file_is() {
        assert!(is_image("17-foto.PNG"));
        assert!(!is_image("informe.pdf"));
        assert_eq!(content_type("a.JPEG"), "image/jpeg");
        assert_eq!(content_type("a.bin"), "application/octet-stream");
    }
}
