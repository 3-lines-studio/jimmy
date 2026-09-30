use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Cuántas entradas del proyecto en curso entran al prompt. El resto vive en su
/// archivo y vuelve cuando el tema vuelve: no hay tope que desaloje nada.
const PROJECT_ENTRIES: usize = 2;
/// Los tipos con los que se clasifica una entrada. La lista es corta a
/// propósito: con el vocabulario abierto cada entrada inventaba el suyo y el
/// tipo terminaba sin servir para nada.
const KINDS: [&str; 8] = [
    "decision",
    "estado",
    "medicion",
    "bugfix",
    "herramienta",
    "identidad",
    "proyecto",
    "plataforma",
];

#[derive(Clone, Serialize, Deserialize)]
struct Record {
    ts: i64,
    key: String,
    kind: String,
    body: String,
    rev: u32,
    last_seen: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    left: Option<String>,
}

struct Entry {
    key: String,
    kind: String,
    date: i64,
    body: String,
}

struct Level1 {
    entries: Vec<Entry>,
    malformed: usize,
}

fn level1_path(workspace: &Path) -> PathBuf {
    workspace.join("notes/memory.md")
}

fn level2_path(workspace: &Path) -> PathBuf {
    workspace.join("notes/memory.jsonl")
}

/// Los hechos transversales: uno por archivo, y todos entran al prompt.
fn facts_dir(workspace: &Path) -> PathBuf {
    workspace.join("notes/memory")
}

/// Los hechos de cada proyecto, un archivo por proyecto. Viven fuera del clon
/// porque un `git clean -fd` se llevaría lo que no esté commiteado.
fn projects_dir(workspace: &Path) -> PathBuf {
    workspace.join("notes/projects")
}

fn project_path(workspace: &Path, project: &str) -> PathBuf {
    projects_dir(workspace).join(format!("{project}.md"))
}

fn events_path(workspace: &Path) -> PathBuf {
    workspace.join("state/memory-events.jsonl")
}

fn md_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .collect();
    paths.sort();
    paths
}

fn read(path: &Path) -> Level1 {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(_) => Level1 {
            entries: Vec::new(),
            malformed: 0,
        },
    }
}

fn read_all(dir: &Path) -> Level1 {
    let mut level = Level1 {
        entries: Vec::new(),
        malformed: 0,
    };
    for path in md_files(dir) {
        let part = read(&path);
        level.entries.extend(part.entries);
        level.malformed += part.malformed;
    }
    level
}

fn merge(into: &mut Level1, other: Level1) {
    into.entries.extend(other.entries);
    into.malformed += other.malformed;
}

/// Los hechos transversales: uno por archivo, y todos entran al prompt. Mientras
/// no exista la carpeta, el `notes/memory.md` de antes.
fn transversal(workspace: &Path) -> Level1 {
    let dir = facts_dir(workspace);
    if !dir.is_dir() {
        return read(&level1_path(workspace));
    }
    read_all(&dir)
}

fn of_project(workspace: &Path, project: &str) -> Vec<Entry> {
    if project.is_empty() {
        return Vec::new();
    }
    read(&project_path(workspace, project)).entries
}

fn every_fact(workspace: &Path) -> Level1 {
    let mut level = transversal(workspace);
    merge(&mut level, read_all(&projects_dir(workspace)));
    level
}

fn format_entry(entry: &Entry) -> String {
    format!(
        "## {} · {} · {}\n\n{}\n",
        entry.key,
        entry.kind,
        format_date(entry.date),
        entry.body.trim()
    )
}

