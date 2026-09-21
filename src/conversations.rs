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
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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

pub struct Project {
    pub name: String,
    pub path: PathBuf,
    pub conversations: Vec<Conversation>,
}

pub fn chat_dir(root: &Path, key: &str) -> PathBuf {
    root.join("chats").join(key)
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
            conversations.sort_by(|a, b| b.key.cmp(&a.key));
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

fn project_dir(workspace: &Path, project: &str) -> PathBuf {
    match project {
        GENERAL => workspace.to_path_buf(),
        name => workspace.join("projects").join(name),
    }
}

fn random_hex() -> String {
    use std::io::Read;
    let mut buf = [0u8; 4];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut buf))
        .is_err()
    {
        buf = std::process::id().to_le_bytes();
    }
    buf.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn new_key() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("web-{ms}-{}", random_hex())
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

    fn chat(root: &Path, key: &str) {
        std::fs::create_dir_all(chat_dir(root, key)).unwrap();
        std::fs::write(chat_dir(root, key).join("transcript.jsonl"), "{}\n").unwrap();
    }

    #[test]
    fn a_chat_without_metadata_is_general_and_read_only() {
        let (root, workspace) = scratch("general");
        chat(&root, "7469057930");
        let conversation = get(&root, &workspace, "7469057930");
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
}
