#[derive(Default, Clone, PartialEq)]
pub struct Described {
    pub dir: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    Path(String),
    Add(String),
    Del(String),
}

pub fn parse_args(raw: &str) -> Option<serde_json::Value> {
    serde_json::from_str(raw).ok()
}

fn skip_redirect(text: &str) -> &str {
    let text = text.trim_start();
    let Some(rest) = text.strip_prefix("2>") else {
        return text;
    };
    let rest = rest.trim_start();
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    &rest[end..]
}

pub fn split_dirs(command: &str) -> (String, String) {
    let mut rest = command.trim().to_string();
    let mut dir = String::new();
    while let Some((candidate, tail)) = cd_prefix(&rest) {
        dir = candidate;
        rest = tail;
    }
    (dir, rest)
}

fn cd_prefix(text: &str) -> Option<(String, String)> {
    let after = text.strip_prefix("cd ")?.trim_start();
    let end = after.find(char::is_whitespace).unwrap_or(after.len());
    if end == 0 {
        return None;
    }
    let candidate = after[..end].to_string();
    let tail = skip_redirect(&after[end..]).trim_start();
    let next = tail.strip_prefix("&&")?.trim_start().to_string();
    Some((candidate, next))
}

pub fn shorten(path: &str, workspace: &str) -> String {
    if workspace.is_empty() {
        return path.to_string();
    }
    if path == workspace {
        return "~".to_string();
    }
    match path
        .strip_prefix(workspace)
        .and_then(|rest| rest.strip_prefix('/'))
    {
        Some(rest) => rest.to_string(),
        None => path.to_string(),
    }
}

pub fn describe(name: &str, raw: &str, workspace: &str) -> Described {
    let Some(args) = parse_args(raw) else {
        return Described {
            dir: String::new(),
            text: raw.to_string(),
        };
    };
    let field = |key: &str| {
        args.get(key)
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string()
    };
    match name {
        "bash" => {
            let (dir, rest) = split_dirs(&field("command"));
            Described {
                dir: shorten(&dir, workspace),
                text: rest,
            }
        }
        "read" | "write" | "edit" => Described {
            dir: String::new(),
            text: shorten(&field("path"), workspace),
        },
        "search" => Described {
            dir: String::new(),
            text: field("query"),
        },
        "fetch" => Described {
            dir: String::new(),
            text: field("url"),
        },
        "browse" => {
            let steps = args
                .get("steps")
                .and_then(|value| value.as_array())
                .map(Vec::len)
                .unwrap_or(0);
            let url = field("url");
            Described {
                dir: String::new(),
                text: if url.is_empty() {
                    format!("{steps} Pasos")
                } else {
                    url
                },
            }
        }
        _ => Described {
            dir: String::new(),
            text: raw.to_string(),
        },
    }
}

fn split_lines(text: &str) -> Vec<String> {
    let text = text.strip_suffix('\n').unwrap_or(text);
    if text.is_empty() {
        return Vec::new();
    }
    text.split('\n').map(str::to_string).collect()
}

pub fn diff_lines(raw: &str) -> (Vec<Line>, String) {
    let Some(args) = parse_args(raw) else {
        return (Vec::new(), String::new());
    };
    let mut lines = Vec::new();
    if let Some(path) = args.get("path").and_then(|value| value.as_str()) {
        lines.push(Line::Path(path.to_string()));
    }
    let mut added = 0;
    let mut removed = 0;
    for edit in args
        .get("edits")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
    {
        for line in split_lines(
            edit.get("oldText")
                .and_then(|value| value.as_str())
                .unwrap_or_default(),
        ) {
            lines.push(Line::Del(format!("- {line}")));
            removed += 1;
        }
        for line in split_lines(
            edit.get("newText")
                .and_then(|value| value.as_str())
                .unwrap_or_default(),
        ) {
            lines.push(Line::Add(format!("+ {line}")));
            added += 1;
        }
    }
    (lines, format!("+{added} -{removed}"))
}

pub fn content_lines(raw: &str) -> (Vec<Line>, String) {
    let Some(args) = parse_args(raw) else {
        return (Vec::new(), String::new());
    };
    let mut lines = Vec::new();
    if let Some(path) = args.get("path").and_then(|value| value.as_str()) {
        lines.push(Line::Path(path.to_string()));
    }
    let content = split_lines(
        args.get("content")
            .and_then(|value| value.as_str())
            .unwrap_or_default(),
    );
    let count = content.len();
    for line in content {
        lines.push(Line::Add(format!("+ {line}")));
    }
    (lines, format!("+{count}"))
}

pub fn noun(name: &str) -> (&'static str, &'static str) {
    match name {
        "read" => ("Archivo", "Archivos"),
        "write" => ("Archivo", "Archivos"),
        "edit" => ("Edición", "Ediciones"),
        "bash" => ("Comando", "Comandos"),
        "search" => ("Búsqueda", "Búsquedas"),
        "fetch" | "browse" => ("Página", "Páginas"),
        _ => ("Paso", "Pasos"),
    }
}

pub fn steps_label(names: &[String]) -> String {
    let mut counts: Vec<(&str, &str, usize)> = Vec::new();
    for name in names {
        let (one, many) = noun(name);
        match counts.iter_mut().find(|(label, _, _)| *label == one) {
            Some(entry) => entry.2 += 1,
            None => counts.push((one, many, 1)),
        }
    }
    let parts: Vec<String> = counts
        .iter()
        .map(|(one, many, count)| {
            let noun = if *count == 1 { *one } else { *many };
            format!("{count} {noun}")
        })
        .collect();
    match parts.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} y {last}", rest.join(", ")),
    }
}

pub fn elapsed(ms: u64) -> String {
    if ms < 1000 {
        return format!("{ms} ms");
    }
    let seconds = ms as f64 / 1000.0;
    if seconds < 60.0 {
        return format!("{seconds:.1} s").replace('.', ",");
    }
    format!("{}m {}s", ms / 60000, (ms % 60000) / 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cd_dirs_are_split_and_shortened() {
        let (dir, rest) = split_dirs("cd /w/ken && cargo test");
        assert_eq!(dir, "/w/ken");
        assert_eq!(rest, "cargo test");
        let (dir, rest) = split_dirs("cd /w/ken 2> /dev/null && cd src && ls");
        assert_eq!(dir, "src");
        assert_eq!(rest, "ls");
        assert_eq!(split_dirs("ls -la"), (String::new(), "ls -la".into()));
    }

    #[test]
    fn the_workspace_shows_as_tilde() {
        assert_eq!(shorten("/w/ken", "/w/ken"), "~");
        assert_eq!(shorten("/w/ken/src/main.rs", "/w/ken"), "src/main.rs");
        assert_eq!(shorten("/otro", "/w/ken"), "/otro");
    }

    #[test]
    fn steps_are_counted_by_noun() {
        let names = vec!["bash".to_string(), "bash".to_string(), "edit".to_string()];
        assert_eq!(steps_label(&names), "2 Comandos y 1 Edición");
        assert_eq!(steps_label(&["bash".to_string()]), "1 Comando");
    }

    #[test]
    fn a_single_edit_says_how_many_lines_moved() {
        let raw =
            r#"{"path":"a.txt","edits":[{"oldText":"one\ntwo\n","newText":"one\nthree\nfour\n"}]}"#;
        let (lines, stat) = diff_lines(raw);
        assert_eq!(stat, "+3 -2");
        assert_eq!(lines.first(), Some(&Line::Path("a.txt".into())));
    }
}
