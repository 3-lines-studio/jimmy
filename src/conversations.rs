//! Projects and conversations.
//!
//! A project is a directory under `workspace/projects`, nothing else: no id, no
//! registry, no metadata. A conversation is a chat with a directory of its own
//! under `root/chats`, and it belongs either to a project or to `general`.
//!
//! The conversations that arrive through a transport are not created by
//! anybody: they exist because Telegram or Slack names them, they live in
//! `general`, and the web frontend watches them without writing. The ones the
//! web frontend creates carry a `meta.json` with their project and title.

use axe::atomic_write;
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const GENERAL: &str = "general";
pub const NEW_TITLE: &str = "nueva conversación";

#[derive(Serialize, Deserialize, Default)]
struct Meta {
    project: Option<String>,
    title: Option<String>,
}

pub struct Conversation {
    pub key: String,
    pub dir: PathBuf,
    pub cwd: PathBuf,
    pub project: String,
    pub title: Option<String>,
    pub read_only: bool,
}

impl Conversation {
    /// Lo último que pasó en la conversación: es lo que ordena la lista. El log
    /// y el transcript se escriben en cada turno, la metadata al crear o al
    /// renombrar.
    pub fn updated(&self) -> SystemTime {
        ["conversation.jsonl", "transcript.jsonl", "meta.json"]
            .iter()
            .filter_map(|name| std::fs::metadata(self.dir.join(name)).ok()?.modified().ok())
            .max()
            .unwrap_or(SystemTime::UNIX_EPOCH)
    }
}

pub struct Project {
    pub name: String,
    pub path: PathBuf,
    pub conversations: Vec<Conversation>,
}

pub fn chat_dir(root: &Path, key: &str) -> PathBuf {
    root.join("chats").join(key)
}

/// Un key de chat es un nombre, o dos con una barra en el medio, como el
/// `canal/hilo` de Slack. Cada parte tiene que ser un nombre de los que se ven
/// en un directorio, así un `..` no saca el camino de `chats/`.
pub fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key.split('/').all(|part| {
            !part.is_empty()
                && !part.starts_with('.')
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        })
}

/// Resolves a key that may not have a conversation yet, which is the case of
/// every chat a transport names for the first time.
pub fn get(root: &Path, workspace: &Path, key: &str) -> Conversation {
    let dir = chat_dir(root, key);
    let meta = read_meta(&dir);
    let project = meta
        .as_ref()
        .and_then(|meta| meta.project.clone())
        .unwrap_or_else(|| GENERAL.to_string());
    let read_only = meta.is_none();
    Conversation {
        key: key.to_string(),
        cwd: project_dir(workspace, &project),
        project,
        title: meta.and_then(|meta| meta.title),
        dir,
        read_only,
    }
}

pub fn projects(root: &Path, workspace: &Path) -> Vec<Project> {
    let mut names = vec![GENERAL.to_string()];
    names.extend(list_dirs(&workspace.join("projects")));

    let mut owned: HashMap<String, Vec<Conversation>> = HashMap::new();
    for key in discovered(root) {
        let conversation = get(root, workspace, &key);
        owned
            .entry(conversation.project.clone())
            .or_default()
            .push(conversation);
    }

    names
        .into_iter()
        .map(|name| {
            let mut conversations = owned.remove(&name).unwrap_or_default();
            conversations.sort_by_cached_key(|conversation| {
                (
                    Reverse(conversation.updated()),
                    Reverse(conversation.key.clone()),
                )
            });
            Project {
                path: project_dir(workspace, &name),
                name,
                conversations,
            }
        })
        .collect()
}

pub fn create(root: &Path, workspace: &Path, project: &str, title: &str) -> Result<String, String> {
    if project != GENERAL && !project_dir(workspace, project).is_dir() {
        return Err(format!("no existe el proyecto {project}"));
    }
    let key = new_key();
    std::fs::create_dir_all(chat_dir(root, &key)).map_err(|e| e.to_string())?;
    write_meta(
        root,
        &key,
        &Meta {
            project: Some(project.to_string()),
            title: Some(title.to_string()),
        },
    )?;
    Ok(key)
}

pub fn rename(root: &Path, key: &str, title: &str) -> Result<(), String> {
    let dir = chat_dir(root, key);
    let mut meta = read_meta(&dir).unwrap_or_default();
    meta.title = Some(title.to_string());
    write_meta(root, key, &meta)
}

