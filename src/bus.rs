//! Who is connected, and who is watching what.
//!
//! The log on disk is the backlog; the bus is what is happening while somebody
//! is connected. Writing an event and telling the watchers happen under the
//! same lock, so somebody who attaches in the middle never misses one nor sees
//! it twice.

use crate::log::Log;
use crate::protocol::Event;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

struct Connection {
    user: String,
    /// La conversación que mira, o nada si sólo quiere saber quién está.
    topic: Option<String>,
    sender: Sender<Event>,
}

pub struct Bus {
    connections: Mutex<Vec<Connection>>,
}

impl Bus {
    pub fn new() -> Arc<Bus> {
        Arc::new(Bus {
            connections: Mutex::new(Vec::new()),
        })
    }

    /// Everything from here on, for a reader that already read its own backlog.
    pub fn attach(&self, key: &str, user: &str) -> Receiver<Event> {
        let mut connections = self.connections.lock().unwrap();
        let (sender, receiver) = mpsc::channel();
        connections.push(Connection {
            user: user.to_string(),
            topic: Some(key.to_string()),
            sender,
        });
        tell_who_is_watching(&mut connections, key);
        tell_who_is_online(&mut connections);
        receiver
    }

    /// Una conexión que sólo espera la presencia: quién está conectado.
    pub fn watch(&self, user: &str) -> Receiver<Event> {
        let mut connections = self.connections.lock().unwrap();
        let (sender, receiver) = mpsc::channel();
        connections.push(Connection {
            user: user.to_string(),
            topic: None,
            sender,
        });
        tell_who_is_online(&mut connections);
        receiver
    }

    pub fn detach(&self, key: &str, user: &str) {
        let mut connections = self.connections.lock().unwrap();
        connections.retain(|watch| !(watch.topic.as_deref() == Some(key) && watch.user == user));
        tell_who_is_watching(&mut connections, key);
        tell_who_is_online(&mut connections);
    }

    pub fn detach_watch(&self, user: &str) {
        let mut connections = self.connections.lock().unwrap();
        connections.retain(|watch| !(watch.topic.is_none() && watch.user == user));
        tell_who_is_online(&mut connections);
    }

    pub fn publish(&self, key: &str, log: &Log, event: &Event) {
        let mut connections = self.connections.lock().unwrap();
        if !matches!(
            event,
            Event::Delta { .. }
                | Event::ToolDelta { .. }
                | Event::Presence { .. }
                | Event::Online { .. }
                | Event::Typing { .. }
        ) {
            log.append(event);
        }
        send(&mut connections, key, event);
    }

    /// Un evento que no queda guardado: lo ven los que están mirando ahora.
    pub fn show(&self, key: &str, event: &Event) {
        let mut connections = self.connections.lock().unwrap();
        send(&mut connections, key, event);
    }
}

fn send(connections: &mut Vec<Connection>, key: &str, event: &Event) {
    connections.retain(|watch| {
        watch.topic.as_deref() != Some(key) || watch.sender.send(event.clone()).is_ok()
    });
}

fn tell_who_is_watching(connections: &mut Vec<Connection>, key: &str) {
    let users = names(
        connections
            .iter()
            .filter(|watch| watch.topic.as_deref() == Some(key)),
    );
    let event = Event::Presence { users };
    connections.retain(|watch| {
        watch.topic.as_deref() != Some(key) || watch.sender.send(event.clone()).is_ok()
    });
}

fn tell_who_is_online(connections: &mut Vec<Connection>) {
    let users = names(connections.iter());
    let event = Event::Online { users };
    connections.retain(|watch| watch.sender.send(event.clone()).is_ok());
}

fn names<'a>(connections: impl Iterator<Item = &'a Connection>) -> Vec<String> {
    let mut users: Vec<String> = connections.map(|watch| watch.user.clone()).collect();
    users.sort();
    users.dedup();
    users
}
