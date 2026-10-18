use crate::protocol::Event;
use std::collections::HashMap;
use std::io::{BufRead, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Instant, SystemTime};

pub const REPLAY: usize = 200;
const REPLAY_BYTES: u64 = 1024 * 1024;

#[derive(Default)]
pub struct Window {
    pub first: usize,
    pub events: Vec<Event>,
}

#[derive(Default)]
pub struct Span {
    pub first: usize,
    pub total: usize,
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
        let first = self.start(&index, end);
        let mut events = Vec::new();
        let _ = self.each(&index, first, end, &mut |_, event| {
            events.push(event.clone());
            Ok::<bool, ()>(true)
        });
        Window { first, events }
    }

    pub fn replay(
        &self,
        since: usize,
        send: &mut dyn FnMut(&[u8]) -> std::io::Result<()>,
    ) -> std::io::Result<Span> {
        let Some(index) = index(&self.path) else {
            return Ok(Span::default());
        };
        let total = index.offsets.len();
        let first = self.start(&index, total);
        let Some((start, stop)) = index.bounds(first, total) else {
            return Ok(Span { first, total });
        };
        let mut file = std::fs::File::open(&self.path)?;
        file.seek(SeekFrom::Start(start))?;
        let mut reader = std::io::BufReader::with_capacity(64 * 1024, file.take(stop - start));
        let mut line = Vec::new();
        let mut at = first;
        loop {
            line.clear();
            if reader.read_until(b'\n', &mut line)? == 0 {
                break;
            }
            if line.last() == Some(&b'\n') {
                line.pop();
            }
            if at >= since && !line.is_empty() {
                send(&line)?;
            }
            at += 1;
        }
        Ok(Span { first, total })
    }

    fn start(&self, index: &Indexed, end: usize) -> usize {
        let first = end
            .saturating_sub(REPLAY)
            .max(index.byte_floor(end, REPLAY_BYTES));
        (first..end)
            .find(|at| index.turn(*at))
            .unwrap_or_else(|| self.previous_start(index, first).unwrap_or(first))
    }

    fn each<E>(
        &self,
        index: &Indexed,
        from: usize,
        to: usize,
        f: &mut dyn FnMut(usize, &Event) -> Result<bool, E>,
    ) -> Result<(), E> {
        if from >= to {
            return Ok(());
        }
        let Some((start, stop)) = index.bounds(from, to) else {
            return Ok(());
        };
        let Ok(mut file) = std::fs::File::open(&self.path) else {
            return Ok(());
        };
        let mut bytes = Vec::with_capacity((stop - start) as usize);
        if file.seek(SeekFrom::Start(start)).is_err() {
            return Ok(());
        }
        if file.take(stop - start).read_to_end(&mut bytes).is_err() {
            return Ok(());
        }
        let mut at = from;
        for line in bytes.split(|byte| *byte == b'\n') {
            let Ok(event) = serde_json::from_slice::<Event>(line) else {
                continue;
            };
            if !f(at, &event)? {
                return Ok(());
            }
            at += 1;
        }
        Ok(())
    }

    fn previous_start(&self, index: &Indexed, before: usize) -> Option<usize> {
        (0..before).rev().find(|at| index.turn(*at))
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
    turns: Arc<Vec<bool>>,
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
    turns: Arc<Vec<bool>>,
    seen: u64,
}

impl Indexed {
    fn byte_floor(&self, end: usize, budget: u64) -> usize {
        let stop = self.offsets.get(end).copied().unwrap_or(self.seen);
        let mut at = end;
        while at > 0 {
            if stop - self.offsets[at - 1] > budget {
                break;
            }
            at -= 1;
        }
        at
    }