pub fn project_dir(workspace: &Path, project: &str) -> PathBuf {
    match project {
        GENERAL => workspace.to_path_buf(),
        name => workspace.join("projects").join(name),
    }
}

/// Le pone otro nombre a la carpeta del proyecto y a la meta de cada una de sus
/// conversaciones. `general` no se renombra: es dónde viven las que no son de
/// nadie.
pub fn rename_project(root: &Path, workspace: &Path, from: &str, to: &str) -> Result<(), String> {
    if from == GENERAL || to == GENERAL {
        return Err("ese proyecto no se renombra".into());
    }
    if from == to {
        return Ok(());
    }
    let old = workspace.join("projects").join(from);
    let new = workspace.join("projects").join(to);
    if !old.is_dir() {
        return Err(format!("no existe el proyecto {from}"));
    }
    if new.exists() {
        return Err("ese proyecto ya existe".into());
    }
    std::fs::rename(&old, &new).map_err(|e| e.to_string())?;
    for key in discovered(root) {
        let dir = chat_dir(root, &key);
        let Some(mut meta) = read_meta(&dir) else {
            continue;
        };
        if meta.project.as_deref() != Some(from) {
            continue;
        }
        meta.project = Some(to.to_string());
        write_meta(root, &key, &meta)?;
    }
    Ok(())
}

/// Un proyecto duplicado es otro directorio con los mismos archivos: la copia
/// es literal, con git y lo que esté ignorado adentro. Las conversaciones no
/// viajan, son del proyecto original.
pub fn duplicate(workspace: &Path, from: &str, to: &str) -> Result<(), String> {
    if from == GENERAL || to == GENERAL {
        return Err("eso no se duplica".into());
    }
    if from == to {
        return Err("ese proyecto ya existe".into());
    }
    let origin = workspace.join("projects").join(from);
    let copy = workspace.join("projects").join(to);
    if !origin.is_dir() {
        return Err(format!("no existe el proyecto {from}"));
    }
    if copy.exists() {
        return Err("ese proyecto ya existe".into());
    }
    let out = std::process::Command::new("cp")
        .arg("-a")
        .arg("--")
        .arg(&origin)
        .arg(&copy)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        let _ = std::fs::remove_dir_all(&copy);
        return Err("no pude copiar los archivos".into());
    }
    Ok(())
}

/// Lo que el proyecto ocupa en el volumen: los bloques que reserva cada
/// archivo, que es lo que descuenta el `df`, y no los bytes que dice tener. No
/// sigue symlinks, igual que `du`.
pub fn size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.metadata() {
            Ok(meta) if meta.is_dir() => size(&entry.path()),
            Ok(meta) => meta.blocks() * 512,
            Err(_) => 0,
        })
        .sum()
}

const TAIL: u64 = 16 * 1024;
const SNIPPET: usize = 140;

/// Lo último que contestó el asistente, en un renglón y sin markdown: es el pie
/// del proyecto en la sidebar. Mira el final del archivo, que puede ser enorme,
/// y se queda con la primera línea entera que encuentra.
pub fn last_message(dir: &Path) -> Option<String> {
    let text = tail(&dir.join("transcript.jsonl"), TAIL)?;
    text.lines()
        .rev()
        .find_map(said_line)
        .map(|said| cut(&flatten(&said), SNIPPET))
}

/// Lo que dijo el asistente en esa línea, si dijo algo: los turnos que sólo
/// llaman herramientas no cuentan.
fn said_line(line: &str) -> Option<String> {
    let entry: serde_json::Value = serde_json::from_str(line).ok()?;
    let message = entry.get("message")?;
    if message.get("Role")?.as_str()? != "assistant" {
        return None;
    }
    let content = message.get("Content")?.as_str()?.trim();
    (!content.is_empty()).then(|| content.to_string())
}

fn tail(path: &Path, size: u64) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let start = length.saturating_sub(size);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    if start == 0 {
        return Some(text);
    }
    text.split_once('\n').map(|(_, rest)| rest.to_string())
}

/// Un solo renglón, sin el markdown que en la sidebar no se renderiza.
fn flatten(text: &str) -> String {
    let linked = unlink(text);
    let bare: String = linked.chars().filter(|c| !"`*".contains(*c)).collect();
    bare.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_start_matches(['#', '>'])
        .trim()
        .to_string()
}

