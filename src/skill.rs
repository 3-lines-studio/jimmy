use std::iter::Peekable;
use std::path::{Path, PathBuf};
use std::str::Lines;

pub const BUILTIN: &str = "/usr/local/share/jimmy/skills";

pub fn dirs(local: PathBuf) -> Vec<PathBuf> {
    vec![local, PathBuf::from(BUILTIN)]
}

pub fn list(dirs: &[PathBuf]) -> String {
    let skills = scan(dirs);
    if skills.is_empty() {
        return format!("No hay skills instaladas en {}.", paths(dirs));
    }
    lines(skills)
}

pub fn index(dirs: &[PathBuf]) -> String {
    let skills = scan(dirs);
    if skills.is_empty() {
        return "No hay ninguna instalada.".to_string();
    }
    lines(skills)
}

pub fn load(dirs: &[PathBuf], name: &str) -> Result<String, String> {
    if name.is_empty() || name.contains(['/', '\\']) || name.contains("..") {
        return Err(format!("nombre de skill inválido: {name}"));
    }
    for dir in dirs {
        let skill_dir = dir.join(name);
        let Ok(text) = std::fs::read_to_string(skill_dir.join("SKILL.md")) else {
            continue;
        };
        return Ok(format!(
            "Skill {name} — {}\n\n{}",
            skill_dir.display(),
            body(&text)
        ));
    }
    Err(format!("no existe la skill `{name}` en {}", paths(dirs)))
}

fn lines(mut skills: Vec<(String, String)>) -> String {
    skills.sort_by(|a, b| a.0.cmp(&b.0));
    skills
        .into_iter()
        .map(|(name, description)| format!("{name} — {description}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn paths(dirs: &[PathBuf]) -> String {
    dirs.iter()
        .map(|dir| dir.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn scan(dirs: &[PathBuf]) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = Vec::new();
    for dir in dirs {
        for skill in scan_dir(dir) {
            if found.iter().any(|(name, _)| name == &skill.0) {
                continue;
            }
            found.push(skill);
        }
    }
    found
}

fn scan_dir(dir: &Path) -> Vec<(String, String)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            if !entry.path().is_dir() {
                return None;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let text = std::fs::read_to_string(entry.path().join("SKILL.md")).ok()?;
            let description = frontmatter(&text)
                .into_iter()
                .find(|(key, _)| key == "description")
                .map(|(_, value)| value)
                .unwrap_or_default();
            Some((name, description))
        })
        .collect()
}

fn frontmatter(text: &str) -> Vec<(String, String)> {
    let mut lines = text.lines().peekable();
    if lines.next().map(str::trim) != Some("---") {
        return Vec::new();
    }
    let mut fields = Vec::new();
    while let Some(line) = lines.next() {
        if line.trim() == "---" {
            break;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if value.starts_with('>') || value.starts_with('|') {
            fields.push((key.trim().to_string(), block(&mut lines)));
            continue;
        }
        fields.push((key.trim().to_string(), unquote(value)));
    }
    fields
}

fn block(lines: &mut Peekable<Lines>) -> String {
    let mut parts = Vec::new();
    while let Some(next) = lines.peek().copied() {
        if next.trim().is_empty() {
            lines.next();
            continue;
        }
        if !next.starts_with(' ') && !next.starts_with('\t') {
            break;
        }
        parts.push(next.trim().to_string());
        lines.next();
    }
    parts.join(" ")
}

fn unquote(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

fn body(text: &str) -> String {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return text.trim().to_string();
    }
    let mut rest = lines.skip_while(|line| line.trim() != "---");
    rest.next();
    rest.collect::<Vec<_>>().join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("jimmy-skills-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("charts")).unwrap();
        std::fs::write(
            dir.join("charts/SKILL.md"),
            "---\nname: charts\ndescription: \"Gráficos\"\n---\n\n# Charts\n\nUsá render_chart.py.\n",
        )
        .unwrap();
        std::fs::write(dir.join("README.txt"), "no es skill").unwrap();
        std::fs::create_dir_all(dir.join("empty")).unwrap();
        dir
    }

    #[test]
    fn list_shows_only_skills_with_a_body() {
        let dir = setup("list");
        assert_eq!(list(&[dir.clone()]), "charts — Gráficos");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_strips_the_frontmatter() {
        let dir = setup("load");
        let out = load(&[dir.clone()], "charts").unwrap();
        assert!(out.contains("# Charts"));
        assert!(!out.contains("description:"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_rejects_a_bad_name() {
        let dir = setup("bad");
        assert!(load(&[dir.clone()], "../secrets").is_err());
        assert!(load(&[dir.clone()], "nope").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_folded_description_joins_its_lines() {
        let dir = setup("folded");
        std::fs::write(
            dir.join("empty/SKILL.md"),
            "---\nname: multi\ndescription: >\n  Una cosa\n  y la otra.\n---\n\n# Multi\n",
        )
        .unwrap();
        assert!(list(&[dir.clone()]).contains("empty — Una cosa y la otra."));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_directory_lists_nothing() {
        assert!(list(&[PathBuf::from("/nope/nope/skills")]).contains("No hay skills"));
    }

    #[test]
    fn a_local_skill_shadows_the_builtin_one() {
        let over = setup("over");
        let under = setup("under");
        std::fs::write(
            over.join("charts/SKILL.md"),
            "---\nname: charts\ndescription: \"Mía\"\n---\n\n# Mía\n",
        )
        .unwrap();
        std::fs::write(
            under.join("charts/SKILL.md"),
            "---\nname: charts\ndescription: \"De fábrica\"\n---\n\n# De fábrica\n",
        )
        .unwrap();
        std::fs::create_dir_all(under.join("otra")).unwrap();
        std::fs::write(
            under.join("otra/SKILL.md"),
            "---\nname: otra\ndescription: \"Sólo del builtin\"\n---\n\n# Otra\n",
        )
        .unwrap();
        let dirs = vec![over.clone(), under.clone()];
        assert_eq!(index(&dirs), "charts — Mía\notra — Sólo del builtin");
        let loaded = load(&dirs, "otra").unwrap();
        assert!(loaded.starts_with(&format!("Skill otra — {}", under.join("otra").display())));
        std::fs::remove_dir_all(&over).unwrap();
        std::fs::remove_dir_all(&under).unwrap();
    }

    #[test]
    fn the_index_says_so_when_there_is_nothing() {
        assert_eq!(
            index(&[PathBuf::from("/nope/nope/skills")]),
            "No hay ninguna instalada."
        );
    }
}
