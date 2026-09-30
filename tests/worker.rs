//! The worker as a process: it boots from the environment, speaks the JSONL
//! protocol, resumes a turn that was cut short, runs a whole turn against a
//! model it does not know is fake, and tells every step of the way.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

struct Worker {
    child: Child,
    stdin: ChildStdin,
    lines: std::io::Lines<BufReader<std::process::ChildStdout>>,
}

impl Worker {
    fn start(root: &Path, base: &str, cwd: &Path) -> Worker {
        let mut child = Command::new(env!("CARGO_BIN_EXE_jimmy"));
        child
            .args(["worker", "--chat", "test"])
            .args(["--cwd".as_ref(), cwd.as_os_str()])
            .env("OPENAI_API_KEY", "test")
            .env("JIMMY_ROOT", root)
            .env("JIMMY_WORKSPACE", root.join("workspace"))
            .env("JIMMY_PROMPT", "jimmy")
            .env("AXE_BASE", base)
            .env("AXE_MODEL", "fake");
        let mut child = child
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut worker = Worker {
            child,
            stdin,
            lines,
        };
        assert!(worker.next().contains("\"ready\""));
        worker
    }

    fn next(&mut self) -> String {
        self.lines
            .next()
            .expect("el worker cerró la salida")
            .expect("no pude leer del worker")
    }

    /// Reads until the turn ends, whatever happened in between.
    fn until_done(&mut self) -> Vec<String> {
        let mut events = Vec::new();
        loop {
            let event = self.next();
            let done = event.contains("\"done\"") || event.contains("\"error\"");
            events.push(event);
            if done {
                return events;
            }
        }
    }

    fn send(&mut self, command: &str) {
        writeln!(self.stdin, "{command}").unwrap();
        self.stdin.flush().unwrap();
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, "{{\"cmd\":\"shutdown\"}}");
        let _ = self.stdin.flush();
        let _ = self.child.wait();
    }
}

fn scratch(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("jimmy-worker-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("chats/test")).unwrap();
    std::fs::create_dir_all(root.join("prompts")).unwrap();
    std::fs::create_dir_all(root.join("workspace")).unwrap();
    std::fs::write(root.join("prompts/jimmy.md"), "sos jimmy, un ayudante.").unwrap();
    root
}

fn answer_chunk(text: &str) -> String {
    format!(
        "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{text}\"}}}}]}}\n\n\
         data: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\n\
         data: [DONE]\n\n"
    )
}

fn tool_chunk(command: &str) -> String {
    format!(
        "data: {{\"choices\":[{{\"delta\":{{\"tool_calls\":[{{\"index\":0,\"id\":\"call_1\",\
         \"function\":{{\"name\":\"bash\",\"arguments\":\"{{\\\"command\\\":\\\"{command}\\\"}}\"}}}}]}}}}]}}\n\n\
         data: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"tool_calls\"}}]}}\n\n\
         data: [DONE]\n\n"
    )
}

/// An OpenAI-compatible endpoint that answers with one body per request, in
/// order. Returns the base URL and how many requests it served.
fn model_server(bodies: Vec<String>) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let served = Arc::new(AtomicUsize::new(0));
    let counter = served.clone();
    std::thread::spawn(move || {
        while let Some(Ok(mut stream)) = listener.incoming().next() {
            let mut request = Vec::new();
            let mut chunk = [0u8; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => request.extend_from_slice(&chunk[..read]),
                }
            }
            let index = counter.fetch_add(1, Ordering::SeqCst);
            let Some(body) = bodies.get(index) else {
                break;
            };
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
        }
    });
    (format!("http://127.0.0.1:{port}/v1"), served)
}

