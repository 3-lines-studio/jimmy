use crate::agent::Agent;
use crate::transport::{Null, Session, Transport};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const TICK: Duration = Duration::from_secs(60);
const HOUR: i64 = 3_600;
const DAY: i64 = 86_400;
const MAX_RUNS_PER_HOUR: usize = 6;
const KEEP: usize = 20;
const DIR: &str = "schedule";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub silent: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub paused: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Run {
    pub ts: i64,
    pub date: String,
    pub ms: u64,
    pub ok: bool,
    pub text: String,
}

pub struct Entry {
    pub name: String,
    pub task: Task,
    pub runs: Vec<Run>,
}

fn is_false(value: &bool) -> bool {
    !value
}

pub fn list(dir: &Path) -> Vec<Entry> {
    let mut entries: Vec<Entry> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|kind| kind == "toml"))
        .filter_map(|entry| {
            let name = entry.path().file_stem()?.to_string_lossy().to_string();
            let task = read_task(&entry.path())?;
            let runs = read_runs(dir, &name);
            Some(Entry { name, task, runs })
        })
        .collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

pub fn set_paused(dir: &Path, name: &str, paused: bool) -> Result<(), String> {
    let path = task_path(dir, name);
    let mut task = read_task(&path).ok_or_else(|| format!("no existe la tarea {name}"))?;
    task.paused = paused;
    let text = toml::to_string(&task).map_err(|e| e.to_string())?;
    axe::atomic_write(&path, text.as_bytes()).map_err(|e| e.to_string())
}

pub fn spawn(transport: Arc<dyn Transport>, agent: Agent, workspace: PathBuf) -> Sender<String> {
    let (sender, runner) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let dir = workspace.join("state").join(DIR);
        migrate(&dir);
        let offset = std::env::var("JIMMY_TZ_OFFSET")
            .ok()
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or(0);
        let mut pending: Option<String> = None;
        loop {
            let mut asked: Vec<String> = pending.take().into_iter().collect();
            while let Ok(name) = runner.try_recv() {
                asked.push(name);
            }
            asked.sort();
            asked.dedup();
            tick(transport.as_ref(), &agent, &dir, offset, &asked);
            pending = runner.recv_timeout(TICK).ok();
        }
    });
    sender
}

fn tick(transport: &dyn Transport, agent: &Agent, dir: &Path, offset: i64, asked: &[String]) {
    let now = now_secs();
    let (date, time) = local_parts(now, offset);
    for entry in list(dir) {
        if entry.task.paused {
            continue;
        }
        let manual = asked.contains(&entry.name);
        if !manual && !due(&entry.task, &entry.runs, now, &date, &time) {
            continue;
        }
        let recent = entry.runs.iter().filter(|run| now - run.ts < HOUR).count();
        if !manual && recent >= MAX_RUNS_PER_HOUR {
            eprintln!("jimmy: agenda: {} superó el tope por hora", entry.name);
            continue;
        }
        eprintln!("jimmy: agenda: corriendo {}", entry.name);
        run(transport, agent, dir, &entry, &date, now);
    }
}

fn run(transport: &dyn Transport, agent: &Agent, dir: &Path, entry: &Entry, date: &str, now: i64) {
    let session = match &entry.task.target {
        Some(target) => match transport.parse_target(target) {
            Ok(session) => session,
            Err(e) => {
                eprintln!("jimmy: agenda: {}: {e}", entry.name);
                return;
            }
        },
        None => Session::channel(entry.name.as_str()),
    };
    let transport: &dyn Transport = match entry.task.target {
        Some(_) => transport,
        None => &Null,
    };
    let started = Instant::now();
    let result = agent.run_task(transport, &session, &entry.task.prompt, entry.task.silent);
    let (ok, text) = match result {
        Ok(reply) => (true, reply),
        Err(e) => (false, e),
    };
    record(
        dir,
        &entry.name,
        Run {
            ts: now,
            date: date.to_string(),
            ms: started.elapsed().as_millis() as u64,
            ok,
            text,
        },
    );
}

