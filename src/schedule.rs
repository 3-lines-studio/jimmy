use crate::agent::Agent;
use crate::transport::Transport;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const TICK: Duration = Duration::from_secs(60);
const HOUR: i64 = 3_600;
const DAY: i64 = 86_400;
const MAX_RUNS_PER_HOUR: usize = 6;

#[derive(Deserialize)]
struct File {
    #[serde(default)]
    task: Vec<Task>,
}

#[derive(Deserialize)]
struct Task {
    name: String,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    chat: Option<i64>,
    prompt: String,
    #[serde(default)]
    when: Option<String>,
    #[serde(default)]
    at: Option<String>,
    #[serde(default)]
    every: Option<String>,
}

impl Task {
    fn target(&self) -> Result<String, String> {
        if let Some(target) = &self.target {
            return Ok(target.clone());
        }
        if let Some(chat) = self.chat {
            return Ok(chat.to_string());
        }
        Err(format!("la tarea {} no tiene destino", self.name))
    }
}

#[derive(Default, Serialize, Deserialize)]
struct State {
    #[serde(default)]
    tasks: HashMap<String, Run>,
}

#[derive(Default, Serialize, Deserialize)]
struct Run {
    #[serde(default)]
    last_run: i64,
    #[serde(default)]
    last_date: String,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    recent: Vec<i64>,
}

pub fn spawn(transport: Arc<dyn Transport>, agent: Agent, workspace: PathBuf) {
    std::thread::spawn(move || {
        let dir = workspace.join("state");
        let offset = std::env::var("JIMMY_TZ_OFFSET")
            .ok()
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or(0);
        loop {
            if let Err(e) = tick(transport.as_ref(), &agent, &dir, offset) {
                eprintln!("jimmy: agenda: {e}");
            }
            std::thread::sleep(TICK);
        }
    });
}

