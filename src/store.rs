use axe::durable::{Journal, TaskRecord, TaskState};
use axe::session::Entry;
use axe::Message;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;

pub const INTERRUPTED: &str =
    "error: interrupted, the process died before this call returned; it was not run again";

pub const RESTART_LIMIT: u32 = 5;

pub enum Recovered {
    Resume,
    Blocked(u32),
    Nothing,
}

pub fn live_turn(journal: &Journal) -> Option<&TaskRecord> {
    journal
        .tasks()
        .filter(|task| {
            task.kind == "turn" && matches!(task.state, TaskState::Pending | TaskState::Running)
        })
        .max_by_key(|task| task.id)
}

pub fn new_turn(journal: &mut Journal) -> TaskRecord {
    TaskRecord {
        id: journal.new_task(),
        kind: "turn".into(),
        state: TaskState::Running,
        phase: "request".into(),
        checkpoint: Value::Null,
        attempts: 0,
        updated: 0,
    }
}

pub fn recover_turn(dir: &Path) -> Result<Recovered, String> {
    if !dir.join("state.json").exists() && !dir.join("journal.jsonl").exists() {
        if dir.join("transcript.jsonl").exists() {
            return Ok(Recovered::Resume);
        }
        return Ok(Recovered::Nothing);
    }
    let mut journal = Journal::open(dir).map_err(|error| error.to_string())?;
    let Some(mut task) = live_turn(&journal).cloned() else {
        return Ok(Recovered::Nothing);
    };
    task.attempts += 1;
    task.state = TaskState::Running;
    task.phase = "restart".into();
    if task.attempts >= RESTART_LIMIT {
        task.state = TaskState::Blocked;
        journal
            .commit(Vec::new(), vec![task])
            .map_err(|error| error.to_string())?;
        return Ok(Recovered::Blocked(RESTART_LIMIT));
    }
    journal
        .commit(Vec::new(), vec![task])
        .map_err(|error| error.to_string())?;
    Ok(Recovered::Resume)
}

pub fn open(dir: &Path) -> Result<Journal, String> {
    import(dir)?;
    Journal::open(dir).map_err(|error| match error.kind() {
        std::io::ErrorKind::WouldBlock => "el chat lo está corriendo otra instancia".to_string(),
        _ => error.to_string(),
    })
}

pub fn entries(journal: &mut Journal) -> Result<Vec<Entry>, String> {
    repair(journal)?;
    Ok(journal
        .entries()
        .iter()
        .map(|record| record.entry.clone())
        .collect())
}

