//! Dónde corre un turno.
//!
//! El agente no sabe si el turno corre en un proceso de esta máquina o en un
//! sandbox de otro lado: pide el turno y recibe eventos. Hoy hay una sola
//! implementación, el pool local, y el corte existe para que la remota entre
//! sin tocar al agente.

use crate::conversations::Conversation;
use crate::protocol::{Command, Event};
use crate::transport::Session;

pub enum Turn {
    Answer(String),
    Failed(String),
}

/// Qué hacer con cada evento en el camino al final del turno: jimmy los anota
/// como el log de la conversación.
pub type OnEvent<'a> = &'a mut dyn FnMut(&Event);

/// Quién corre el turno de una conversación, y dónde.
///
/// Un sandbox es de una org: adentro vive su workspace y corre su trabajo. En
/// local el sandbox es este mismo proceso, así que prepararlo o dormirlo no
/// hace falta; en un proveedor de verdad, `ensure` lo crea o lo despierta y
/// `suspend` lo guarda sin bajarlo.
pub trait Sandbox: Send + Sync {
    /// Dejarlo listo: crearlo si no existe, despertarlo si está dormido.
    ///
    /// Los tres métodos de vida no tienen quien los llame todavía: en local no
    /// hay nada que preparar y todavía no se borra una org. El día que el
    /// sandbox sea de un proveedor, se llaman antes y después de cada turno.
    #[allow(dead_code)]
    fn ensure(&self, org: &str) -> Result<(), String>;

    fn turn(
        &self,
        session: &Session,
        conversation: &Conversation,
        command: Command,
        on_event: OnEvent,
    ) -> Result<Turn, String>;

    /// Interrumpe el turno de esa conversación, si hay uno corriendo.
    fn cancel(&self, key: &str);

    /// Baja el turno a la fuerza, para que no siga escribiendo en una carpeta
    /// que estamos por borrar.
    fn kill(&self, key: &str);

    fn running(&self, key: &str) -> bool;

    /// Guardarlo sin bajarlo, para que despierte cuando haga falta.
    #[allow(dead_code)]
    fn suspend(&self, org: &str) -> Result<(), String>;

    /// Bajarlo y llevarse lo efímero. Lo que vive en el workspace queda.
    #[allow(dead_code)]
    fn destroy(&self, org: &str) -> Result<(), String>;
}