fn text_of(entries: &[Entry]) -> String {
    entries
        .iter()
        .map(format_entry)
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn render(workspace: &Path, project: &str) -> String {
    let mut entries = transversal(workspace).entries;
    let mut mine = of_project(workspace, project);
    mine.sort_by_key(|entry| std::cmp::Reverse(entry.date));
    let outside: Vec<String> = mine
        .iter()
        .skip(PROJECT_ENTRIES)
        .map(|entry| entry.key.clone())
        .collect();
    entries.extend(mine.into_iter().take(PROJECT_ENTRIES));
    if entries.is_empty() {
        return String::new();
    }
    let text = text_of(&entries);
    emit(
        workspace,
        json!({
            "kind": "memory",
            "op": "render",
            "ts": now(),
            "project": project,
            "bytes": text.len(),
            "entries": entries.len(),
            "outside": outside,
        }),
    );
    if outside.is_empty() {
        return text.trim().to_string();
    }
    // Lo que quedó afuera está en el archivo del proyecto: el aviso sirve para
    // saber que existe, no para pedir que alguien consolide nada.
    format!(
        "{}\n\n[en `notes/projects/{project}.md` y afuera del prompt: {}]",
        text.trim(),
        outside.join(", ")
    )
}

pub fn list(workspace: &Path) -> String {
    let mut paths = Vec::new();
    let dir = facts_dir(workspace);
    if dir.is_dir() {
        paths.extend(md_files(&dir));
    } else {
        paths.push(level1_path(workspace));
    }
    paths.extend(md_files(&projects_dir(workspace)));
    let mut out = Vec::new();
    for path in paths {
        let entries = read(&path).entries;
        if entries.is_empty() {
            continue;
        }
        let name = path
            .strip_prefix(workspace)
            .unwrap_or(&path)
            .display()
            .to_string();
        out.push(name);
        for entry in entries {
            out.push(format!(
                "  {} · {} · {}",
                entry.key,
                entry.kind,
                format_date(entry.date)
            ));
        }
    }
    out.join("\n")
}

pub fn show(workspace: &Path, key: &str) -> Result<String, String> {
    let key = key.trim();
    if let Some(entry) = every_fact(workspace).entries.iter().find(|e| e.key == key) {
        return Ok(format_entry(entry).trim_end().to_string());
    }
    let (records, _) = load_records(&level2_path(workspace));
    let Some(record) = records.iter().rev().find(|record| record.key == key) else {
        return Err(format!("{key} no está en la memoria"));
    };
    Ok(format!(
        "## {} · {} · {} (nivel 2)\n{}",
        record.key,
        record.kind,
        format_date(record.last_seen),
        record.body.trim()
    ))
}

pub fn add(workspace: &Path, key: &str, kind: &str, text: &str) -> Result<String, String> {
    let key = key.trim();
    let kind = kind.trim();
    let text = text.trim();
    if !valid_key(key) {
        return Err(format!(
            "clave inválida: {key} (minúsculas, números, `-` y `familia/tema`)"
        ));
    }
    if !KINDS.contains(&kind) {
        return Err(format!(
            "tipo desconocido: {kind} (los que hay: {})",
            KINDS.join(", ")
        ));
    }
    if text.is_empty() {
        return Err("el hecho está vacío".into());
    }
    let path = match key.split_once('/') {
        Some((family, _)) => {
            known_project(workspace, family)?;
            project_path(workspace, family)
        }
        None => facts_dir(workspace).join(format!("{key}.md")),
    };
    let mut entries = read(&path).entries;
    let date = today();
    match entries.iter().position(|entry| entry.key == key) {
        Some(index) => {
            entries[index].kind = kind.to_string();
            entries[index].body = text.to_string();
            entries[index].date = date;
        }
        None => entries.insert(
            0,
            Entry {
                key: key.to_string(),
                kind: kind.to_string(),
                date,
                body: text.to_string(),
            },
        ),
    }
    write_entries(&path, &entries)?;
    Ok(format!(
        "{} · {kind} · {}",
        path.display(),
        format_date(date)
    ))
}

pub fn migrate(workspace: &Path) -> Result<String, String> {
    let mut moved = 0;
    for entry in read(&level1_path(workspace)).entries {
        let path = match entry.key.split_once('/') {
            Some((family, _)) => project_path(workspace, family),
            None => facts_dir(workspace).join(format!("{}.md", entry.key)),
        };
        let mut entries = read(&path).entries;
        if entries.iter().any(|present| present.key == entry.key) {
            continue;
        }
        entries.insert(0, entry);
        write_entries(&path, &entries)?;
        moved += 1;
    }
    Ok(format!(
        "migrate: {moved} hechos a notes/memory y notes/projects"
    ))
}

fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.chars().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '/' || c == '.'
        })
        && !key.starts_with('/')
        && !key.ends_with('/')
        && !key.contains("//")
}

