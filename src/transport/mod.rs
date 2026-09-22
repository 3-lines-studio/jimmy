pub mod slack;
pub mod telegram;

use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Session {
    pub channel: String,
    pub thread: Option<String>,
}

impl Session {
    pub fn channel(channel: impl Into<String>) -> Self {
        Self {
            channel: channel.into(),
            thread: None,
        }
    }

    pub fn key(&self) -> String {
        match &self.thread {
            Some(thread) => format!("{}/{}", self.channel, thread),
            None => self.channel.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Msg(pub String);

pub struct Event {
    pub session: Session,
    pub sender: String,
    pub is_bot: bool,
    /// El usuario apretó frenar: no es un mensaje, es una orden.
    pub stop: bool,
    pub text: String,
    pub image: Option<String>,
    pub voice: Option<(String, u64)>,
}

pub trait Transport: Send + Sync {
    fn parse_target(&self, target: &str) -> Result<Session, String>;
    fn progress(&self, session: &Session) -> Option<Msg>;
    /// Un paso del turno, para el mensaje que ya está en pantalla. El
    /// transporte que no tiene dónde escribirlo no hace nada.
    fn status(&self, _session: &Session, _placeholder: &Msg, _text: &str) {}
    fn answer(&self, session: &Session, placeholder: Option<Msg>, markdown: &str);
    fn note(&self, session: &Session, text: &str);
    fn fail(&self, session: &Session, placeholder: Option<Msg>, text: &str);
    fn download(&self, id: &str) -> Result<(String, Vec<u8>), String>;
    fn send_media(
        &self,
        session: &Session,
        path: &Path,
        caption: Option<&str>,
    ) -> Result<Msg, String>;
}

pub trait EventSource: Send {
    fn recv(&mut self) -> Result<Vec<Event>, String>;
}