fn tick(transport: &dyn Transport, agent: &Agent, dir: &Path, offset: i64) -> Result<(), String> {
    let path = dir.join("schedule.toml");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(_) => return Ok(()),
    };
    let file: File = toml::from_str(&text).map_err(|e| format!("schedule.toml: {e}"))?;
    let state_path = dir.join("schedule.state.json");
    let mut state: State = std::fs::read_to_string(&state_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    let now = now_secs();
    let (date, time) = local_parts(now, offset);
    let mut changed = false;
    let mut finished = Vec::new();
    for task in &file.task {
        let run = state.tasks.entry(task.name.clone()).or_default();
        if !due(task, run, now, &date, &time) {
            continue;
        }
        let session = match task
            .target()
            .and_then(|target| transport.parse_target(&target))
        {
            Ok(session) => session,
            Err(e) => {
                eprintln!("jimmy: agenda: {}: {e}", task.name);
                continue;
            }
        };
        run.recent.retain(|stamp| now - stamp < HOUR);
        if run.recent.len() >= MAX_RUNS_PER_HOUR {
            eprintln!("jimmy: agenda: {} superó el tope por hora", task.name);
            continue;
        }
        changed = true;
        run.recent.push(now);
        run.last_run = now;
        run.last_date = date.clone();
        run.done = task.when.is_some();
        eprintln!("jimmy: agenda: corriendo {}", task.name);
        let lock = crate::chat_lock(&session.key());
        let _guard = lock.lock().unwrap();
        if let Err(e) = agent.run_task(transport, &session, &task.prompt) {
            transport.note(&session, &format!("⚠️ la tarea {} falló: {e}", task.name));
        }
        if task.when.is_some() {
            finished.push(task.name.clone());
        }
    }
    if !finished.is_empty() {
        let fresh = std::fs::read_to_string(&path).unwrap_or_default();
        let cleaned = strip_tasks(&fresh, &finished);
        if cleaned != fresh {
            std::fs::write(&path, cleaned).map_err(|e| e.to_string())?;
        }
        for name in &finished {
            state.tasks.remove(name);
        }
        changed = true;
    }
    if changed {
        let text = serde_json::to_string_pretty(&state).map_err(|e| e.to_string())?;
        std::fs::write(&state_path, text).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn due(task: &Task, run: &Run, now: i64, date: &str, time: &str) -> bool {
    if run.done {
        return false;
    }
    if let Some(when) = task.when.as_deref() {
        return format!("{date}T{time}") >= when.trim().replace(' ', "T");
    }
    if let Some(at) = task.at.as_deref() {
        return hhmm(at).is_some_and(|at| time >= at.as_str()) && run.last_date != date;
    }
    if let Some(every) = task.every.as_deref() {
        return period(every)
            .is_some_and(|period| run.last_run == 0 || now - run.last_run >= period);
    }
    false
}

fn strip_tasks(text: &str, names: &[String]) -> String {
    blocks(text)
        .into_iter()
        .filter(|block| match toml::from_str::<File>(block) {
            Ok(file) => !file.task.iter().any(|task| names.contains(&task.name)),
            Err(_) => true,
        })
        .collect()
}

fn blocks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        if !current.is_empty() && line.trim_start().starts_with("[[task]]") {
            out.push(std::mem::take(&mut current));
        }
        current.push_str(line);
        current.push('\n');
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn hhmm(s: &str) -> Option<String> {
    let (hour, minute) = s.trim().split_once(':')?;
    let hour: u32 = hour.trim().parse().ok()?;
    let minute: u32 = minute.trim().parse().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    Some(format!("{hour:02}:{minute:02}"))
}

fn period(s: &str) -> Option<i64> {
    let s = s.trim();
    let unit = s.chars().last()?;
    let value: i64 = s[..s.len() - unit.len_utf8()].trim().parse().ok()?;
    let factor = match unit {
        's' => 1,
        'm' => 60,
        'h' => HOUR,
        'd' => DAY,
        _ => return None,
    };
    Some(value * factor)
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn local_parts(now: i64, offset: i64) -> (String, String) {
    let local = now + offset * HOUR;
    let days = local.div_euclid(DAY);
    let secs = local.rem_euclid(DAY);
    let (year, month, day) = civil_from_days(days);
    (
        format!("{year:04}-{month:02}-{day:02}"),
        format!("{:02}:{:02}", secs / HOUR, secs % HOUR / 60),
    )
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(when: Option<&str>, at: Option<&str>, every: Option<&str>) -> Task {
        Task {
            name: "t".into(),
            target: None,
            chat: Some(1),
            prompt: "p".into(),
            when: when.map(String::from),
            at: at.map(String::from),
            every: every.map(String::from),
        }
    }

    #[test]
    fn task_target_falls_back_to_chat() {
        let mut t = task(None, Some("09:00"), None);
        assert_eq!(t.target().unwrap(), "1");
        t.target = Some("C1/1.2".into());
        assert_eq!(t.target().unwrap(), "C1/1.2");
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
    }

    #[test]
    fn local_time_applies_offset() {
        assert_eq!(local_parts(0, 0), ("1970-01-01".into(), "00:00".into()));
        assert_eq!(local_parts(0, -3), ("1969-12-31".into(), "21:00".into()));
    }

    #[test]
    fn one_shot_fires_once_after_its_time() {
        let t = task(Some("2026-09-14T15:00"), None, None);
        assert!(!due(&t, &Run::default(), 0, "2026-09-14", "14:59"));
        assert!(due(&t, &Run::default(), 0, "2026-09-14", "15:00"));
        let run = Run {
            done: true,
            ..Run::default()
        };
        assert!(!due(&t, &run, 0, "2026-09-15", "10:00"));
    }

    #[test]
    fn daily_fires_once_per_date() {
        let t = task(None, Some("09:00"), None);
        let run = Run {
            last_date: "2026-09-14".into(),
            ..Run::default()
        };
        assert!(due(&t, &run, 0, "2026-09-15", "09:00"));
        assert!(!due(&t, &run, 0, "2026-09-14", "10:00"));
        assert!(!due(&t, &run, 0, "2026-09-15", "08:59"));
    }

    #[test]
    fn interval_uses_last_run() {
        let t = task(None, None, Some("6h"));
        assert!(due(&t, &Run::default(), 1_000, "2026-09-14", "00:00"));
        let run = Run {
            last_run: 1_000,
            ..Run::default()
        };
        assert!(!due(&t, &run, 1_000 + HOUR, "2026-09-14", "00:00"));
        assert!(due(&t, &run, 1_000 + 6 * HOUR, "2026-09-14", "00:00"));
    }

    #[test]
    fn strips_finished_one_shot_blocks() {
        let text = "# tareas\n\n[[task]]\nname = \"a\"\nchat = 1\nwhen = \"2026-01-01T00:00\"\nprompt = \"p\"\n\n[[task]]\nname = \"b\"\nchat = 1\nat = \"09:00\"\nprompt = \"q\"\n";
        let out = strip_tasks(text, &["a".to_string()]);
        assert!(out.starts_with("# tareas"));
        let file: File = toml::from_str(&out).unwrap();
        assert_eq!(file.task.len(), 1);
        assert_eq!(file.task[0].name, "b");
    }

    #[test]
    fn parses_a_task_file() {
        let file: File = toml::from_str(
            r#"
[[task]]
name = "morning"
chat = 123
at = "09:00"
prompt = "hi"

[[task]]
name = "once"
chat = 456
when = "2026-09-14T15:00"
prompt = "bye"
"#,
        )
        .unwrap();
        assert_eq!(file.task.len(), 2);
        assert_eq!(file.task[0].name, "morning");
        assert_eq!(file.task[0].at.as_deref(), Some("09:00"));
        assert_eq!(file.task[1].when.as_deref(), Some("2026-09-14T15:00"));
        assert!(file.task[0].every.is_none());
    }

    #[test]
    fn parses_hhmm_and_periods() {
        assert_eq!(hhmm("9:5").as_deref(), Some("09:05"));
        assert_eq!(hhmm("24:00"), None);
        assert_eq!(period("6h"), Some(6 * HOUR));
        assert_eq!(period("30m"), Some(1_800));
        assert_eq!(period("2d"), Some(2 * DAY));
        assert_eq!(period("cada rato"), None);
    }
}