fn known_project(workspace: &Path, family: &str) -> Result<(), String> {
    let mut known: Vec<String> = md_files(&projects_dir(workspace))
        .iter()
        .filter_map(|path| {
            path.file_stem()
                .map(|name| name.to_string_lossy().to_string())
        })
        .collect();
    let clones = std::fs::read_dir(workspace.join("projects"));
    for entry in clones.into_iter().flatten().flatten() {
        if entry.path().is_dir() {
            known.push(entry.file_name().to_string_lossy().to_string());
        }
    }
    known.sort();
    known.dedup();
    if known.iter().any(|name| name == family) {
        return Ok(());
    }
    let near = known
        .iter()
        .min_by_key(|name| distance(name, family))
        .filter(|name| distance(name, family) <= 2);
    let hint = match near {
        Some(name) => format!(" (¿{name}?)"),
        None => format!(" (los que hay: {})", known.join(", ")),
    };
    Err(format!("no conozco el proyecto {family}{hint}"))
}

fn distance(a: &str, b: &str) -> usize {
    let right: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=right.len()).collect();
    for (i, left) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, other) in right.iter().enumerate() {
            let previous = row[j + 1];
            row[j + 1] = if left == *other {
                diagonal
            } else {
                1 + diagonal.min(row[j]).min(row[j + 1])
            };
            diagonal = previous;
        }
    }
    row[right.len()]
}

fn write_entries(path: &Path, entries: &[Entry]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    axe::atomic_write(path, text_of(entries).as_bytes())
        .map_err(|e| format!("{}: {e}", path.display()))
}

pub fn sync(workspace: &Path) -> Result<String, String> {
    let started = std::time::Instant::now();
    std::fs::create_dir_all(workspace.join("notes")).map_err(|e| e.to_string())?;
    let level1 = every_fact(workspace);
    if level1.entries.is_empty() {
        return Ok("sync: no hay hechos".into());
    }
    let (records, corrupt) = load_records(&level2_path(workspace));
    let latest = latest(&records);

    let mut fresh = Vec::new();
    let (mut created, mut updated, mut reasserted, mut promoted) = (0, 0, 0, 0);
    for entry in &level1.entries {
        let Some(previous) = latest.get(&entry.key) else {
            created += 1;
            fresh.push(new_record(entry, 1, None));
            continue;
        };
        if previous.left.is_some() {
            promoted += 1;
            fresh.push(new_record(entry, previous.rev + 1, None));
            continue;
        }
        if previous.body != entry.body || previous.kind != entry.kind {
            updated += 1;
            fresh.push(new_record(entry, previous.rev + 1, None));
            continue;
        }
        if previous.last_seen != entry.date {
            reasserted += 1;
            fresh.push(new_record(entry, previous.rev + 1, None));
        }
    }

    let present: Vec<&str> = level1.entries.iter().map(|e| e.key.as_str()).collect();
    let mut unknown: Vec<&str> = level1
        .entries
        .iter()
        .map(|entry| entry.kind.as_str())
        .filter(|kind| !KINDS.contains(kind))
        .collect();
    unknown.sort_unstable();
    unknown.dedup();
    let mut dropped = 0;
    for (key, previous) in &latest {
        if previous.left.is_none() && !present.contains(&key.as_str()) {
            dropped += 1;
            fresh.push(Record {
                ts: now(),
                left: Some("dropped".into()),
                ..previous.clone()
            });
        }
    }

    append_records(&level2_path(workspace), &fresh)?;
    let bytes = text_of(&level1.entries).len();
    let entries = level1.entries.len();
    emit(
        workspace,
        json!({
            "kind": "memory",
            "op": "sync",
            "ts": now(),
            "ms": started.elapsed().as_millis() as u64,
            "bytes": bytes,
            "entries": entries,
            "created": created,
            "updated": updated,
            "reasserted": reasserted,
            "promoted": promoted,
            "dropped": dropped,
            "kinds": unknown,
            "malformed": level1.malformed,
            "corrupt": corrupt,
        }),
    );

    let mut report = format!(
        "sync: {entries} hechos, {bytes} bytes · {created} nuevos · {updated} actualizados · {reasserted} reafirmados · {promoted} vueltas"
    );
    if dropped > 0 {
        report.push_str(&format!(" · {dropped} borradas a mano"));
    }
    if !unknown.is_empty() {
        report.push_str(&format!(
            " · tipos fuera de la lista: {}",
            unknown.join(", ")
        ));
    }
    if level1.malformed > 0 {
        report.push_str(&format!(" · {} con encabezado inválido", level1.malformed));
    }
    if corrupt > 0 {
        report.push_str(&format!(" · {corrupt} líneas corruptas en el nivel 2"));
    }
    Ok(report)
}

