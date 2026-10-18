pub mod slack;

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
    pub author: String,
    pub is_bot: bool,
    pub stop: bool,
    pub text: String,
    pub image: Option<String>,
}

pub trait Transport: Send + Sync {
    fn parse_target(&self, target: &str) -> Result<Session, String>;
    fn progress(&self, session: &Session) -> Option<Msg>;
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

pub struct Idle;

impl EventSource for Idle {
    fn recv(&mut self) -> Result<Vec<Event>, String> {
        std::thread::sleep(std::time::Duration::from_secs(3_600));
        Ok(Vec::new())
    }
}

pub struct Null;

impl Transport for Null {
    fn parse_target(&self, key: &str) -> Result<Session, String> {
        Err(format!("no hay transporte configurado para {key}"))
    }

    fn progress(&self, _: &Session) -> Option<Msg> {
        None
    }

    fn answer(&self, _: &Session, _: Option<Msg>, _: &str) {}

    fn note(&self, _: &Session, _: &str) {}

    fn fail(&self, _: &Session, _: Option<Msg>, _: &str) {}

    fn download(&self, _: &str) -> Result<(String, Vec<u8>), String> {
        Err("este transporte no baja archivos".into())
    }

    fn send_media(&self, _: &Session, _: &Path, _: Option<&str>) -> Result<Msg, String> {
        Err("este transporte no manda archivos".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_null_transport_knows_no_chat() {
        assert!(Null.parse_target("123456789").is_err());
    }
}
