//! El workspace de una org: lo que hay adentro, sin decir dónde vive.
//!
//! La web no arma caminos ni abre archivos: pide proyectos, carpetas, adjuntos
//! y el hilo por nombre. Hoy la implementación es el disco del control plane
//! con el layout de siempre; cuando el trabajo se mude al sandbox, va a ser su
//! filesystem, y la web no se entera.

use crate::conversations;
use crate::files;
use crate::log::{self, Log};
use crate::media;
use std::path::{Path, PathBuf};

/// Un proyecto con lo que la lista necesita saber: cuánto ocupa y si tiene
/// trabajo sin versionar. Nada de caminos.
#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub name: String,
    pub conversations: Vec<Conversation>,
    pub size: u64,
    pub unversioned: bool,
}

/// Una conversación vista desde afuera: quién es, de qué proyecto, cómo se
/// llama y qué dijo último. Dónde vive es asunto del workspace.
#[derive(Debug, Clone, PartialEq)]
pub struct Conversation {
    pub key: String,
    pub project: String,
    pub title: Option<String>,
    pub read_only: bool,
    pub last: Option<String>,
}

pub trait Workspace: Send + Sync {
    fn projects(&self) -> Result<Vec<Project>, String>;

    /// Una carpeta de un proyecto, un nivel.
    fn tree(&self, project: &str, path: &str) -> Result<Vec<files::Entry>, String>;

    fn read_file(
        &self,
        project: &str,
        path: &str,
        limit: Option<u64>,
    ) -> Result<(Vec<u8>, u64), String>;

    /// Un adjunto de la conversación, por el nombre con el que se subió.
    fn read_attachment(&self, key: &str, name: &str) -> Result<Vec<u8>, String>;

    /// El hilo de una conversación, hasta el evento `end`.
    fn window(&self, key: &str, end: usize) -> Result<log::Window, String>;
}

/// El workspace del disco de siempre: la raíz del control plane y el workspace
/// de la org que se está mirando.
pub struct Local {
    root: PathBuf,
    workspace: PathBuf,
}

impl Local {
    pub fn new(root: PathBuf, workspace: PathBuf) -> Local {
        Local { root, workspace }
    }

    /// Mientras las escrituras no pasen por acá, quien las haga necesita los
    /// caminos. Esto es lo que se va cuando el sandbox sea el dueño.
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    fn project_dir(&self, project: &str) -> Option<PathBuf> {
        if project.is_empty() || project.contains('/') || project.starts_with('.') {
            return None;
        }
        let dir = conversations::project_dir(&self.workspace, project);
        dir.is_dir().then_some(dir)
    }

    fn chat_dir(&self, key: &str) -> PathBuf {
        conversations::chat_dir(&self.root, key)
    }
}

impl Workspace for Local {
    fn projects(&self) -> Result<Vec<Project>, String> {
        Ok(conversations::projects(&self.root, &self.workspace)
            .into_iter()
            .map(|project| Project {
                unversioned: unversioned(&project.path),
                size: match project.name == conversations::GENERAL {
                    true => 0,
                    false => conversations::size(&project.path),
                },
                conversations: project.conversations.into_iter().map(view).collect(),
                name: project.name,
            })
            .collect())
    }

    fn tree(&self, project: &str, path: &str) -> Result<Vec<files::Entry>, String> {
        let dir = self.project_dir(project).ok_or("ese proyecto no existe")?;
        files::list(&dir, path).ok_or_else(|| "esa carpeta no está".to_string())
    }

    fn read_file(
        &self,
        project: &str,
        path: &str,
        limit: Option<u64>,
    ) -> Result<(Vec<u8>, u64), String> {
        let dir = self.project_dir(project).ok_or("ese proyecto no existe")?;
        let path = files::resolve(&dir, path).ok_or("ese archivo no está")?;
        let size = std::fs::metadata(&path)
            .map_err(|_| "ese archivo no está".to_string())?
            .len();
        Ok((files::read(&path, limit)?, size))
    }

    fn read_attachment(&self, key: &str, name: &str) -> Result<Vec<u8>, String> {
        let conversation = conversations::get(&self.root, &self.workspace, key);
        let name = media::safe_name(name).ok_or("ese nombre no sirve")?;
        std::fs::read(media::dir(&conversation).join(name)).map_err(|_| "no está".to_string())
    }

    fn window(&self, key: &str, end: usize) -> Result<log::Window, String> {
        let dir = self.chat_dir(key);
        if !dir.is_dir() {
            return Err("esa conversación no existe".into());
        }
        Ok(Log::in_dir(&dir).window(end))
    }
}

fn view(conversation: conversations::Conversation) -> Conversation {
    Conversation {
        last: conversations::last_message(&conversation.dir),
        key: conversation.key,
        project: conversation.project,
        title: conversation.title,
        read_only: conversation.read_only,
    }
}

/// Un proyecto con algo adentro y sin git es trabajo sin versionar: la web
/// avisa antes de dejarlo borrar.
pub fn unversioned(dir: &Path) -> bool {
    !empty(dir) && !dir.join(".git").exists()
}

pub fn empty(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|entries| entries.count() == 0)
        .unwrap_or(true)
}
