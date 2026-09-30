//! Who is connected, and who is watching what.
//!
//! The log on disk is the backlog; the bus is what is happening while somebody
//! is connected. Writing an event and telling the watchers happen under the
//! same lock, so somebody who attaches in the middle never misses one nor sees
//! it twice.

use crate::protocol::Event;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

struct Connection {
    id: u64,
    user: String,
    /// La conversación que mira, o nada si sólo quiere saber quién está.
    topic: Option<String>,
    sender: Sender<Event>,
}

pub struct Bus {
    connections: Mutex<Vec<Connection>>,
    next: AtomicU64,
}

impl Bus {
    pub fn new() -> Arc<Bus> {
        Arc::new(Bus {
            connections: Mutex::new(Vec::new()),
            next: AtomicU64::new(0),
        })
    }

    /// Everything from here on, for a reader that already read its own backlog.
    pub fn attach(&self, key: &str, user: &str) -> (u64, Receiver<Event>) {
        let mut connections = self.connections.lock().unwrap();
        let (sender, receiver) = mpsc::channel();
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        connections.push(Connection {
            id,
            user: user.to_string(),
            topic: Some(key.to_string()),
            sender,
        });
        tell_who_is_watching(&mut connections, key);
        tell_who_is_online(&mut connections);
        (id, receiver)
    }

    /// Una conexión que sólo espera la presencia: quién está conectado.
    pub fn watch(&self, user: &str) -> (u64, Receiver<Event>) {
        let mut connections = self.connections.lock().unwrap();
        let (sender, receiver) = mpsc::channel();
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        connections.push(Connection {
            id,
            user: user.to_string(),
            topic: None,
            sender,
        });
        tell_who_is_online(&mut connections);
        (id, receiver)
    }

    /// Se va esa conexión y ninguna otra: quien recarga la página se conecta de
    /// nuevo antes de que la vieja se entere, y la vieja no se lleva puesta a la
    /// nueva.
    pub fn detach(&self, key: &str, id: u64) {
        let mut connections = self.connections.lock().unwrap();
        connections.retain(|watch| watch.id != id);
        tell_who_is_watching(&mut connections, key);
        tell_who_is_online(&mut connections);
    }

    pub fn detach_watch(&self, id: u64) {
        let mut connections = self.connections.lock().unwrap();
        connections.retain(|watch| watch.id != id);
        tell_who_is_online(&mut connections);
    }

    /// Un evento, sin más: lo ven los que están mirando ahora. El log lo
    /// escribe el worker, y el que avisa no lo guarda.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detaching_leaves_the_other_connection_of_the_same_user() {
        let bus = Bus::new();
        let (gone, dead) = bus.attach("chat", "bob");
        let (_, live) = bus.attach("chat", "bob");
        drop(dead);
        bus.detach("chat", gone);
        while live.try_recv().is_ok() {}

        bus.show(
            "chat",
            &Event::Assistant {
                text: "hola".into(),
            },
        );
        assert!(live.try_recv().is_ok(), "la conexión viva dejó de escuchar");
    }
}
