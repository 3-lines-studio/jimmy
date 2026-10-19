pub fn every(value: &str) -> String {
    let text = value.trim();
    let mut characters = text.chars();
    let Some(unit) = characters.next_back() else {
        return format!("cada {text}");
    };
    let amount = characters.as_str().trim();
    let name = match unit {
        's' => "s",
        'm' => "min",
        'h' => "h",
        'd' => "d",
        other => return format!("cada {amount} {other}"),
    };
    format!("cada {amount} {name}")
}

pub fn when(when: Option<&str>, at: Option<&str>, every_value: Option<&str>) -> String {
    if let Some(when) = when.filter(|value| !value.is_empty()) {
        let normalized = when.replace(' ', "T");
        let (date, time) = normalized.split_once('T').unwrap_or((&normalized, ""));
        let mut parts = date.split('-').rev();
        let day = parts.next().unwrap_or_default();
        let month = parts.next().unwrap_or_default();
        return format!("una vez · {day}/{month} a las {time}");
    }
    if let Some(at) = at.filter(|value| !value.is_empty()) {
        return format!("todos los días · {at}");
    }
    if let Some(value) = every_value.filter(|value| !value.is_empty()) {
        return every(value);
    }
    "sin horario".to_string()
}

pub fn duration(ms: u64) -> String {
    if ms < 1000 {
        return format!("{ms} ms");
    }
    let seconds = (ms as f64 / 1000.0).round() as u64;
    if seconds < 60 {
        return format!("{seconds} s");
    }
    let minutes = (seconds as f64 / 60.0).round() as u64;
    if minutes < 60 {
        return format!("{minutes} min");
    }
    format!("{} h", (minutes as f64 / 60.0).round() as u64)
}

pub fn day(day: &str, today: &str, yesterday: &str) -> String {
    if day == today {
        return "hoy".to_string();
    }
    if day == yesterday {
        return "ayer".to_string();
    }
    let parts: Vec<&str> = day.split('-').collect();
    match (parts.len(), parts.get(1), parts.get(2)) {
        (3, Some(month), Some(date)) => format!("{date}/{month}"),
        _ => day.to_string(),
    }
}

pub fn moment(day: &str, clock: &str, today: &str, yesterday: &str) -> String {
    format!("{} {clock}", self::day(day, today, yesterday))
}

pub fn ago(seconds: i64) -> String {
    let seconds = seconds.max(0);
    if seconds < 60 {
        return format!("hace {seconds} s");
    }
    let minutes = (seconds as f64 / 60.0).round() as i64;
    if minutes < 60 {
        return format!("hace {minutes} min");
    }
    let hours = (minutes as f64 / 60.0).round() as i64;
    if hours < 24 {
        return format!("hace {hours} h");
    }
    format!("hace {} d", (hours as f64 / 24.0).round() as i64)
}

#[derive(Clone, Copy, PartialEq)]
pub struct Mark {
    pub paused: bool,
    pub blocked: bool,
    pub unread: u64,
    pub last_ok: Option<bool>,
    pub running: bool,
}

pub fn state(mark: Mark) -> &'static str {
    if mark.paused {
        return "";
    }
    if mark.blocked {
        return "error";
    }
    if mark.running {
        return "running";
    }
    match mark.last_ok {
        None => "",
        Some(true) => "done",
        Some(false) => "error",
    }
}

pub fn unread(marks: &[Mark]) -> u64 {
    marks.iter().map(|mark| mark.unread).sum()
}

pub fn failed(marks: &[Mark]) -> usize {
    marks
        .iter()
        .filter(|mark| mark.unread > 0 && mark.last_ok == Some(false))
        .count()
}

pub fn badge(marks: &[Mark]) -> &'static str {
    match failed(marks) {
        0 => "unread",
        _ => "error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_schedule_reads_at_a_glance() {
        assert_eq!(when(None, Some("05:00"), None), "todos los días · 05:00");
        assert_eq!(when(None, None, Some("30m")), "cada 30 min");
        assert_eq!(when(None, None, Some("6h")), "cada 6 h");
        assert_eq!(when(None, None, Some("2d")), "cada 2 d");
        assert_eq!(
            when(Some("2026-09-14T15:00"), None, None),
            "una vez · 14/09 a las 15:00"
        );
        assert_eq!(when(None, None, None), "sin horario");
    }

    #[test]
    fn a_long_duration_is_not_shown_in_seconds() {
        assert_eq!(duration(320), "320 ms");
        assert_eq!(duration(38_000), "38 s");
        assert_eq!(duration(125_000), "2 min");
        assert_eq!(duration(7_300_000), "2 h");
    }

    #[test]
    fn the_run_says_which_day_it_was() {
        assert_eq!(moment("2026-09-26", "05:00", "2026-09-26", "2026-09-25"), "hoy 05:00");
        assert_eq!(moment("2026-09-25", "22:00", "2026-09-26", "2026-09-25"), "ayer 22:00");
        assert_eq!(moment("2026-09-14", "05:00", "2026-09-26", "2026-09-25"), "14/09 05:00");
    }

    #[test]
    fn how_long_ago_it_ran() {
        assert_eq!(ago(30), "hace 30 s");
        assert_eq!(ago(720), "hace 12 min");
        assert_eq!(ago(61_200), "hace 17 h");
        assert_eq!(ago(259_200), "hace 3 d");
        assert_eq!(ago(-5), "hace 0 s");
    }

    #[test]
    fn the_counter_adds_what_was_not_read() {
        let marks = [
            Mark { paused: false, blocked: false, unread: 3, last_ok: Some(true), running: false },
            Mark { paused: false, blocked: false, unread: 0, last_ok: Some(false), running: false },
            Mark { paused: false, blocked: false, unread: 2, last_ok: Some(false), running: false },
            Mark { paused: true, blocked: false, unread: 1, last_ok: Some(false), running: false },
        ];
        assert_eq!(unread(&marks), 6);
        assert_eq!(unread(&[]), 0);
        assert_eq!(failed(&marks), 2);
        assert_eq!(badge(&marks), "error");
        let one = [Mark { paused: false, blocked: false, unread: 1, last_ok: Some(true), running: false }];
        assert_eq!(badge(&one), "unread");
    }

    #[test]
    fn a_blocked_task_looks_like_an_error() {
        let blocked = Mark { paused: false, blocked: true, unread: 0, last_ok: None, running: false };
        assert_eq!(state(blocked), "error");
        let paused = Mark { paused: true, ..blocked };
        assert_eq!(state(paused), "");
        let done = Mark { paused: false, blocked: false, unread: 0, last_ok: Some(true), running: false };
        assert_eq!(state(done), "done");
    }
}
