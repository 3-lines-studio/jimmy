use axe::Tool;
use serde::Deserialize;
use std::io::Write;
use std::process::{Command, Stdio};

pub fn all() -> Vec<Tool> {
    vec![search(), browse()]
}

#[derive(Deserialize)]
struct SearchArgs {
    query: String,
    count: Option<usize>,
}

fn search() -> Tool {
    let mut tool = axe::new_tool(
        "search",
        "Busca en la web y devuelve resultados numerados (título, URL y snippet).",
        r#"{"type":"object","properties":{"query":{"type":"string","description":"Consulta de búsqueda"},"count":{"type":"integer","description":"Cantidad de resultados (default 8)"}},"required":["query"]}"#,
        |args: SearchArgs| {
            let mut argv = vec!["search".to_string()];
            if let Some(count) = args.count {
                argv.push("-n".into());
                argv.push(count.to_string());
            }
            argv.push(args.query);
            or_error("search", run(&argv, None))
        },
    );
    tool.snippet = "Busca en la web (DuckDuckGo) y devuelve resultados.";
    tool
}

#[derive(Deserialize)]
struct BrowseArgs {
    url: Option<String>,
    steps: Option<Vec<serde_json::Value>>,
    session: Option<String>,
    size: Option<String>,
    slow: Option<u64>,
    #[serde(default)]
    record: bool,
    #[serde(default)]
    shot: bool,
}

fn browse() -> Tool {
    let mut tool = axe::new_tool(
        "browse",
        "Abre páginas con un Chromium real: ejecuta JavaScript y guarda capturas o video.",
        r#"{"type":"object","properties":{"url":{"type":"string","description":"URL a abrir (acción goto)"},"steps":{"type":"array","description":"Secuencia de pasos [{action,...}] corrida en un solo navegador (acción run)","items":{"type":"object"}},"session":{"type":"string","description":"Nombre de sesión para reusar cookies"},"size":{"type":"string","description":"Viewport WxH, default 1920x1080"},"slow":{"type":"integer","description":"Pausa en ms después de cada paso"},"record":{"type":"boolean","description":"Graba un video de la corrida"},"shot":{"type":"boolean","description":"Con url: guarda una captura PNG"}}}"#,
        |args: BrowseArgs| match browse_argv(&args) {
            Ok((argv, stdin)) => or_error("browse", run(&argv, stdin.as_deref())),
            Err(error) => format!("browse: {error}"),
        },
    );
    tool.snippet = "Abre una URL o corre pasos en un Chromium real.";
    tool.sequential = true;
    tool
}

fn browse_argv(args: &BrowseArgs) -> Result<(Vec<String>, Option<String>), String> {
    let mut argv = vec!["browse".to_string()];
    let stdin = match (&args.steps, &args.url) {
        (Some(steps), _) => {
            argv.push("run".into());
            argv.push("-".into());
            Some(serde_json::to_string(steps).map_err(|error| error.to_string())?)
        }
        (None, Some(url)) => {
            argv.push("goto".into());
            argv.push(url.clone());
            if args.shot {
                argv.push("--shot".into());
            }
            None
        }
        (None, None) => return Err("indicá url o steps".into()),
    };

    if let Some(session) = &args.session {
        argv.push("--session".into());
        argv.push(session.clone());
    }
    if let Some(size) = &args.size {
        argv.push("--size".into());
        argv.push(size.clone());
    }
    if let Some(slow) = args.slow {
        argv.push("--slow".into());
        argv.push(slow.to_string());
    }
    if args.record {
        argv.push("--record".into());
    }

    Ok((argv, stdin))
}

fn or_error(program: &str, result: Result<String, String>) -> String {
    match result {
        Ok(text) => text,
        Err(error) => format!("{program}: {error}"),
    }
}

fn run(argv: &[String], stdin: Option<&str>) -> Result<String, String> {
    let (program, args) = argv.split_first().ok_or("sin comando")?;
    let mut command = Command::new(program);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });

    let mut child = command.spawn().map_err(|error| error.to_string())?;
    if let Some(input) = stdin {
        let mut pipe = child.stdin.take().ok_or("sin stdin")?;
        pipe.write_all(input.as_bytes())
            .map_err(|error| error.to_string())?;
    }

    let output = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }

    let code = output
        .status
        .code()
        .map(|code| code.to_string())
        .unwrap_or_else(|| "señal".into());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    if stderr.is_empty() {
        Err(format!("terminó con {code}"))
    } else {
        Err(format!("terminó con {code}: {stderr}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(url: Option<&str>, steps: Option<Vec<serde_json::Value>>) -> BrowseArgs {
        BrowseArgs {
            url: url.map(str::to_string),
            steps,
            session: None,
            size: None,
            slow: None,
            record: false,
            shot: false,
        }
    }

    #[test]
    fn goto_builds_goto() {
        let mut input = args(Some("https://example.com"), None);
        input.shot = true;
        input.session = Some("demo".into());
        let (argv, stdin) = browse_argv(&input).unwrap();
        assert_eq!(
            argv,
            vec![
                "browse",
                "goto",
                "https://example.com",
                "--shot",
                "--session",
                "demo"
            ]
        );
        assert!(stdin.is_none());
    }

    #[test]
    fn steps_build_run_on_stdin() {
        let steps = vec![serde_json::json!({"action": "goto", "url": "https://x"})];
        let mut input = args(None, Some(steps));
        input.record = true;
        input.slow = Some(600);
        let (argv, stdin) = browse_argv(&input).unwrap();
        assert_eq!(
            argv,
            vec!["browse", "run", "-", "--slow", "600", "--record"]
        );
        assert_eq!(stdin.unwrap(), r#"[{"action":"goto","url":"https://x"}]"#);
    }

    #[test]
    fn neither_url_nor_steps_errors() {
        assert!(browse_argv(&args(None, None)).is_err());
    }
}
