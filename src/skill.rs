use std::path::Path;

pub fn list(dir: &Path) -> String {
    let mut skills = scan(dir);
    skills.sort_by(|a, b| a.0.cmp(&b.0));
    if skills.is_empty() {
        return format!("No hay skills instaladas en {}.", dir.display());
    }
    skills
        .into_iter()
        .map(|(name, description)| format!("{name} — {description}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn load(dir: &Path, name: &str) -> Result<String, String> {
    if name.is_empty() || name.contains(['/', '\\']) || name.contains("..") {
        return Err(format!("nombre de skill inválido: {name}"));
    }
    let skill_dir = dir.join(name);
    let text = std::fs::read_to_string(skill_dir.join("SKILL.md"))
        .map_err(|_| format!("no existe la skill `{name}` en {}", dir.display()))?;
    Ok(format!(
        "Skill {name} — {}\n\n{}",
        skill_dir.display(),
        body(&text)
    ))
}

fn scan(dir: &Path) -> Vec<(String, String)> {
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
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return Vec::new();
    }
    let mut fields = Vec::new();
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            fields.push((key.trim().to_string(), unquote(value.trim())));
        }
    }
    fields
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
        assert_eq!(list(&dir), "charts — Gráficos");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_strips_the_frontmatter() {
        let dir = setup("load");
        let out = load(&dir, "charts").unwrap();
        assert!(out.contains("# Charts"));
        assert!(!out.contains("description:"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_rejects_a_bad_name() {
        let dir = setup("bad");
        assert!(load(&dir, "../secrets").is_err());
        assert!(load(&dir, "nope").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_directory_lists_nothing() {
        assert!(list(Path::new("/nope/nope/skills")).contains("No hay skills"));
    }
}
