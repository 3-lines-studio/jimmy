use std::path::{Path, PathBuf};

pub const DEFAULT: &str = "identidad,estilo,codigo,jimmy,herramientas,workspace,memoria,agenda,git";
const BUILTIN: &str = "/usr/local/share/jimmy/prompts";

pub fn dirs(root: &Path) -> Vec<PathBuf> {
    vec![root.join("prompts"), PathBuf::from(BUILTIN)]
}

pub fn assemble(spec: &str, dirs: &[PathBuf]) -> Result<String, String> {
    let mut parts = Vec::new();
    for name in spec.split(',').map(str::trim).filter(|n| !n.is_empty()) {
        parts.push(fragment(name, dirs)?);
    }
    Ok(parts.join("\n\n"))
}

fn fragment(name: &str, dirs: &[PathBuf]) -> Result<String, String> {
    for dir in dirs {
        let Ok(text) = std::fs::read_to_string(dir.join(format!("{name}.md"))) else {
            continue;
        };
        let text = text.trim();
        if !text.is_empty() {
            return Ok(text.to_string());
        }
    }
    Err(format!("no encontré el fragmento `{name}.md`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("jimmy-prompt-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn write(dir: &Path, name: &str, text: &str) {
        std::fs::write(dir.join(format!("{name}.md")), text).unwrap();
    }

    #[test]
    fn joins_fragments_in_spec_order() {
        let dir = dir("order");
        write(&dir, "uno", "primero\n");
        write(&dir, "dos", "segundo\n");
        assert_eq!(assemble("uno, dos", &[dir]).unwrap(), "primero\n\nsegundo");
    }

    #[test]
    fn earlier_dir_wins() {
        let over = dir("over");
        let under = dir("under");
        write(&over, "identidad", "mía");
        write(&under, "identidad", "de fábrica");
        assert_eq!(assemble("identidad", &[over, under]).unwrap(), "mía");
    }

    #[test]
    fn empty_spec_assembles_nothing() {
        assert_eq!(assemble("  ", &[]).unwrap(), "");
    }

    #[test]
    fn missing_fragment_is_an_error() {
        let dir = dir("missing");
        assert!(assemble("no-existe", &[dir]).is_err());
    }
}