    fn turn(&self, at: usize) -> bool {
        self.turns.get(at).copied().unwrap_or(false)
    }

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
    let (mut offsets, mut turns, seen) = match cache.get(path) {
        Some(index) if index.len == len && index.modified == modified => {
            return Some(Indexed {
                offsets: index.offsets.clone(),
                turns: index.turns.clone(),
                seen: index.seen,
            });
        }
        Some(index) if len > index.len => {
            ((*index.offsets).clone(), (*index.turns).clone(), index.seen)
        }
        _ => (Vec::new(), Vec::new(), 0),
    };
    let mut file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::with_capacity((len - seen) as usize);
    file.seek(SeekFrom::Start(seen)).ok()?;
    file.take(len - seen).read_to_end(&mut bytes).ok()?;
    let seen = scan(&bytes, seen, &mut offsets, &mut turns);
    let offsets = Arc::new(offsets);
    let turns = Arc::new(turns);
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
            turns: turns.clone(),
            used: Instant::now(),
        },
    );
    Some(Indexed {
        offsets,
        turns,
        seen,
    })
}

fn scan(bytes: &[u8], base: u64, offsets: &mut Vec<u64>, turns: &mut Vec<bool>) -> u64 {
    let mut pos = 0;
    let mut done = base;
    while let Some(at) = bytes[pos..].iter().position(|byte| *byte == b'\n') {
        if let Ok(event) = serde_json::from_slice::<Event>(&bytes[pos..pos + at]) {
            offsets.push(base + pos as u64);
            turns.push(is_turn_start(&event));
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
        assert!(window.events.len() <= REPLAY);
        assert!(window.events.len() > REPLAY - 13);
        assert!(matches!(window.events[0], Event::User { .. }));
        assert_eq!(window.first + window.events.len(), events.len());
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
        assert_eq!(window.events.len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_index_follows_appends_and_keeps_paging() {
        let (log, dir) = log_with("crece", &[user("uno"), Event::Done { text: "a".into() }]);
        let first = log.window(usize::MAX);
        assert_eq!(first.events.len(), 2);
        for turn in 0..250 {
            log.append(&user(&turn.to_string()));
            log.append(&Event::Done {
                text: turn.to_string(),
            });
        }
        let window = log.window(usize::MAX);
        let all = full(&dir.join("conversation.jsonl"));
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
    fn the_window_stops_at_the_byte_budget() {
        let mut events = Vec::new();
        for turn in 0..3 {
            events.push(user(&turn.to_string()));
            events.push(Event::Assistant {
                text: "x".repeat(400 * 1024),
            });
        }
        let (log, dir) = log_with("presupuesto", &events);
        let window = log.window(usize::MAX);
        assert!(
            window.events.len() < events.len(),
            "{}",
            window.events.len()
        );
        assert!(matches!(window.events[0], Event::User { .. }));
        let bytes: usize = serde_json::to_string(&window.events).unwrap().len();
        assert!(bytes < 1400 * 1024, "{bytes}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn replay_sends_only_what_is_missing() {
        let mut events = Vec::new();
        for turn in 0..4 {
            events.push(user(&turn.to_string()));
            events.push(Event::Done {
                text: turn.to_string(),
            });
        }
        let (log, dir) = log_with("replay", &events);
        let window = log.window(usize::MAX);
        let expected: Vec<Vec<u8>> = window
            .events
            .iter()
            .map(|event| serde_json::to_vec(event).unwrap())
            .collect();
        for since in [0, window.first, window.first + 1, window.first + 3, 99] {
            let mut sent: Vec<Vec<u8>> = Vec::new();
            let span = log
                .replay(since, &mut |line| {
                    sent.push(line.to_vec());
                    Ok(())
                })
                .unwrap();
            assert_eq!(span.first, window.first);
            assert_eq!(span.total, events.len());
            let skip = since.saturating_sub(window.first).min(window.events.len());
            assert_eq!(sent, expected[skip..], "since={since}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_rewritten_log_is_read_again() {
        let (log, dir) = log_with("reescrito", &[user("uno"), user("dos")]);
        assert_eq!(log.window(usize::MAX).events.len(), 2);
        std::fs::File::create(dir.join("conversation.jsonl")).unwrap();
        assert!(log.window(usize::MAX).events.is_empty());
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
        assert_eq!(window.events.len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
