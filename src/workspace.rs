//! El workspace de una org: lo que hay adentro, sin decir dónde vive.
//!
//! La web no arma caminos ni abre archivos: pide proyectos, carpetas, adjuntos
//! y el hilo por nombre, y cuando quiere cambiar algo lo pide también. Hoy la
//! implementación es el disco del control plane con el layout de siempre;
//! cuando el trabajo se mude al sandbox, va a ser su filesystem, y la web no se
//! entera.

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
    /// Cómo se llama este workspace cuando hay que mostrarlo o acortar un
    /// camino en la pantalla.
    fn label(&self) -> String;

    /// Lo que hay, para armar la lista.
    fn projects(&self) -> Result<Vec<Project>, String>;

    /// Las conversaciones de un proyecto.
    fn conversations(&self, project: &str) -> Result<Vec<Conversation>, String>;

    /// Una carpeta de un proyecto, un nivel.
    fn tree(&self, project: &str, path: &str) -> Result<Vec<files::Entry>, String>;

    /// Un archivo de un proyecto, con su tamaño. El tope es opcional porque una
    /// imagen se manda entera y un archivo de texto se corta.
    fn read_file(
        &self,
        project: &str,
        path: &str,
        limit: Option<u64>,
    ) -> Result<(Vec<u8>, u64), String>;

    /// El hilo de una conversación, hasta el evento `end`.
    fn window(&self, key: &str, end: usize) -> Result<log::Window, String>;

    /// Un adjunto de una conversación, por el nombre con el que se subió.
    fn read_attachment(&self, key: &str, name: &str) -> Result<Vec<u8>, String>;

    /// Los adjuntos listos para mandarle al modelo.
    fn read_attachments(&self, key: &str, names: &[String]) -> Result<Vec<axe::Image>, String>;

    /// Guardar un adjunto y devolver el nombre con el que quedó.
    fn write_attachment(&self, key: &str, name: &str, data: &[u8]) -> Result<String, String>;

    /// La conversación, si se puede escribir desde acá.
    fn writable(&self, key: &str) -> Result<Conversation, String>;

    fn create_project(&self, name: &str) -> Result<(), String>;

    fn rename_project(&self, from: &str, to: &str) -> Result<(), String>;

    fn duplicate_project(&self, from: &str, to: &str) -> Result<(), String>;

    /// Borrarlo sólo si no se pierde nada, salvo que el pedido se haga cargo.
    fn delete_project(&self, name: &str, force: bool) -> Result<(), String>;

    fn create_conversation(&self, project: &str, title: &str) -> Result<String, String>;

    fn rename_conversation(&self, key: &str, title: &str) -> Result<(), String>;

    fn delete_conversation(&self, key: &str) -> Result<(), String>;
}

/// Dónde trabaja una org: la raíz del control plane que le toca y el workspace
/// adentro. Es todo lo que hace falta para correr un turno suyo.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub root: PathBuf,
    pub workspace: PathBuf,
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

    pub fn place(&self) -> Place {
        Place {
            root: self.root.clone(),
            workspace: self.workspace.clone(),
        }
    }

    fn chat_dir(&self, key: &str) -> PathBuf {
        conversations::chat_dir(&self.root, key)
    }

    /// El directorio de un proyecto. `general` es el workspace entero, que es
    /// dónde viven las conversaciones que no son de nadie.
    fn project_dir(&self, project: &str) -> Result<PathBuf, String> {
        if project.is_empty() || project.contains('/') || project.starts_with('.') {
            return Err("ese nombre no sirve para un proyecto".into());
        }
        let dir = conversations::project_dir(&self.workspace, project);
        if !dir.is_dir() {
            return Err(format!("no existe el proyecto {project}"));
        }
        Ok(dir)
    }

    fn conversation_of(&self, key: &str) -> conversations::Conversation {
        conversations::get(&self.root, &self.workspace, key)
    }
}

impl Workspace for Local {
    fn label(&self) -> String {
        self.workspace.display().to_string()
    }

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

    fn conversations(&self, project: &str) -> Result<Vec<Conversation>, String> {
        Ok(self
            .projects()?
            .into_iter()
            .find(|candidate| candidate.name == project)
            .map(|project| project.conversations)
            .unwrap_or_default())
    }

    fn tree(&self, project: &str, path: &str) -> Result<Vec<files::Entry>, String> {
        let dir = self.project_dir(project)?;
        files::list(&dir, path).ok_or_else(|| "esa carpeta no está".to_string())
    }

