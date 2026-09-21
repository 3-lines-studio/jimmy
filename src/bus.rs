//! Who is watching a conversation right now.
//!
//! The log on disk is the backlog; the bus is what is happening while somebody
//! is connected. Writing an event and telling the watchers happen under the
//! same lock, so somebody who attaches in the middle never misses one nor sees
//! it twice.

use crate::log::Log;
use crate::protocol::Event;
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

struct Watcher {
    user: String,
    sender: Sender<Event>,
}

pub struct Bus {
    watchers: Mutex<HashMap<String, Vec<Watcher>>>,
}

impl Bus {
    pub fn new() -> Arc<Bus> {
        Arc::new(Bus {
            watchers: Mutex::new(HashMap::new()),
        })
    }

    /// The events so far and everything from here on.
    pub fn attach(&self, key: &str, log: &Log, user: &str) -> (Vec<Event>, Receiver<Event>) {
        let mut watchers = self.watchers.lock().unwrap();
        let backlog = log.events();
        let (sender, receiver) = mpsc::channel();
        watchers.entry(key.to_string()).or_default().push(Watcher {
            user: user.to_string(),
            sender,
        });
        self.tell_who_is_watching(&mut watchers, key);
        (backlog, receiver)
    }

    pub fn detach(&self, key: &str, user: &str) {
        let mut watchers = self.watchers.lock().unwrap();
        if let Some(list) = watchers.get_mut(key) {
            if let Some(index) = list.iter().position(|watcher| watcher.user == user) {
                list.remove(index);
            }
        }
        self.tell_who_is_watching(&mut watchers, key);
    }

    pub fn publish(&self, key: &str, log: &Log, event: &Event) {
        let mut watchers = self.watchers.lock().unwrap();
        if !matches!(
            event,
            Event::Delta { .. } | Event::ToolDelta { .. } | Event::Presence { .. }
        ) {
            log.append(event);
        }
        let Some(list) = watchers.get_mut(key) else {
            return;
        };
        list.retain(|watcher| watcher.sender.send(event.clone()).is_ok());
    }

    fn tell_who_is_watching(&self, watchers: &mut HashMap<String, Vec<Watcher>>, key: &str) {
        let Some(list) = watchers.get_mut(key) else {
            return;
        };
        let mut users: Vec<String> = list.iter().map(|watcher| watcher.user.clone()).collect();
        users.sort();
        users.dedup();
        let event = Event::Presence { users };
        list.retain(|watcher| watcher.sender.send(event.clone()).is_ok());
    }
}
