use std::path::{Path, PathBuf};

pub const DEFAULT: &str =
    "identidad,estilo,codigo,jimmy,herramientas,skills,dev,workspace,memoria,agenda,git";
const BUILTIN: &str = "/usr/local/share/jimmy/prompts";

pub fn dirs(root: &Path) -> Vec<PathBuf> {
    vec![root.join("prompts"), PathBuf::from(BUILTIN)]
}

pub fn parse_vars(spec: &str) -> Vec<(String, String)> {
    spec.split(',')
        .filter_map(|pair| pair.split_once('='))
        .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
        .collect()
}

pub fn assemble(spec: &str, dirs: &[PathBuf], vars: &[(String, String)]) -> Result<String, String> {
    let mut parts = Vec::new();
    for name in spec.split(',').map(str::trim).filter(|n| !n.is_empty()) {
        parts.push(fill(&fragment(name, dirs)?, vars)?);
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

fn fill(text: &str, vars: &[(String, String)]) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 2..];
        let Some(end) = tail.find("}}") else {
            return Err(format!("placeholder sin cerrar en `{}`", &rest[start..]));
        };
        let key = tail[..end].trim();
        let Some((_, value)) = vars.iter().find(|(name, _)| name == key) else {
            return Err(format!("falta `{{{key}}}` en JIMMY_VARS"));
        };
        out.push_str(value);
        rest = &tail[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
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

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn joins_fragments_in_spec_order() {
        let dir = dir("order");
        write(&dir, "uno", "primero\n");
        write(&dir, "dos", "segundo\n");
        assert_eq!(
            assemble("uno, dos", &[dir], &[]).unwrap(),
            "primero\n\nsegundo"
        );
    }

    #[test]
    fn earlier_dir_wins() {
        let over = dir("over");
        let under = dir("under");
        write(&over, "identidad", "mía");
        write(&under, "identidad", "de fábrica");
        assert_eq!(assemble("identidad", &[over, under], &[]).unwrap(), "mía");
    }

    #[test]
    fn empty_spec_assembles_nothing() {
        assert_eq!(assemble("  ", &[], &[]).unwrap(), "");
    }

    #[test]
    fn missing_fragment_is_an_error() {
        let dir = dir("missing");
        assert!(assemble("no-existe", &[dir], &[]).is_err());
    }

    #[test]
    fn fills_placeholders() {
        let dir = dir("fill");
        write(
            &dir,
            "identidad",
            "Sos {{asistente}}, el asistente de {{usuario}}.\n",
        );
        let vars = vars(&[("usuario", "Don Berti"), ("asistente", "Jimmy")]);
        assert_eq!(
            assemble("identidad", &[dir], &vars).unwrap(),
            "Sos Jimmy, el asistente de Don Berti."
        );
    }

    #[test]
    fn leaves_single_braces_alone() {
        let dir = dir("braces");
        write(&dir, "herramientas", "`[{\"action\":\"goto\"}]`\n");
        assert_eq!(
            assemble("herramientas", &[dir], &vars(&[("usuario", "x")])).unwrap(),
            "`[{\"action\":\"goto\"}]`"
        );
    }

    #[test]
    fn missing_var_is_an_error() {
        let dir = dir("missing-var");
        write(&dir, "identidad", "Hola {{usuario}}");
        assert!(assemble("identidad", &[dir], &[]).is_err());
    }

    #[test]
    fn unclosed_placeholder_is_an_error() {
        let dir = dir("unclosed");
        write(&dir, "identidad", "Hola {{usuario");
        assert!(assemble("identidad", &[dir], &vars(&[("usuario", "x")])).is_err());
    }

    #[test]
    fn parses_clave_valor_pairs() {
        assert_eq!(
            parse_vars("usuario=Don Berti, asistente = Jimmy"),
            vars(&[("usuario", "Don Berti"), ("asistente", "Jimmy")])
        );
        assert_eq!(parse_vars("basura"), Vec::<(String, String)>::new());
    }
}
