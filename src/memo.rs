use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const BUDGET: usize = 16 * 1024;
const STALE_DAYS: i64 = 30;
const STALE_SHIFT: u32 = 6;
const KEEP: [&str; 4] = ["usuario", "proyectos", "entorno", "decisiones-vigentes"];

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
    start: usize,
    end: usize,
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

fn events_path(workspace: &Path) -> PathBuf {
    workspace.join("state/memory-events.jsonl")
}

pub fn render(workspace: &Path) -> String {
    let Ok(text) = std::fs::read_to_string(level1_path(workspace)) else {
        return String::new();
    };
    if text.len() <= BUDGET {
        return text.trim().to_string();
    }

    let level1 = parse(&text);
    let mut keep = vec![false; level1.entries.len()];
    let mut used = 0;
    for (index, entry) in level1.entries.iter().enumerate() {
        if KEEP.contains(&entry.key.as_str()) {
            keep[index] = true;
            used += entry.end - entry.start;
        }
    }
    for (index, entry) in level1.entries.iter().enumerate().rev() {
        if keep[index] {
            continue;
        }
        let bytes = entry.end - entry.start;
        if used + bytes > BUDGET {
            continue;
        }
        keep[index] = true;
        used += bytes;
    }

    emit(
        workspace,
        json!({
            "kind": "memory",
            "op": "render",
            "ts": now(),
            "bytes": text.len(),
            "budget": BUDGET,
            "entries": level1.entries.len(),
        }),
    );
    format!(
        "[cortado: el nivel 1 tiene {} bytes y el tope es {BUDGET}]\n{}",
        text.len(),
        prune(&text, &level1.entries, &keep).trim()
    )
}

pub fn sync(workspace: &Path) -> Result<String, String> {
    let started = std::time::Instant::now();
    std::fs::create_dir_all(workspace.join("notes")).map_err(|e| e.to_string())?;
    let Ok(text) = std::fs::read_to_string(level1_path(workspace)) else {
        return Ok("sync: no hay notes/memory.md".into());
    };
    let level1 = parse(&text);
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
    let bytes = text.len();
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
            "malformed": level1.malformed,
            "corrupt": corrupt,
        }),
    );

    let mut report = format!(
        "sync: {entries} entradas, {bytes} bytes · {created} nuevas · {updated} actualizadas · {reasserted} reafirmadas · {promoted} vueltas"
    );
    if dropped > 0 {
        report.push_str(&format!(" · {dropped} borradas a mano"));
    }
    if level1.malformed > 0 {
        report.push_str(&format!(" · {} con encabezado inválido", level1.malformed));
    }
    if corrupt > 0 {
        report.push_str(&format!(" · {corrupt} líneas corruptas en el nivel 2"));
    }
    Ok(report)
}

