use crate::log::Log;
use crate::protocol::Event;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

struct Connection {
    id: u64,
    user: String,
    topic: Option<String>,
    sender: Sender<Event>,
}

pub struct Bus {
    connections: Mutex<Vec<Connection>>,
    next: AtomicU64,
    version: AtomicU64,
}

impl Bus {
    pub fn new() -> Arc<Bus> {
        Arc::new(Bus {
            connections: Mutex::new(Vec::new()),
            next: AtomicU64::new(0),
            version: AtomicU64::new(0),
        })
    }

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

    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Relaxed)
    }

    pub fn publish(&self, key: &str, log: &Log, event: &Event) {
        self.version.fetch_add(1, Ordering::Relaxed);
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
