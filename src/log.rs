use crate::protocol::Event;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Instant, SystemTime};

pub const REPLAY: usize = 200;

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

    pub fn window(&self, end: usize) -> Window {
        let Some(index) = index(&self.path) else {
            return Window::default();
        };
        let total = index.offsets.len();
        let end = end.min(total);
        let mut first = end.saturating_sub(REPLAY);
        let mut events = self.slice(&index, first, end);
        match events.iter().position(is_turn_start) {
            Some(at) => {
                first += at;
                events.drain(..at);
            }
            None => {
                if let Some(at) = self.previous_start(&index, first) {
                    first = at;
                    events = self.slice(&index, first, end);
                }
            }
        }
        Window {
            first,
            total,
            events,
        }
    }

    fn slice(&self, index: &Indexed, from: usize, to: usize) -> Vec<Event> {
        if from >= to {
            return Vec::new();
        }
        let Some((start, stop)) = index.bounds(from, to) else {
            return Vec::new();
        };
        let Ok(mut file) = std::fs::File::open(&self.path) else {
            return Vec::new();
        };
        let mut bytes = Vec::with_capacity((stop - start) as usize);
        if file.seek(SeekFrom::Start(start)).is_err() {
            return Vec::new();
        }
        if file.take(stop - start).read_to_end(&mut bytes).is_err() {
            return Vec::new();
        }
        bytes
            .split(|byte| *byte == b'\n')
            .filter_map(|line| serde_json::from_slice(line).ok())
            .collect()
    }

    fn previous_start(&self, index: &Indexed, before: usize) -> Option<usize> {
        const STEP: usize = 64;
        let mut end = before;
        while end > 0 {
            let start = end.saturating_sub(STEP);
            let events = self.slice(index, start, end);
            if let Some(at) = events.iter().rposition(is_turn_start) {
                return Some(start + at);
            }
            end = start;
        }
        None
    }

    pub fn last_answer(&self) -> Option<String> {
        let mut file = std::fs::File::open(&self.path).ok()?;
        let length = file.metadata().ok()?.len();
        file.seek(SeekFrom::Start(length.saturating_sub(16 * 1024)))
            .ok()?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).ok()?;
        String::from_utf8_lossy(&bytes)
            .lines()
            .rev()
            .find_map(|line| {
                let event: Event = serde_json::from_str(line).ok()?;
                match event {
                    Event::Done { text } | Event::Assistant { text } if !text.trim().is_empty() => {
                        Some(text)
                    }
                    _ => None,
                }
            })
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
}

fn is_turn_start(event: &Event) -> bool {
    matches!(event, Event::User { .. })
}

struct Index {
    seen: u64,
    len: u64,
    modified: Option<SystemTime>,
    offsets: Arc<Vec<u64>>,
    used: Instant,
}

const INDEXES: usize = 64;

fn cache() -> &'static Mutex<HashMap<PathBuf, Index>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Index>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Clone)]
struct Indexed {
    offsets: Arc<Vec<u64>>,
    seen: u64,
}

impl Indexed {
    fn bounds(&self, from: usize, to: usize) -> Option<(u64, u64)> {
        let start = *self.offsets.get(from)?;
        let stop = self.offsets.get(to).copied().unwrap_or(self.seen);
        (stop > start).then_some((start, stop))
    }
}

fn index(path: &Path) -> Option<Indexed> {
    let meta = std::fs::metadata(path).ok()?;
    let len = meta.len();
    let modified = meta.modified().ok();
    let mut cache = cache().lock().unwrap();
    let (mut offsets, seen) = match cache.get(path) {
        Some(index) if index.len == len && index.modified == modified => {
            return Some(Indexed {
                offsets: index.offsets.clone(),
                seen: index.seen,
            });
        }
        Some(index) if len > index.len => ((*index.offsets).clone(), index.seen),
        _ => (Vec::new(), 0),
    };
    let mut file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::with_capacity((len - seen) as usize);
    file.seek(SeekFrom::Start(seen)).ok()?;
    file.take(len - seen).read_to_end(&mut bytes).ok()?;
    let seen = scan(&bytes, seen, &mut offsets);
    let offsets = Arc::new(offsets);
    if cache.len() >= INDEXES {
        let oldest = cache
            .iter()
            .min_by_key(|(_, index)| index.used)
            .map(|(path, _)| path.clone());
        if let Some(oldest) = oldest {
            cache.remove(&oldest);
        }
    }
    cache.insert(
        path.to_path_buf(),
        Index {
            seen,
            len,
            modified,
            offsets: offsets.clone(),
            used: Instant::now(),
        },
    );
    Some(Indexed { offsets, seen })
}

fn scan(bytes: &[u8], base: u64, offsets: &mut Vec<u64>) -> u64 {
    let mut pos = 0;
    let mut done = base;
    while let Some(at) = bytes[pos..].iter().position(|byte| *byte == b'\n') {
        if serde_json::from_slice::<Event>(&bytes[pos..pos + at]).is_ok() {
            offsets.push(base + pos as u64);
        }
        pos += at + 1;
        done = base + pos as u64;
    }
    done
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

    fn full(path: &Path) -> Vec<Event> {
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
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

    #[test]
    fn the_index_follows_appends_and_keeps_paging() {
        let (log, dir) = log_with("crece", &[user("uno"), Event::Done { text: "a".into() }]);
        let first = log.window(usize::MAX);
        assert_eq!(first.total, 2);
        for turn in 0..250 {
            log.append(&user(&turn.to_string()));
            log.append(&Event::Done {
                text: turn.to_string(),
            });
        }
        let window = log.window(usize::MAX);
        let all = full(&dir.join("conversation.jsonl"));
        assert_eq!(window.total, all.len());
        assert_eq!(window.first + window.events.len(), all.len());
        assert_eq!(
            serde_json::to_value(&window.events).unwrap(),
            serde_json::to_value(&all[window.first..]).unwrap()
        );
        let earlier = log.window(window.first);
        assert_eq!(
            serde_json::to_value(&earlier.events).unwrap(),
            serde_json::to_value(&all[earlier.first..window.first]).unwrap()
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_rewritten_log_is_read_again() {
        let (log, dir) = log_with("reescrito", &[user("uno"), user("dos")]);
        assert_eq!(log.window(usize::MAX).total, 2);
        std::fs::File::create(dir.join("conversation.jsonl")).unwrap();
        assert_eq!(log.window(usize::MAX).total, 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_damaged_line_is_skipped_and_not_counted() {
        let (log, dir) = log_with("roto", &[user("uno")]);
        let path = dir.join("conversation.jsonl");
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str("{\"event\":\"done\"\n");
        std::fs::write(&path, text).unwrap();
        let window = log.window(usize::MAX);
        assert_eq!(window.total, 1);
        assert_eq!(window.events.len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