fn repair(journal: &mut Journal) -> Result<(), String> {
    let mut answered = HashSet::new();
    let mut intents = Vec::new();
    for record in journal.entries() {
        match &record.entry {
            Entry::Message { message } if message.role == "tool" => {
                answered.insert(message.tool_call_id.clone());
            }
            Entry::ToolIntent { id, .. } => intents.push(id.clone()),
            _ => {}
        }
    }
    let missing: Vec<Entry> = intents
        .into_iter()
        .filter(|id| !answered.contains(id))
        .map(|id| Entry::Message {
            message: Message {
                role: "tool".into(),
                content: INTERRUPTED.into(),
                tool_call_id: id,
                ..Message::default()
            },
        })
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    journal
        .append(missing)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub fn load(dir: &Path) -> Vec<Entry> {
    if !dir.join("state.json").exists() && !dir.join("journal.jsonl").exists() {
        return legacy(dir);
    }
    Journal::load(dir)
        .map(|records| records.into_iter().map(|record| record.entry).collect())
        .unwrap_or_default()
}

pub fn texts(dir: &Path) -> Vec<(String, String)> {
    let Ok(text) = std::fs::read_to_string(dir.join("conversation.jsonl")) else {
        return legacy(dir)
            .into_iter()
            .filter_map(|entry| match entry {
                Entry::Message { message } => Some((message.role, message.content)),
                _ => None,
            })
            .collect();
    };
    let mut out = Vec::new();
    for line in text.lines() {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let role = match event.get("event").and_then(|kind| kind.as_str()) {
            Some("user") => "user",
            Some("assistant") => "assistant",
            Some("tool_result") => "tool",
            _ => continue,
        };
        let Some(content) = event.get("text").and_then(|text| text.as_str()) else {
            continue;
        };
        if content.trim().is_empty() {
            continue;
        }
        out.push((role.to_string(), content.to_string()));
    }
    out
}

fn import(dir: &Path) -> Result<(), String> {
    if dir.join("state.json").exists() || dir.join("journal.jsonl").exists() {
        return Ok(());
    }
    let entries = legacy(dir);
    if entries.is_empty() {
        return Ok(());
    }
    Journal::seed(dir, entries).map_err(|error| error.to_string())?;
    let _ = std::fs::rename(
        dir.join("transcript.jsonl"),
        dir.join("transcript.imported.jsonl"),
    );
    Ok(())
}

fn legacy(dir: &Path) -> Vec<Entry> {
    let Ok(text) = std::fs::read_to_string(dir.join("transcript.jsonl")) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axe::Message;

    fn dir(tag: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("jimmy-store-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn entry(role: &str, content: &str) -> String {
        let message = Message {
            role: role.into(),
            content: content.into(),
            ..Message::default()
        };
        serde_json::to_string(&Entry::Message { message }).unwrap()
    }

    #[test]
    fn a_running_turn_counts_restarts_and_parks() {
        let dir = dir("restart");
        let mut journal = open(&dir).unwrap();
        let task = new_turn(&mut journal);
        journal.commit(Vec::new(), vec![task.clone()]).unwrap();
        drop(journal);

        for attempt in 1..RESTART_LIMIT {
            assert!(matches!(recover_turn(&dir).unwrap(), Recovered::Resume));
            let journal = Journal::open(&dir).unwrap();
            assert_eq!(journal.task(task.id).unwrap().attempts, attempt);
        }
        match recover_turn(&dir).unwrap() {
            Recovered::Blocked(limit) => assert_eq!(limit, RESTART_LIMIT),
            _ => panic!("esperaba bloqueado"),
        }
        let journal = Journal::open(&dir).unwrap();
        assert_eq!(journal.task(task.id).unwrap().state, TaskState::Blocked);
        drop(journal);
        assert!(matches!(recover_turn(&dir).unwrap(), Recovered::Nothing));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_dangling_intent_becomes_one_interrupted_result() {
        let dir = dir("repair");
        let mut journal = open(&dir).unwrap();
        journal
            .append(vec![Entry::ToolIntent {
                id: "call-1".into(),
                name: "bash".into(),
                arguments: "{}".into(),
            }])
            .unwrap();
        let got = entries(&mut journal).unwrap();
        assert_eq!(got.len(), 2);
        match &got[1] {
            Entry::Message { message } => {
                assert_eq!(message.role, "tool");
                assert_eq!(message.tool_call_id, "call-1");
                assert_eq!(message.content, INTERRUPTED);
            }
            _ => panic!("esperaba un resultado"),
        }
        assert_eq!(entries(&mut journal).unwrap().len(), 2);
    }

    #[test]
    fn a_transcript_is_imported_once_and_renamed() {
        let dir = dir("import");
        let transcript = format!(
            "{}\n{}\n",
            entry("user", "hola"),
            entry("assistant", "listo")
        );
        std::fs::write(dir.join("transcript.jsonl"), &transcript).unwrap();

        let journal = open(&dir).unwrap();
        assert_eq!(journal.entries().len(), 2);
        drop(journal);

        assert!(dir.join("state.json").exists());
        assert!(dir.join("transcript.imported.jsonl").exists());
        assert!(!dir.join("transcript.jsonl").exists());
        assert_eq!(load(&dir).len(), 2);
    }

    #[test]
    fn a_legacy_transcript_without_a_journal_still_reads() {
        let dir = dir("legacy");
        std::fs::write(
            dir.join("transcript.jsonl"),
            format!("{}\n", entry("assistant", "viejo")),
        )
        .unwrap();
        assert_eq!(load(&dir).len(), 1);
    }

    #[test]
    fn texts_come_from_the_event_log() {
        let dir = dir("texts");
        std::fs::write(
            dir.join("conversation.jsonl"),
            "{\"event\":\"user\",\"text\":\"hola\"}\n\
             {\"event\":\"delta\",\"text\":\"no\"}\n\
             {\"event\":\"assistant\",\"text\":\"hola\"}\n\
             {\"event\":\"done\",\"text\":\"hola\"}\n\
             {\"event\":\"tool_result\",\"text\":\"salida\"}\n",
        )
        .unwrap();
        assert_eq!(
            texts(&dir),
            [
                ("user".to_string(), "hola".to_string()),
                ("assistant".to_string(), "hola".to_string()),
                ("tool".to_string(), "salida".to_string()),
            ]
        );
    }

    #[test]
    fn an_import_keeps_working_after_a_snapshot() {
        let dir = dir("snapshot");
        std::fs::write(
            dir.join("transcript.jsonl"),
            format!("{}\n", entry("user", "uno")),
        )
        .unwrap();
        let mut journal = open(&dir).unwrap();
        journal.snapshot().unwrap();
        drop(journal);
        let journal = open(&dir).unwrap();
        assert_eq!(journal.entries().len(), 1);
    }
}