pub fn miss(workspace: &Path, text: &str) -> Result<String, String> {
    std::fs::create_dir_all(workspace.join("state")).map_err(|e| e.to_string())?;
    emit(
        workspace,
        json!({"kind": "memory", "op": "miss", "ts": now(), "text": text}),
    );
    Ok(format!("miss anotado: {text}"))
}

fn new_record(entry: &Entry, rev: u32, left: Option<String>) -> Record {
    Record {
        ts: now(),
        key: entry.key.clone(),
        kind: entry.kind.clone(),
        body: entry.body.clone(),
        rev,
        last_seen: entry.date,
        left,
    }
}

fn parse(text: &str) -> Level1 {
    let mut malformed = 0;
    let mut entries: Vec<Entry> = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            match heading(rest.trim_end()) {
                Some((key, kind, date)) => entries.push(Entry {
                    key,
                    kind,
                    date,
                    body: String::new(),
                }),
                None => malformed += 1,
            }
            continue;
        }
        if let Some(entry) = entries.last_mut() {
            entry.body.push_str(line);
            entry.body.push('\n');
        }
    }
    for entry in &mut entries {
        entry.body = entry.body.trim().to_string();
    }
    Level1 { entries, malformed }
}

fn heading(line: &str) -> Option<(String, String, i64)> {
    let parts: Vec<&str> = line.split('·').map(str::trim).collect();
    if parts.len() != 3 {
        return None;
    }
    let key = parts[0];
    if key.is_empty() {
        return None;
    }
    let valid = key
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '/' || c == '.');
    if !valid {
        return None;
    }
    Some((key.to_string(), parts[1].to_string(), parse_date(parts[2])?))
}

fn load_records(path: &Path) -> (Vec<Record>, usize) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return (Vec::new(), 0);
    };
    let mut records = Vec::new();
    let mut corrupt = 0;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Record>(line) {
            Ok(record) => records.push(record),
            Err(_) => corrupt += 1,
        }
    }
    (records, corrupt)
}

fn latest(records: &[Record]) -> BTreeMap<String, Record> {
    let mut map = BTreeMap::new();
    for record in records {
        map.insert(record.key.clone(), record.clone());
    }
    map
}