#[test]
fn resumes_a_turn_that_was_cut_short() {
    let root = scratch("resume");
    let chat = root.join("chats/test");
    std::fs::write(
        chat.join("transcript.jsonl"),
        "{\"type\":\"message\",\"message\":{\"Role\":\"user\",\"Content\":\"hola\"}}\n\
         {\"type\":\"message\",\"message\":{\"Role\":\"assistant\",\"Content\":\"listo\"}}\n",
    )
    .unwrap();
    std::fs::write(chat.join("inflight"), b"").unwrap();

    let mut worker = Worker::start(&root, "http://127.0.0.1:1/v1", &root.join("workspace"));
    worker.send("{\"cmd\":\"resume\"}");
    let answer = worker.next();
    assert!(answer.contains("\"done\""), "{answer}");
    assert!(answer.contains("listo"), "{answer}");
    drop(worker);
    assert!(
        !chat.join("inflight").exists(),
        "el marcador quedó sin limpiar"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

/// Takes the turn of that chat the way another instance would: the same
/// `inflight` file, locked until the returned handle goes away.
fn hold_the_turn(chat: &Path) -> std::fs::File {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(chat.join("inflight"))
        .unwrap();
    let taken = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    assert_eq!(taken, 0, "no pude tomar el turno");
    file
}

#[test]
fn a_resume_that_another_instance_owns_stays_quiet() {
    let root = scratch("locked-resume");
    let chat = root.join("chats/test");
    std::fs::write(chat.join("transcript.jsonl"), "").unwrap();
    let held = hold_the_turn(&chat);

    let mut worker = Worker::start(&root, "http://127.0.0.1:1/v1", &root.join("workspace"));
    worker.send("{\"cmd\":\"resume\"}");
    worker.send("{\"cmd\":\"prompt\",\"text\":\"hola\"}");
    let events = worker.until_done();
    let first = events.first().unwrap();
    assert!(
        first.contains("otra instancia"),
        "el resume tenía que quedarse callado y el prompt avisar: {events:?}"
    );
    drop(worker);
    drop(held);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_prompt_that_loses_the_race_leaves_nothing_behind() {
    let root = scratch("locked-prompt");
    let chat = root.join("chats/test");
    let (base, served) = model_server(vec![answer_chunk("segundo")]);

    let mut worker = Worker::start(&root, &base, &root.join("workspace"));
    let held = hold_the_turn(&chat);
    worker.send("{\"cmd\":\"prompt\",\"text\":\"primero\"}");
    let refused = worker.until_done();
    assert!(
        refused.last().unwrap().contains("otra instancia"),
        "{refused:?}"
    );
    drop(held);

    worker.send("{\"cmd\":\"prompt\",\"text\":\"segundo\"}");
    let events = worker.until_done();
    assert!(events.last().unwrap().contains("segundo"), "{events:?}");
    assert_eq!(served.load(Ordering::SeqCst), 1, "solo el segundo turno");
    drop(worker);

    let transcript = std::fs::read_to_string(chat.join("transcript.jsonl")).unwrap();
    assert!(!transcript.contains("primero"), "{transcript}");
    assert!(transcript.contains("segundo"), "{transcript}");
    assert!(!chat.join("inflight").exists());
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn runs_a_turn_against_the_model_and_leaves_a_transcript() {
    let root = scratch("prompt");
    let chat = root.join("chats/test");
    let (base, served) = model_server(vec![answer_chunk("hola desde el fake")]);

    let mut worker = Worker::start(&root, &base, &root.join("workspace"));
    worker.send("{\"cmd\":\"prompt\",\"text\":\"hola\",\"author\":\"ana\"}");
    let events = worker.until_done();
    assert_eq!(served.load(Ordering::SeqCst), 1);
    assert!(
        events.iter().any(|event| event.contains("\"assistant\"")),
        "esperaba el mensaje del asistente: {events:?}"
    );
    assert!(events.last().unwrap().contains("hola desde el fake"));
    drop(worker);

    let transcript = std::fs::read_to_string(chat.join("transcript.jsonl")).unwrap();
    assert!(
        transcript.contains("[ana] hola"),
        "el modelo tiene que saber quién le habla: {transcript}"
    );
    assert!(transcript.contains("hola desde el fake"), "{transcript}");
    assert!(!chat.join("inflight").exists());
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn compacting_answers_with_what_it_summarized() {
    let root = scratch("compact");
    let chat = root.join("chats/test");
    let (base, _) = model_server(vec![
        answer_chunk("uno"),
        answer_chunk("dos"),
        answer_chunk("tres"),
        answer_chunk("un resumen"),
        answer_chunk("un resumen"),
    ]);

    let mut worker = Worker::start(&root, &base, &root.join("workspace"));
    for text in ["uno", "dos", "tres"] {
        worker.send(&format!("{{\"cmd\":\"prompt\",\"text\":\"{text}\"}}"));
        worker.until_done();
    }
    worker.send("{\"cmd\":\"compact\"}");
    let events = worker.until_done();
    let done = events.last().unwrap();
    assert!(done.contains("\"done\""), "{done}");
    assert!(done.contains("compactado"), "{done}");
    drop(worker);

    let transcript = std::fs::read_to_string(chat.join("transcript.jsonl")).unwrap();
    assert!(
        transcript.contains("\"type\":\"compaction\""),
        "{transcript}"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_tool_call_shows_up_as_events() {
    let root = scratch("tool");
    let (base, served) = model_server(vec![tool_chunk("echo hola"), answer_chunk("listo")]);

    let mut worker = Worker::start(&root, &base, &root.join("workspace"));
    worker.send("{\"cmd\":\"prompt\",\"text\":\"corré echo hola\"}");
    let events = worker.until_done();
    assert_eq!(
        served.load(Ordering::SeqCst),
        2,
        "el modelo se llama una vez por vuelta del loop"
    );

    let start = events.iter().find(|e| e.contains("tool_start")).unwrap();
    assert!(start.contains("\"name\":\"bash\""), "{start}");
    assert!(start.contains("echo hola"), "{start}");
    let result = events.iter().find(|e| e.contains("tool_result")).unwrap();
    assert!(result.contains("hola"), "{result}");
    assert!(result.contains("\"failed\":false"), "{result}");
    let done = events.last().unwrap();
    assert!(done.contains("\"done\""), "{done}");
    assert!(done.contains("listo"), "{done}");
    std::fs::remove_dir_all(&root).unwrap();
}

/// Two tool calls in one assistant message: they run in parallel and each
/// result lands when it is ready, not in call order.
fn parallel_tool_chunk(first: &str, second: &str) -> String {
    let args = |command: &str| serde_json::json!({"command": command}).to_string();
    let calls = serde_json::json!({
        "choices": [{"delta": {"tool_calls": [
            {"index": 0, "id": "call_1", "function": {"name": "bash", "arguments": args(first)}},
            {"index": 1, "id": "call_2", "function": {"name": "bash", "arguments": args(second)}}
        ]}}]
    });
    format!(
        "data: {calls}\n\n\
         data: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"tool_calls\"}}]}}\n\n\
         data: [DONE]\n\n"
    )
}

/// Whatever the order they finish in, the results stay with the call that
/// asked for them: the provider rejects anything else.
#[test]
fn a_parallel_batch_lands_next_to_its_call() {
    let root = scratch("batch");
    let chat = root.join("chats/test");
    let (base, _) = model_server(vec![
        parallel_tool_chunk("sleep 2", "echo rapido"),
        answer_chunk("listo"),
    ]);

    let mut worker = Worker::start(&root, &base, &root.join("workspace"));
    worker.send("{\"cmd\":\"prompt\",\"text\":\"dos cosas\"}");
    worker.until_done();
    drop(worker);

    let transcript = std::fs::read_to_string(chat.join("transcript.jsonl")).unwrap();
    let roles: Vec<String> = transcript
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|entry| Some(entry.get("message")?.get("Role")?.as_str()?.to_string()))
        .collect();
    assert_eq!(roles, ["user", "assistant", "tool", "tool", "assistant"]);
    std::fs::remove_dir_all(&root).unwrap();
}

/// El log de la conversación lo escribe el worker, que es el único que está
/// donde vive la conversación: la API de archivos no tiene append, así que hay
/// un solo escritor por archivo.
#[test]
fn the_log_lives_with_the_conversation() {
    let root = scratch("log");
    let chat = root.join("chats/test");
    let dibujo = root.join("workspace/dibujo.png");
    std::fs::write(&dibujo, b"\x89PNG\r\n\x1a\n y lo que siga").unwrap();
    std::fs::write(
        chat.join("meta.json"),
        r#"{"project":"general","title":"con adjuntos"}"#,
    )
    .unwrap();
    let (base, _) = model_server(vec![
        tool_chunk(&format!(
            "{} send {} --target test --caption 'un dibujo'",
            env!("CARGO_BIN_EXE_jimmy"),
            dibujo.display()
        )),
        answer_chunk("listo"),
    ]);

    let mut worker = Worker::start(&root, &base, &root.join("workspace"));
    worker.send("{\"cmd\":\"prompt\",\"text\":\"hola\",\"author\":\"ana\"}");
    worker.until_done();
    drop(worker);

    let log = std::fs::read_to_string(chat.join("conversation.jsonl")).unwrap();
    let first = log.lines().next().unwrap();
    assert!(
        first.contains("\"user\"") && first.contains("\"author\":\"ana\""),
        "el turno arranca con lo que dijo quien lo pidió: {log}"
    );
    assert!(log.contains("\"done\""), "{log}");
    assert!(
        log.contains("\"event\":\"image\"") && log.contains("un dibujo"),
        "lo que el asistente manda va al log: {log}"
    );
    assert!(
        !chat.join("outbox.jsonl").exists(),
        "la cola queda vacía: si no, se manda dos veces"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

/// Quién frenó el turno lo dice el worker, que es el que lo sabe: el padre
/// sólo pide la interrupción.
#[test]
fn a_cancel_says_who_stopped_the_turn() {
    let root = scratch("cancel");
    let chat = root.join("chats/test");
    let (base, _) = model_server(vec![tool_chunk("sleep 2"), answer_chunk("tarde")]);

    let mut worker = Worker::start(&root, &base, &root.join("workspace"));
    worker.send("{\"cmd\":\"prompt\",\"text\":\"hola larga\"}");
    let first = worker.next();
    assert!(first.contains("tool_start"), "{first}");
    worker.send("{\"cmd\":\"cancel\",\"author\":\"ana\"}");

    let log = esperar(&chat, "\"stopped\"");
    assert!(log.contains("\"stopped\""), "{log}");
    assert!(log.contains("\"author\":\"ana\""), "{log}");
    drop(worker);
    std::fs::remove_dir_all(&root).unwrap();
}

fn esperar(chat: &Path, needle: &str) -> String {
    let path = chat.join("conversation.jsonl");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        if text.contains(needle) || std::time::Instant::now() > deadline {
            return text;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn the_tools_run_in_the_project_the_conversation_belongs_to() {
    let root = scratch("cwd");
    let project = root.join("workspace/projects/ken");
    std::fs::create_dir_all(&project).unwrap();
    let (base, _) = model_server(vec![tool_chunk("pwd"), answer_chunk("listo")]);

    let mut worker = Worker::start(&root, &base, &project);
    worker.send("{\"cmd\":\"prompt\",\"text\":\"dónde estoy\"}");
    let events = worker.until_done();

    let result = events.iter().find(|e| e.contains("tool_result")).unwrap();
    assert!(
        result.contains(project.to_str().unwrap()),
        "esperaba el directorio del proyecto en {result}"
    );
    std::fs::remove_dir_all(&root).unwrap();
}
