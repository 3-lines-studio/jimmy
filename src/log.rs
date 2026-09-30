//! The conversation as a client sees it: one JSON event per line, in the chat
//! directory, next to the transcript.
//!
//! The transcript is what the model gets back on the next turn; this is what a
//! human reads. Both describe the same turn, and the parent is the only writer:
//! it logs the events the worker sends it, plus the message the user sent.

use crate::protocol::Event;
use std::io::Write;
use std::path::{Path, PathBuf};

/// How much of the backlog a client gets when it attaches. Enough to open the
/// conversation in context, and bounded so a long one does not come down the
/// wire whole.
pub const REPLAY: usize = 200;

/// Un tramo del log: los eventos que van de `first` a `end`, y cuántos hay en
/// total, que es lo que el cliente necesita para pedir los anteriores.
#[derive(Default)]
pub struct Window {
    pub first: usize,
    pub total: usize,
    pub events: Vec<Event>,
}

pub struct Log {
    path: PathBuf,
}

impl Log {
    pub fn in_dir(dir: &Path) -> Self {
        Self {
            path: dir.join("conversation.jsonl"),
        }
    }

    /// El tramo que termina en `end`, de hasta [`REPLAY`] eventos. Arranca donde
    /// arranca un turno: un corte en el medio dejaría un grupo de tools partido.
    /// Si el tramo no abarca ni un turno, el corte se corre hacia atrás hasta el
    /// último, que es lo que lo deja entero.
    pub fn window(&self, end: usize) -> Window {
        let all = self.read();
        let end = end.min(all.len());
        let mut first = end.saturating_sub(REPLAY);
        if let Some(offset) = all[first..end].iter().position(is_turn_start) {
            first += offset;
        } else if let Some(offset) = all[..first].iter().rposition(is_turn_start) {
            first = offset;
        }
        Window {
            first,
            total: all.len(),
            events: all[first..end].to_vec(),
        }
    }

    pub fn append(&self, event: &Event) {
        let Ok(mut line) = serde_json::to_string(event) else {
            return;
        };
        line.push('\n');
        let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        else {
            return;
        };
        let _ = file.write_all(line.as_bytes());
    }

    fn read(&self) -> Vec<Event> {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return Vec::new();
        };
        text.lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }
}

fn is_turn_start(event: &Event) -> bool {
    matches!(event, Event::User { .. })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(text: &str) -> Event {
        Event::User {
            text: text.to_string(),
            author: String::new(),
            images: Vec::new(),
        }
    }

    fn log_with(name: &str, events: &[Event]) -> (Log, PathBuf) {
        let dir = std::env::temp_dir().join(format!("jimmy-log-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log = Log::in_dir(&dir);
        for event in events {
            log.append(event);
        }
        (log, dir)
    }

    #[test]
    fn the_window_starts_where_a_turn_does() {
        let mut events = Vec::new();
        for turn in 0..40 {
            events.push(user(&turn.to_string()));
            for tool in 0..6 {
                events.push(Event::ToolStart {
                    id: format!("{turn}-{tool}"),
                    name: "read".into(),
                    args: "{}".into(),
                });
                events.push(Event::ToolResult {
                    id: format!("{turn}-{tool}"),
                    text: "x".into(),
                    ms: 1,
                    failed: false,
                });
            }
        }
        let (log, dir) = log_with("turnos-largos", &events);
        let window = log.window(usize::MAX);
        assert_eq!(window.total, events.len());
        assert!(window.events.len() <= REPLAY);
        assert!(window.events.len() > REPLAY - 13);
        assert!(matches!(window.events[0], Event::User { .. }));
        assert_eq!(window.first + window.events.len(), window.total);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_backlog_before_a_window_stops_before_it() {
        let mut events = Vec::new();
        for turn in 0..300 {
            events.push(user(&turn.to_string()));
            events.push(Event::Done {
                text: turn.to_string(),
            });
        }
        let (log, dir) = log_with("turnos-cortos", &events);
        let window = log.window(usize::MAX);
        let earlier = log.window(window.first);
        assert!(earlier.events.len() <= REPLAY);
        assert!(matches!(earlier.events.first(), Some(Event::User { .. })));
        assert_eq!(earlier.first + earlier.events.len(), window.first);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_short_log_comes_whole() {
        let (log, dir) = log_with("corto", &[user("hola")]);
        let window = log.window(usize::MAX);
        assert_eq!(window.first, 0);
        assert_eq!(window.total, 1);
        assert_eq!(window.events.len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