    fn read_file(
        &self,
        project: &str,
        path: &str,
        limit: Option<u64>,
    ) -> Result<(Vec<u8>, u64), String> {
        let dir = self.project_dir(project)?;
        let path = files::resolve(&dir, path).ok_or("ese archivo no está")?;
        let size = std::fs::metadata(&path)
            .map_err(|_| "ese archivo no está".to_string())?
            .len();
        Ok((files::read(&path, limit)?, size))
    }

    fn window(&self, key: &str, end: usize) -> Result<log::Window, String> {
        let dir = self.chat_dir(key);
        if !dir.is_dir() {
            return Err("esa conversación no existe".into());
        }
        Ok(Log::in_dir(&dir).window(end))
    }

    fn read_attachment(&self, key: &str, name: &str) -> Result<Vec<u8>, String> {
        let name = media::safe_name(name).ok_or("ese nombre no sirve")?;
        let dir = media::dir(&self.conversation_of(key));
        std::fs::read(dir.join(name)).map_err(|_| "no está".to_string())
    }

    fn read_attachments(&self, key: &str, names: &[String]) -> Result<Vec<axe::Image>, String> {
        let dir = media::dir(&self.conversation_of(key));
        names
            .iter()
            .map(|name| {
                let name = media::safe_name(name).ok_or("ese adjunto no sirve")?;
                axe::image::attach(&dir.join(name).display().to_string())
            })
            .collect()
    }

    fn write_attachment(&self, key: &str, name: &str, data: &[u8]) -> Result<String, String> {
        media::store(&media::dir(&self.conversation_of(key)), name, data)
    }

    fn writable(&self, key: &str) -> Result<Conversation, String> {
        let conversation = self.conversation_of(key);
        if conversation.read_only || !conversation.dir.is_dir() {
            return Err("esa conversación no se escribe desde acá".into());
        }
        Ok(view(conversation))
    }

    fn create_project(&self, name: &str) -> Result<(), String> {
        if !is_project_name(name) {
            return Err("ese nombre no sirve para un proyecto".into());
        }
        let dir = self.workspace.join("projects").join(name);
        if dir.exists() {
            return Err("ese proyecto ya existe".into());
        }
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())
    }

    fn rename_project(&self, from: &str, to: &str) -> Result<(), String> {
        conversations::rename_project(&self.root, &self.workspace, from, to)
    }

    fn duplicate_project(&self, from: &str, to: &str) -> Result<(), String> {
        conversations::duplicate(&self.workspace, from, to)
    }

    fn delete_project(&self, name: &str, force: bool) -> Result<(), String> {
        if !is_project_name(name) {
            return Err("ese no es un proyecto que se pueda borrar".into());
        }
        let dir = self.project_dir(name)?;
        disposable(&dir, force)?;
        std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())
    }

    fn create_conversation(&self, project: &str, title: &str) -> Result<String, String> {
        conversations::create(&self.root, &self.workspace, project, title)
    }

    fn rename_conversation(&self, key: &str, title: &str) -> Result<(), String> {
        conversations::rename(&self.root, key, title)
    }

    fn delete_conversation(&self, key: &str) -> Result<(), String> {
        let dir = self.chat_dir(key);
        if !dir.is_dir() {
            return Ok(());
        }
        std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())
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

fn is_project_name(name: &str) -> bool {
    !name.is_empty()
        && name != conversations::GENERAL
        && !name.contains('/')
        && !name.starts_with('.')
}

/// Un proyecto sin git y con algo adentro: no hay copia en ningún otro lado,
/// así que borrarlo es una decisión del que lo pide.
pub fn unversioned(dir: &Path) -> bool {
    !empty(dir) && !dir.join(".git").exists()
}

pub fn empty(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|entries| entries.count() == 0)
        .unwrap_or(true)
}

fn disposable(dir: &Path, force: bool) -> Result<(), String> {
    if empty(dir) {
        return Ok(());
    }
    if unversioned(dir) {
        return match force {
            true => Ok(()),
            false => Err("tiene archivos que no están en git".into()),
        };
    }
    if !git(dir, &["status", "--porcelain"])?.trim().is_empty() {
        return Err("tiene cambios sin commitear".into());
    }
    if !git(
        dir,
        &["log", "--branches", "--not", "--remotes", "--oneline"],
    )?
    .trim()
    .is_empty()
    {
        return Err("tiene commits sin pushear".into());
    }
    Ok(())
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err("no pude preguntarle a git".into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}