fn append_records(path: &Path, records: &[Record]) -> Result<(), String> {
    if records.is_empty() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let mut buffer = String::new();
    for record in records {
        buffer.push_str(&serde_json::to_string(record).map_err(|e| e.to_string())?);
        buffer.push('\n');
    }
    file.write_all(buffer.as_bytes())
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn emit(workspace: &Path, event: serde_json::Value) {
    let path = events_path(workspace);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let mut line = event.to_string();
    line.push('\n');
    let _ = file.write_all(line.as_bytes());
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn today() -> i64 {
    now() / 86_400
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = (month + 9) % 12;
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

fn parse_date(text: &str) -> Option<i64> {
    let parts: Vec<&str> = text.split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    let year = parts[0].parse::<i64>().ok()?;
    let month = parts[1].parse::<i64>().ok()?;
    let day = parts[2].parse::<i64>().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(days_from_civil(year, month, day))
}

fn format_date(days: i64) -> String {
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("jimmy-memo-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("notes")).unwrap();
        std::fs::create_dir_all(path.join("state")).unwrap();
        path
    }

    fn write_level1(workspace: &Path, text: &str) {
        std::fs::write(level1_path(workspace), text).unwrap();
    }

    fn stored(workspace: &Path) -> Vec<Record> {
        load_records(&level2_path(workspace)).0
    }

    fn date(days_ago: i64) -> String {
        format_date(today() - days_ago)
    }

    fn entry(key: &str, days_ago: i64, body: &str) -> String {
        let day = date(days_ago);
        format!("## {key} · decision · {day}\n{body}\n\n")
    }

    fn fact(workspace: &Path, key: &str, days_ago: i64, body: &str) {
        let path = if key.contains('/') {
            project_path(workspace, key.split('/').next().unwrap())
        } else {
            facts_dir(workspace).join(format!("{key}.md"))
        };
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let before = std::fs::read_to_string(&path).unwrap_or_default();
        std::fs::write(&path, format!("{}{before}", entry(key, days_ago, body))).unwrap();
    }

    #[test]
    fn date_roundtrip_handles_leap_years() {
        for days in [-10_000, 0, 19_000, 20_000] {
            let (year, month, day) = civil_from_days(days);
            assert_eq!(days_from_civil(year, month, day), days);
        }
        assert_eq!(format_date(days_from_civil(2024, 2, 29)), "2024-02-29");
        assert_eq!(parse_date("2024-02-30"), Some(days_from_civil(2024, 3, 1)));
        assert_eq!(parse_date("2026-13-01"), None);
        assert_eq!(parse_date("ayer"), None);
    }

    #[test]
    fn parse_reads_heading_fields_and_body() {
        let first = date(3);
        let second = date(9);
        let text = format!(
            "contexto suelto\n\n## jimmy/telemetria · decision · {first}\nuna linea\notra\n\n## usuario · identidad · {second}\nvive en caba\n"
        );
        let level1 = parse(&text);
        assert_eq!(level1.entries.len(), 2);
        assert_eq!(level1.malformed, 0);
        assert_eq!(level1.entries[0].key, "jimmy/telemetria");
        assert_eq!(level1.entries[0].kind, "decision");
        assert_eq!(level1.entries[0].body, "una linea\notra");
        assert_eq!(level1.entries[1].key, "usuario");
        assert_eq!(level1.entries[1].body, "vive en caba");
    }

    #[test]
    fn parse_flags_malformed_heading() {
        let text =
            "## sin fecha\ncuerpo\n\n## Mal Con Mayusculas · decision · 2026-01-01\ncuerpo\n";
        let level1 = parse(text);
        assert!(level1.entries.is_empty());
        assert_eq!(level1.malformed, 2);
    }

    #[test]
    fn sync_records_new_entries() {
        let workspace = workspace("sync-new");
        write_level1(&workspace, &entry("a", 1, "cuerpo a"));
        let report = sync(&workspace).unwrap();
        assert!(report.contains("1 nuevos"));
        let records = stored(&workspace);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].key, "a");
        assert_eq!(records[0].rev, 1);
        assert!(records[0].left.is_none());
    }

    #[test]
    fn sync_bumps_revision_on_body_change() {
        let workspace = workspace("sync-update");
        write_level1(&workspace, &entry("a", 1, "cuerpo a"));
        sync(&workspace).unwrap();
        write_level1(&workspace, &entry("a", 1, "cuerpo corregido"));
        sync(&workspace).unwrap();
        let records = stored(&workspace);
        assert_eq!(records.len(), 2);
        assert_eq!(records[1].rev, 2);
        assert_eq!(records[1].body, "cuerpo corregido");
    }

    #[test]
    fn sync_bumps_revision_on_reassertion() {
        let workspace = workspace("sync-reassert");
        write_level1(&workspace, &entry("a", 20, "cuerpo a"));
        sync(&workspace).unwrap();
        write_level1(&workspace, &entry("a", 0, "cuerpo a"));
        sync(&workspace).unwrap();
        let records = stored(&workspace);
        assert_eq!(records.len(), 2);
        assert_eq!(records[1].rev, 2);
        assert_eq!(records[1].body, "cuerpo a");
    }

    #[test]
    fn sync_reports_manual_drop_once() {
        let workspace = workspace("sync-drop");
        let both = format!("{}{}", entry("a", 1, "cuerpo a"), entry("b", 1, "cuerpo b"));
        write_level1(&workspace, &both);
        sync(&workspace).unwrap();
        write_level1(&workspace, &entry("a", 1, "cuerpo a"));
        let report = sync(&workspace).unwrap();
        assert!(report.contains("1 borradas a mano"));
        assert_eq!(
            stored(&workspace).last().unwrap().left.as_deref(),
            Some("dropped")
        );
        let second = sync(&workspace).unwrap();
        assert!(!second.contains("borradas a mano"));
    }

    #[test]
    fn render_is_the_facts_plus_the_newest_of_the_project() {
        let workspace = workspace("render-facts");
        fact(&workspace, "usuario", 10, "quién es");
        fact(&workspace, "jimmy/vieja", 30, "vieja");
        fact(&workspace, "jimmy/media", 5, "media");
        fact(&workspace, "jimmy/nueva", 1, "nueva");
        let out = render(&workspace, "jimmy");
        assert!(out.contains("quién es"));
        assert!(out.contains("nueva"));
        assert!(out.contains("media"));
        assert!(!out.contains("## jimmy/vieja"));
        assert!(out.contains("afuera del prompt: jimmy/vieja"), "{out}");
    }

    #[test]
    fn render_without_a_project_is_only_the_facts() {
        let workspace = workspace("render-general");
        fact(&workspace, "usuario", 10, "quién es");
        fact(&workspace, "jimmy/nueva", 1, "nueva");
        let out = render(&workspace, "");
        assert!(out.contains("quién es"));
        assert!(!out.contains("nueva"));
    }

    #[test]
    fn render_falls_back_to_memory_md_until_the_facts_exist() {
        let workspace = workspace("render-legacy");
        write_level1(&workspace, &entry("usuario", 1, "quién es"));
        assert!(render(&workspace, "jimmy").contains("quién es"));
        std::fs::create_dir_all(facts_dir(&workspace)).unwrap();
        assert!(render(&workspace, "jimmy").is_empty());
    }

    #[test]
    fn add_writes_a_fact_and_updates_it_in_place() {
        let workspace = workspace("add");
        std::fs::create_dir_all(workspace.join("projects/jimmy")).unwrap();
        add(&workspace, "jimmy/telemetria", "medicion", "un número").unwrap();
        assert!(show(&workspace, "jimmy/telemetria")
            .unwrap()
            .contains("un número"));
        add(&workspace, "jimmy/telemetria", "decision", "otro número").unwrap();
        let text = std::fs::read_to_string(project_path(&workspace, "jimmy")).unwrap();
        assert_eq!(text.matches("## jimmy/telemetria").count(), 1);
        assert!(text.contains("decision"));
        assert!(text.contains("otro número"));
    }

    #[test]
    fn add_rejects_an_unknown_project_and_an_unknown_kind() {
        let workspace = workspace("add-unknown");
        std::fs::create_dir_all(workspace.join("projects/picsel")).unwrap();
        let error = add(&workspace, "picssel/algo", "estado", "x").unwrap_err();
        assert!(error.contains("¿picsel?"), "{error}");
        let error = add(&workspace, "jimmy/algo", "inventado", "x").unwrap_err();
        assert!(error.contains("tipo desconocido"), "{error}");
    }

    #[test]
    fn migrate_splits_memory_md_into_facts_and_projects() {
        let workspace = workspace("migrate");
        let text = format!(
            "{}{}",
            entry("usuario", 1, "quién es"),
            entry("jimmy/estado", 2, "cómo está")
        );
        write_level1(&workspace, &text);
        let report = migrate(&workspace).unwrap();
        assert!(report.contains("2 hechos"), "{report}");
        assert!(facts_dir(&workspace).join("usuario.md").exists());
        assert!(project_path(&workspace, "jimmy").exists());
        assert!(render(&workspace, "jimmy").contains("quién es"));
    }

    #[test]
    fn list_names_the_files_and_their_keys() {
        let workspace = workspace("list");
        fact(&workspace, "usuario", 1, "quién es");
        fact(&workspace, "jimmy/estado", 1, "cómo está");
        let out = list(&workspace);
        assert!(out.contains("notes/memory/usuario.md"), "{out}");
        assert!(out.contains("notes/projects/jimmy.md"), "{out}");
        assert!(out.contains("jimmy/estado"), "{out}");
    }

    #[test]
    fn show_finds_a_fact_and_then_the_dropped_one() {
        let workspace = workspace("show");
        fact(&workspace, "usuario", 1, "vive acá");
        assert!(show(&workspace, "usuario").unwrap().contains("vive acá"));
        sync(&workspace).unwrap();
        std::fs::remove_file(facts_dir(&workspace).join("usuario.md")).unwrap();
        let out = show(&workspace, "usuario").unwrap();
        assert!(out.contains("nivel 2"), "{out}");
        assert!(show(&workspace, "no-existe").is_err());
    }

    #[test]
    fn concurrent_events_keep_one_json_per_line() {
        let workspace = workspace("events");
        let threads: Vec<_> = (0..8)
            .map(|n| {
                let workspace = workspace.clone();
                std::thread::spawn(move || {
                    for i in 0..50 {
                        emit(
                            &workspace,
                            json!({"op": "render", "n": n, "i": i, "text": "x".repeat(200)}),
                        );
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let text = std::fs::read_to_string(events_path(&workspace)).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 400);
        for line in lines {
            serde_json::from_str::<serde_json::Value>(line).unwrap();
        }
    }
}