/// `[así](https://ejemplo.com)` se queda con `así`.
fn unlink(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find("](") else {
            break;
        };
        let after = &rest[open + close + 2..];
        let Some(end) = after.find(')') else {
            break;
        };
        out.push_str(&rest[..open]);
        out.push_str(&rest[open + 1..open + close]);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

fn cut(text: &str, limit: usize) -> String {
    match text.char_indices().nth(limit) {
        Some((at, _)) => text[..at].to_string(),
        None => text.to_string(),
    }
}

fn new_key() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("web-{ms}-{}", crate::random::hex(4))
}

fn meta_path(root: &Path, key: &str) -> PathBuf {
    chat_dir(root, key).join("meta.json")
}

fn read_meta(dir: &Path) -> Option<Meta> {
    let text = std::fs::read_to_string(dir.join("meta.json")).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_meta(root: &Path, key: &str, meta: &Meta) -> Result<(), String> {
    let text = serde_json::to_string_pretty(meta).map_err(|e| e.to_string())?;
    atomic_write(&meta_path(root, key), text.as_bytes()).map_err(|e| e.to_string())
}

fn list_dirs(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

/// Every chat directory that has something in it, at any depth: a Slack thread
/// is a level deeper than a Telegram chat.
fn discovered(root: &Path) -> Vec<String> {
    let mut keys = Vec::new();
    walk(&root.join("chats"), "", &mut keys);
    keys.sort();
    keys
}

/// A chat that was created but never used has only its metadata, and still
/// counts: it is what the web frontend just made.
fn is_conversation(path: &Path) -> bool {
    ["transcript.jsonl", "conversation.jsonl", "meta.json"]
        .iter()
        .any(|name| path.join(name).exists())
}

fn walk(dir: &Path, prefix: &str, keys: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let key = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if is_conversation(&path) {
            keys.push(key);
            continue;
        }
        walk(&path, &key, keys);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("jimmy-conv-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("root");
        let workspace = base.join("workspace");
        std::fs::create_dir_all(root.join("chats")).unwrap();
        std::fs::create_dir_all(workspace.join("projects/ken")).unwrap();
        std::fs::create_dir_all(workspace.join("projects/bifrost")).unwrap();
        (root, workspace)
    }

    #[test]
    fn a_key_never_leaves_the_chats_directory() {
        assert!(valid_key("7469057930"));
        assert!(valid_key("web-1790635496411-f755c8d0"));
        assert!(valid_key("C0123/1726570000.123456"));
        assert!(!valid_key(".."));
        assert!(!valid_key("../../afuera"));
        assert!(!valid_key("a/../b"));
        assert!(!valid_key("/etc"));
        assert!(!valid_key("a//b"));
        assert!(!valid_key(".oculto"));
        assert!(!valid_key(""));
    }

    fn chat(root: &Path, key: &str) {
        std::fs::create_dir_all(chat_dir(root, key)).unwrap();
        std::fs::write(chat_dir(root, key).join("transcript.jsonl"), "{}\n").unwrap();
    }

    fn transcript(root: &Path, key: &str, lines: &[&str]) {
        std::fs::create_dir_all(chat_dir(root, key)).unwrap();
        std::fs::write(
            chat_dir(root, key).join("transcript.jsonl"),
            lines.join("\n") + "\n",
        )
        .unwrap();
    }

    const DIJO: &str = r#"{"type":"message","message":{"Role":"assistant","Content":"%s"}}"#;

    fn said(root: &Path, key: &str) -> Option<String> {
        last_message(&chat_dir(root, key))
    }

    #[test]
    fn the_last_answer_skips_the_turns_that_only_call_tools() {
        let (root, _) = scratch("last");
        transcript(
            &root,
            "uno",
            &[
                &DIJO.replace("%s", "arranco"),
                r#"{"type":"message","message":{"Role":"user","Content":"dale"}}"#,
                &DIJO.replace("%s", ""),
                r#"{"type":"message","message":{"Role":"tool","Content":"salida"}}"#,
            ],
        );
        assert_eq!(said(&root, "uno").as_deref(), Some("arranco"));
        std::fs::remove_dir_all(root.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_chat_where_nobody_answered_yet_has_nothing_to_show() {
        let (root, _) = scratch("sinnada");
        transcript(
            &root,
            "uno",
            &[r#"{"type":"message","message":{"Role":"user","Content":"hola"}}"#],
        );
        assert_eq!(said(&root, "uno"), None);
        std::fs::remove_dir_all(root.parent().unwrap()).unwrap();
    }

    #[test]
    fn the_answer_comes_in_one_line_and_without_markdown() {
        let (root, _) = scratch("renglon");
        transcript(
            &root,
            "uno",
            &[&DIJO.replace(
                "%s",
                "## Listo\\n\\nMirá [el PR](https://github.com/x/y/pull/165) y el `cargo test`: **todo verde**.",
            )],
        );
        assert_eq!(
            said(&root, "uno").as_deref(),
            Some("Listo Mirá el PR y el cargo test: todo verde.")
        );
        std::fs::remove_dir_all(root.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_long_answer_is_cut_without_breaking_a_character() {
        let (root, _) = scratch("corte");
        let largo = "ñ".repeat(400);
        transcript(&root, "uno", &[&DIJO.replace("%s", &largo)]);
        let said = said(&root, "uno").unwrap();
        assert_eq!(said.chars().count(), SNIPPET);
        std::fs::remove_dir_all(root.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_transcript_longer_than_the_tail_still_finds_the_last_answer() {
        let (root, _) = scratch("cola");
        let mut lines = vec![String::from(
            r#"{"type":"message","message":{"Role":"assistant","Content":"vieja"}}"#,
        )];
        for i in 0..2000 {
            lines.push(format!(
                r#"{{"type":"message","message":{{"Role":"tool","Content":"salida {i}"}}}}"#,
            ));
        }
        lines.push(
            r#"{"type":"message","message":{"Role":"assistant","Content":"la última"}}"#
                .to_string(),
        );
        let borrowed: Vec<&str> = lines.iter().map(String::as_str).collect();
        transcript(&root, "uno", &borrowed);
        assert_eq!(said(&root, "uno").as_deref(), Some("la última"));
        std::fs::remove_dir_all(root.parent().unwrap()).unwrap();
    }

    #[test]
    fn renaming_a_project_moves_the_folder_and_its_conversations() {
        let (root, workspace) = scratch("renombrar");
        let key = create(&root, &workspace, "ken", "una charla").unwrap();
        rename_project(&root, &workspace, "ken", "ken-viejo").unwrap();
        assert!(!workspace.join("projects/ken").exists());
        assert!(workspace.join("projects/ken-viejo").is_dir());
        let listed = projects(&root, &workspace);
        let moved = listed.iter().find(|p| p.name == "ken-viejo").unwrap();
        assert_eq!(moved.conversations[0].key, key);
        assert_eq!(
            moved.conversations[0].cwd,
            workspace.join("projects/ken-viejo")
        );
        std::fs::remove_dir_all(root.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_project_that_is_not_there_or_already_exists_is_not_renamed() {
        let (root, workspace) = scratch("norename");
        assert!(rename_project(&root, &workspace, "fantasma", "otro").is_err());
        assert!(rename_project(&root, &workspace, "ken", "bifrost").is_err());
        assert!(rename_project(&root, &workspace, GENERAL, "otro").is_err());
        assert!(rename_project(&root, &workspace, "ken", GENERAL).is_err());
        assert!(workspace.join("projects/ken").is_dir());
        std::fs::remove_dir_all(root.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_chat_without_metadata_is_general_and_read_only() {
        let (root, workspace) = scratch("general");
        chat(&root, "123456789");
        let conversation = get(&root, &workspace, "123456789");
        assert_eq!(conversation.project, GENERAL);
        assert_eq!(conversation.cwd, workspace);
        assert!(conversation.read_only);
        assert!(conversation.title.is_none());
        std::fs::remove_dir_all(root.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_slack_thread_is_a_conversation_of_its_own() {
        let (root, workspace) = scratch("thread");
        chat(&root, "C1/1699.1");
        assert_eq!(discovered(&root), ["C1/1699.1"]);
        let conversation = get(&root, &workspace, "C1/1699.1");
        assert_eq!(conversation.dir, root.join("chats/C1/1699.1"));
        std::fs::remove_dir_all(root.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_directory_without_a_chat_is_not_a_conversation() {
        let (root, workspace) = scratch("empty");
        chat(&root, "C1/1699.1");
        assert_eq!(
            projects(&root, &workspace)[0].conversations.len(),
            1,
            "C1 es un cajón, no una conversación"
        );
        std::fs::remove_dir_all(root.parent().unwrap()).unwrap();
    }

    #[test]
    fn projects_show_general_first_with_its_own_conversations() {
        let (root, workspace) = scratch("projects");
        chat(&root, "123");
        let key = create(&root, &workspace, "ken", "arrancar ken").unwrap();
        let listed = projects(&root, &workspace);
        let names: Vec<&str> = listed.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["general", "bifrost", "ken"]);

        let general = &listed[0];
        assert_eq!(general.conversations.len(), 1);
        assert_eq!(general.conversations[0].key, "123");

        let ken = &listed[2];
        assert_eq!(ken.path, workspace.join("projects/ken"));
        assert_eq!(ken.conversations[0].key, key);
        assert_eq!(ken.conversations[0].title.as_deref(), Some("arrancar ken"));
        assert_eq!(ken.conversations[0].cwd, workspace.join("projects/ken"));
        assert!(!ken.conversations[0].read_only);
        std::fs::remove_dir_all(root.parent().unwrap()).unwrap();
    }

    #[test]
    fn the_threads_go_from_the_newest_to_the_oldest() {
        let (root, workspace) = scratch("order");
        let old = create(&root, &workspace, "ken", "vieja").unwrap();
        let new = create(&root, &workspace, "ken", "nueva").unwrap();
        let middle = create(&root, &workspace, "ken", "media").unwrap();
        let busy = create(&root, &workspace, "ken", "al día").unwrap();
        touch(&chat_dir(&root, &old).join("meta.json"), 1_000);
        touch(&chat_dir(&root, &middle).join("meta.json"), 2_000);
        touch(&chat_dir(&root, &new).join("meta.json"), 3_000);
        touch(&chat_dir(&root, &busy).join("meta.json"), 500);
        std::fs::write(chat_dir(&root, &busy).join("transcript.jsonl"), "{}\n").unwrap();

        let listed = projects(&root, &workspace);
        let ken = listed.iter().find(|project| project.name == "ken").unwrap();
        let titles: Vec<&str> = ken
            .conversations
            .iter()
            .filter_map(|conversation| conversation.title.as_deref())
            .collect();
        assert_eq!(
            titles,
            ["al día", "nueva", "media", "vieja"],
            "manda el último turno, no la creación"
        );
        std::fs::remove_dir_all(root.parent().unwrap()).unwrap();
    }

    fn touch(path: &Path, seconds: u64) {
        let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds);
        file.set_modified(when).unwrap();
    }

    #[test]
    fn creating_in_a_project_that_does_not_exist_fails() {
        let (root, workspace) = scratch("missing");
        assert!(create(&root, &workspace, "nada", "x").is_err());
        assert!(std::fs::read_dir(root.join("chats"))
            .unwrap()
            .next()
            .is_none());
        std::fs::remove_dir_all(root.parent().unwrap()).unwrap();
    }

    #[test]
    fn rename_keeps_the_project() {
        let (root, workspace) = scratch("rename");
        let key = create(&root, &workspace, "ken", "sin título").unwrap();
        rename(&root, &key, "arrancar ken").unwrap();
        let conversation = get(&root, &workspace, &key);
        assert_eq!(conversation.project, "ken");
        assert_eq!(conversation.title.as_deref(), Some("arrancar ken"));
        std::fs::remove_dir_all(root.parent().unwrap()).unwrap();
    }

    #[test]
    fn el_peso_suma_lo_de_adentro_y_no_sigue_symlinks() {
        let (root, workspace) = scratch("peso");
        let dir = workspace.join("projects/peso");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/main.go"), "package main").unwrap();
        assert_eq!(
            size(&dir),
            size(&dir.join("src")),
            "da igual el subdirectorio"
        );

        let solo_archivos = size(&dir);
        assert!(solo_archivos > 0);
        std::os::unix::fs::symlink("/data/workspace", dir.join("afuera")).unwrap();
        assert_eq!(size(&dir), solo_archivos, "el symlink no se sigue");

        std::fs::write(dir.join("nota.txt"), vec![0u8; 10_000]).unwrap();
        assert!(
            size(&dir) >= solo_archivos + 10_000,
            "un archivo grande suma lo que pide"
        );
        assert_eq!(size(&workspace.join("projects/fantasma")), 0);
        std::fs::remove_dir_all(root.parent().unwrap()).unwrap();
    }
}
