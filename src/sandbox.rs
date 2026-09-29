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

/// Quién corre el turno de una conversación.
pub trait Sandbox: Send + Sync {
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
}