fn due(task: &Task, runs: &[Run], now: i64, date: &str, time: &str) -> bool {
    if let Some(when) = task.when.as_deref() {
        return runs.is_empty() && format!("{date}T{time}") >= when.trim().replace(' ', "T");
    }
    if let Some(at) = task.at.as_deref() {
        return hhmm(at).is_some_and(|at| time >= at.as_str()) && last_date(runs) != Some(date);
    }
    if let Some(every) = task.every.as_deref() {
        return period(every)
            .is_some_and(|period| last_run(runs) == 0 || now - last_run(runs) >= period);
    }
    false
}

fn last_run(runs: &[Run]) -> i64 {
    runs.last().map(|run| run.ts).unwrap_or(0)
}

fn last_date(runs: &[Run]) -> Option<&str> {
    runs.last().map(|run| run.date.as_str())
}

fn task_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.toml"))
}

fn runs_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.jsonl"))
}

fn read_task(path: &Path) -> Option<Task> {
    let text = std::fs::read_to_string(path).ok()?;
    match toml::from_str(&text) {
        Ok(task) => Some(task),
        Err(e) => {
            eprintln!("jimmy: agenda: {}: {e}", path.display());
            None
        }
    }
}

fn read_runs(dir: &Path, name: &str) -> Vec<Run> {
    let Ok(text) = std::fs::read_to_string(runs_path(dir, name)) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn record(dir: &Path, name: &str, run: Run) {
    let mut runs = read_runs(dir, name);
    runs.push(run);
    if runs.len() > KEEP {
        runs.drain(..runs.len() - KEEP);
    }
    let text: String = runs
        .iter()
        .filter_map(|run| serde_json::to_string(run).ok())
        .map(|line| line + "\n")
        .collect();
    let _ = axe::atomic_write(&runs_path(dir, name), text.as_bytes());
}

#[derive(Deserialize)]
struct Legacy {
    #[serde(default)]
    task: Vec<LegacyTask>,
}

#[derive(Deserialize)]
struct LegacyTask {
    name: String,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    chat: Option<i64>,
    prompt: String,
    #[serde(default)]
    silent: bool,
    #[serde(default)]
    when: Option<String>,
    #[serde(default)]
    at: Option<String>,
    #[serde(default)]
    every: Option<String>,
}

fn migrate(dir: &Path) {
    let Some(state) = dir.parent() else {
        return;
    };
    let legacy = state.join("schedule.toml");
    let Ok(text) = std::fs::read_to_string(&legacy) else {
        return;
    };
    let Ok(file) = toml::from_str::<Legacy>(&text) else {
        eprintln!("jimmy: agenda: no pude leer {}", legacy.display());
        return;
    };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    for task in file.task {
        let target = task
            .target
            .or_else(|| task.chat.map(|chat| chat.to_string()));
        let converted = Task {
            when: task.when,
            at: task.at,
            every: task.every,
            target,
            prompt: task.prompt,
            silent: task.silent,
            paused: false,
        };
        let path = task_path(dir, &task.name);
        if path.exists() {
            continue;
        }
        if let Ok(text) = toml::to_string(&converted) {
            let _ = axe::atomic_write(&path, text.as_bytes());
        }
    }
    let _ = std::fs::rename(&legacy, state.join("schedule.toml.old"));
    let _ = std::fs::remove_file(state.join("schedule.state.json"));
    eprintln!(
        "jimmy: agenda: migré {} a {}",
        legacy.display(),
        dir.display()
    );
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

    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("jimmy-agenda-{tag}-{}", crate::random::hex(4)));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn task(when: Option<&str>, at: Option<&str>, every: Option<&str>) -> Task {
        Task {
            when: when.map(String::from),
            at: at.map(String::from),
            every: every.map(String::from),
            target: None,
            prompt: "p".into(),
            silent: false,
            paused: false,
        }
    }

    fn run(ts: i64, date: &str) -> Run {
        Run {
            ts,
            date: date.into(),
            ms: 10,
            ok: true,
            text: "ok".into(),
        }
    }

    #[test]
    fn one_shot_fires_once_after_its_time() {
        let t = task(Some("2026-09-14T15:00"), None, None);
        assert!(!due(&t, &[], 0, "2026-09-14", "14:59"));
        assert!(due(&t, &[], 0, "2026-09-14", "15:00"));
        assert!(!due(&t, &[run(0, "2026-09-14")], 0, "2026-09-15", "10:00"));
    }

    #[test]
    fn daily_fires_once_per_date() {
        let t = task(None, Some("09:00"), None);
        let done = [run(0, "2026-09-14")];
        assert!(due(&t, &done, 0, "2026-09-15", "09:00"));
        assert!(!due(&t, &done, 0, "2026-09-14", "10:00"));
        assert!(!due(&t, &[], 0, "2026-09-15", "08:59"));
    }

    #[test]
    fn interval_uses_last_run() {
        let t = task(None, None, Some("6h"));
        assert!(due(&t, &[], 1_000, "2026-09-14", "00:00"));
        let done = [run(1_000, "2026-09-14")];
        assert!(!due(&t, &done, 1_000 + HOUR, "2026-09-14", "00:00"));
        assert!(due(&t, &done, 1_000 + 6 * HOUR, "2026-09-14", "00:00"));
    }

    #[test]
    fn a_task_without_a_schedule_never_fires() {
        assert!(!due(&task(None, None, None), &[], 0, "2026-09-14", "10:00"));
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
    fn reads_a_task_per_file_and_ignores_a_broken_one() {
        let dir = scratch("read");
        std::fs::write(
            dir.join("morning.toml"),
            "at = \"09:00\"\nprompt = \"hola\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("night.toml"),
            "prompt = \"chau\"\nevery = \"6h\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("roto.toml"), "at = ").unwrap();
        std::fs::write(dir.join("notas.md"), "no es una tarea").unwrap();

        let entries = list(&dir);
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["morning", "night"]);
        assert_eq!(entries[0].task.at.as_deref(), Some("09:00"));
        assert_eq!(entries[1].task.every.as_deref(), Some("6h"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn keeps_the_last_runs_and_reads_them_back() {
        let dir = scratch("runs");
        for ts in 0..(KEEP as i64 + 3) {
            record(&dir, "t", run(ts, "2026-09-14"));
        }
        let runs = read_runs(&dir, "t");
        assert_eq!(runs.len(), KEEP);
        assert_eq!(runs.first().unwrap().ts, 3);
        assert_eq!(last_run(&runs), (KEEP as i64) + 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pausing_rewrites_the_file_without_losing_anything() {
        let dir = scratch("pause");
        std::fs::write(
            dir.join("t.toml"),
            "at = \"09:00\"\ntarget = \"123\"\nprompt = \"hola\"\nsilent = true\n",
        )
        .unwrap();

        set_paused(&dir, "t", true).unwrap();
        let entry = &list(&dir)[0];
        assert!(entry.task.paused);
        assert!(entry.task.silent);
        assert_eq!(entry.task.target.as_deref(), Some("123"));
        assert_eq!(entry.task.prompt, "hola");
        assert_eq!(entry.task.at.as_deref(), Some("09:00"));

        set_paused(&dir, "t", false).unwrap();
        assert!(!list(&dir)[0].task.paused);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_unknown_task_cannot_be_paused() {
        let dir = scratch("unknown");
        assert!(set_paused(&dir, "no-existe", true).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn migrates_the_single_file_into_one_per_task() {
        let root = scratch("migrate");
        let dir = root.join(DIR);
        std::fs::write(
            root.join("schedule.toml"),
            "[[task]]\nname = \"memoria\"\nchat = 123\nat = \"05:00\"\nprompt = \"p\"\n\n[[task]]\nname = \"limpieza\"\nevery = \"6h\"\nprompt = \"q\"\nsilent = true\n",
        )
        .unwrap();
        std::fs::write(root.join("schedule.state.json"), "{}").unwrap();

        migrate(&dir);
        let entries = list(&dir);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "limpieza");
        assert!(entries[0].task.silent);
        assert_eq!(entries[1].name, "memoria");
        assert_eq!(entries[1].task.target.as_deref(), Some("123"));
        assert!(root.join("schedule.toml.old").exists());
        assert!(!root.join("schedule.toml").exists());
        assert!(!root.join("schedule.state.json").exists());

        migrate(&dir);
        assert_eq!(list(&dir).len(), 2);
        std::fs::remove_dir_all(&root).ok();
    }
}
