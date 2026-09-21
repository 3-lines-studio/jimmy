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

pub struct Bus {
    watchers: Mutex<HashMap<String, Vec<Sender<Event>>>>,
}

impl Bus {
    pub fn new() -> Arc<Bus> {
        Arc::new(Bus {
            watchers: Mutex::new(HashMap::new()),
        })
    }

    /// The events so far and everything from here on.
    pub fn attach(&self, key: &str, log: &Log) -> (Vec<Event>, Receiver<Event>) {
        let mut watchers = self.watchers.lock().unwrap();
        let backlog = log.events();
        let (sender, receiver) = mpsc::channel();
        watchers.entry(key.to_string()).or_default().push(sender);
        (backlog, receiver)
    }

    pub fn publish(&self, key: &str, log: &Log, event: &Event) {
        let mut watchers = self.watchers.lock().unwrap();
        log.append(event);
        let Some(list) = watchers.get_mut(key) else {
            return;
        };
        list.retain(|sender| sender.send(event.clone()).is_ok());
    }
}