pub fn demote(workspace: &Path) -> Result<String, String> {
    let started = std::time::Instant::now();
    let path = level1_path(workspace);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("memory.md: {e}"))?;
    let level1 = parse(&text);
    let (records, _) = load_records(&level2_path(workspace));
    let latest = latest(&records);
    let today = today();

    struct Candidate {
        index: usize,
        expired: bool,
        last_seen: i64,
        rev: u32,
    }

    let mut candidates = Vec::new();
    let mut stale = Vec::new();
    for (index, entry) in level1.entries.iter().enumerate() {
        if KEEP.contains(&entry.key.as_str()) {
            continue;
        }
        let (rev, last_seen) = match latest.get(&entry.key) {
            Some(record) => (record.rev, record.last_seen),
            None => (1, entry.date),
        };
        if today > review_after(last_seen, rev) {
            stale.push(format!(
                "{} ({})",
                entry.key,
                format_date(review_after(last_seen, rev))
            ));
        }
        candidates.push(Candidate {
            index,
            expired: today > review_after(last_seen, rev),
            last_seen,
            rev,
        });
    }
    candidates.sort_by(|a, b| {
        b.expired
            .cmp(&a.expired)
            .then(a.last_seen.cmp(&b.last_seen))
            .then(a.rev.cmp(&b.rev))
    });

    let bytes_before = text.len();
    let mut removed = Vec::new();
    let mut freed = 0;
    for candidate in &candidates {
        if bytes_before - freed <= BUDGET {
            break;
        }
        let entry = &level1.entries[candidate.index];
        freed += entry.end - entry.start;
        removed.push(candidate.index);
    }

    let mut moved = Vec::new();
    if !removed.is_empty() {
        let mut down = Vec::new();
        for index in &removed {
            let entry = &level1.entries[*index];
            let rev = latest.get(&entry.key).map(|r| r.rev).unwrap_or(1);
            down.push(Record {
                ts: now(),
                key: entry.key.clone(),
                kind: entry.kind.clone(),
                body: entry.body.clone(),
                rev,
                last_seen: entry.date,
                left: Some("demoted".into()),
            });
            moved.push(entry.key.clone());
        }
        append_records(&level2_path(workspace), &down)?;
        let mut keep = vec![true; level1.entries.len()];
        for index in &removed {
            keep[*index] = false;
        }
        axe::atomic_write(&path, prune(&text, &level1.entries, &keep).as_bytes())
            .map_err(|e| format!("memory.md: {e}"))?;
    }

    let bytes_after = bytes_before - freed;
    emit(
        workspace,
        json!({
            "kind": "memory",
            "op": "demote",
            "ts": now(),
            "ms": started.elapsed().as_millis() as u64,
            "bytes_before": bytes_before,
            "bytes_after": bytes_after,
            "budget": BUDGET,
            "moved": moved,
            "stale": stale,
        }),
    );

    let mut report = format!(
        "demote: {bytes_before} → {bytes_after} bytes (tope {BUDGET}) · {} bajaron",
        moved.len()
    );
    if !moved.is_empty() {
        report.push_str(&format!(": {}", moved.join(", ")));
    }
    if bytes_after > BUDGET {
        report.push_str(" · ojo: sigue por encima del tope");
    }
    if !stale.is_empty() {
        report.push_str(&format!(" · vencidas: {}", stale.join(", ")));
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

fn review_after(last_seen: i64, rev: u32) -> i64 {
    let shift = rev.saturating_sub(1).min(STALE_SHIFT);
    last_seen + STALE_DAYS * (1i64 << shift)
}

fn parse(text: &str) -> Level1 {
    let chunks: Vec<&str> = text.split_inclusive('\n').collect();
    let mut offsets = Vec::with_capacity(chunks.len());
    let mut position = 0;
    for chunk in &chunks {
        offsets.push(position);
        position += chunk.len();
    }

    let mut malformed = 0;
    let mut starts = Vec::new();
    for (index, chunk) in chunks.iter().enumerate() {
        let Some(rest) = chunk.strip_prefix("## ") else {
            continue;
        };
        match heading(rest.trim_end()) {
            Some((key, kind, date)) => starts.push((index, key, kind, date)),
            None => malformed += 1,
        }
    }

    let mut entries = Vec::new();
    for (position, (index, key, kind, date)) in starts.iter().enumerate() {
        let end_chunk = starts
            .get(position + 1)
            .map(|next| next.0)
            .unwrap_or(chunks.len());
        let end = offsets.get(end_chunk).copied().unwrap_or(text.len());
        let body = chunks[*index + 1..end_chunk].concat().trim().to_string();
        entries.push(Entry {
            key: key.clone(),
            kind: kind.clone(),
            date: *date,
            body,
            start: offsets[*index],
            end,
        });
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

fn prune(text: &str, entries: &[Entry], keep: &[bool]) -> String {
    let Some(first) = entries.first() else {
        return text.to_string();
    };
    let mut out = String::new();
    out.push_str(&text[..first.start]);
    let mut cursor = first.start;
    for (index, entry) in entries.iter().enumerate() {
        if keep[index] {
            continue;
        }
        out.push_str(&text[cursor..entry.start]);
        cursor = entry.end;
    }
    out.push_str(&text[cursor..]);
    out
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
    let _ = writeln!(file, "{event}");
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

    fn read_level1(workspace: &Path) -> String {
        std::fs::read_to_string(level1_path(workspace)).unwrap()
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
        assert!(report.contains("1 nuevas"));
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
    fn sync_detects_return_after_demote() {
        let workspace = workspace("sync-promote");
        let filler = "f".repeat(BUDGET + 1_000);
        let kept = date(1);
        let text = format!(
            "## usuario · identidad · {kept}\n{filler}\n\n{}",
            entry("a", 40, "cuerpo a")
        );
        write_level1(&workspace, &text);
        sync(&workspace).unwrap();
        let report = demote(&workspace).unwrap();
        assert!(report.contains("1 bajaron: a"), "{report}");
        assert!(!read_level1(&workspace).contains("cuerpo a"));
        write_level1(&workspace, &text);
        let report = sync(&workspace).unwrap();
        assert!(report.contains("1 vueltas"), "{report}");
        let records = stored(&workspace);
        let last = records.last().unwrap();
        assert_eq!(last.key, "a");
        assert_eq!(last.rev, 2);
        assert!(last.left.is_none());
    }

    #[test]
    fn demote_never_moves_fixed_keys() {
        let workspace = workspace("demote-keep");
        let body = "x".repeat(BUDGET);
        let day = date(400);
        write_level1(
            &workspace,
            &format!("## usuario · identidad · {day}\n{body}\n"),
        );
        sync(&workspace).unwrap();
        let report = demote(&workspace).unwrap();
        assert!(report.contains("0 bajaron"));
        assert!(report.contains("sigue por encima del tope"));
        assert!(read_level1(&workspace).contains("usuario"));
    }

    #[test]
    fn demote_is_idempotent_when_under_budget() {
        let workspace = workspace("demote-idle");
        write_level1(&workspace, &entry("a", 1, "cuerpo a"));
        sync(&workspace).unwrap();
        let before = stored(&workspace).len();
        let report = demote(&workspace).unwrap();
        assert!(report.contains("0 bajaron"));
        assert_eq!(stored(&workspace).len(), before);
    }

    #[test]
    fn demote_frees_until_under_budget_oldest_first() {
        let workspace = workspace("demote-budget");
        let body = "y".repeat(6_000);
        let text = format!(
            "{}{}{}",
            entry("nueva", 1, &body),
            entry("vieja", 300, &body),
            entry("antigua", 400, &body)
        );
        write_level1(&workspace, &text);
        sync(&workspace).unwrap();
        let report = demote(&workspace).unwrap();
        assert!(report.contains("1 bajaron"), "{report}");
        assert!(report.contains("antigua"));
        let after = read_level1(&workspace);
        assert!(after.len() <= BUDGET);
        assert!(after.contains("nueva"));
        assert!(!after.contains("antigua"));
        let moved: Vec<String> = stored(&workspace)
            .iter()
            .filter(|record| record.left.as_deref() == Some("demoted"))
            .map(|record| record.key.clone())
            .collect();
        assert_eq!(moved, vec!["antigua"]);
    }

    #[test]
    fn render_returns_file_as_is_when_it_fits() {
        let workspace = workspace("render-fit");
        write_level1(&workspace, &entry("a", 1, "cuerpo a"));
        assert_eq!(render(&workspace), read_level1(&workspace).trim());
    }

    #[test]
    fn render_keeps_fixed_keys_and_never_cuts_an_entry() {
        let workspace = workspace("render-cut");
        let body = "z".repeat(6_000);
        let first = date(1);
        let text = format!(
            "## usuario · identidad · {first}\n{body}\n\n{}{}",
            entry("media", 2, &body),
            entry("nueva", 1, &body)
        );
        write_level1(&workspace, &text);
        let out = render(&workspace);
        assert!(out.contains("usuario"));
        assert!(out.contains("nueva"));
        assert!(!out.contains("media"));
        assert!(out.len() <= BUDGET + 120);
    }
}
