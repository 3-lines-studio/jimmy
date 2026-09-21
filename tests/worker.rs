//! The worker as a process: it boots from the environment, speaks the JSONL
//! protocol, resumes a turn that was cut short and runs a whole turn against
//! a model it does not know is fake.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};

struct Worker {
    child: Child,
    stdin: ChildStdin,
    lines: std::io::Lines<BufReader<std::process::ChildStdout>>,
}

impl Worker {
    fn start(root: &Path, base: &str) -> Worker {
        let mut child = Command::new(env!("CARGO_BIN_EXE_jimmy"))
            .args(["worker", "--chat", "test"])
            .env("OPENAI_API_KEY", "test")
            .env("JIMMY_ROOT", root)
            .env("JIMMY_WORKSPACE", root.join("workspace"))
            .env("JIMMY_PROMPT", "jimmy")
            .env("AXE_BASE", base)
            .env("AXE_MODEL", "fake")
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
    std::fs::write(root.join("prompts/jimmy.md"), "sos jimmy, un ayudante.").unwrap();
    root
}

/// The smallest OpenAI-compatible endpoint: one streamed sentence.
fn model_server() -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        if let Some(Ok(mut stream)) = listener.incoming().next() {
            let mut request = Vec::new();
            let mut chunk = [0u8; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => request.extend_from_slice(&chunk[..read]),
                }
            }
            let body = "data: {\"choices\":[{\"delta\":{\"content\":\"hola desde el fake\"}}]}\n\n\
                        data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
                        data: [DONE]\n\n";
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
        }
    });
    (format!("http://127.0.0.1:{port}/v1"), handle)
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

    let mut worker = Worker::start(&root, "http://127.0.0.1:1/v1");
    worker.send("{\"cmd\":\"resume\"}");
    let answer = worker.next();
    assert!(answer.contains("\"answer\""), "{answer}");
    assert!(answer.contains("listo"), "{answer}");
    drop(worker);
    assert!(
        !chat.join("inflight").exists(),
        "el marcador quedó sin limpiar"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn runs_a_turn_against_the_model_and_leaves_a_transcript() {
    let root = scratch("prompt");
    let chat = root.join("chats/test");
    let (base, server) = model_server();

    let mut worker = Worker::start(&root, &base);
    worker.send("{\"cmd\":\"prompt\",\"text\":\"hola\"}");
    let answer = worker.next();
    assert!(answer.contains("\"answer\""), "{answer}");
    assert!(answer.contains("hola desde el fake"), "{answer}");
    drop(worker);

    let transcript = std::fs::read_to_string(chat.join("transcript.jsonl")).unwrap();
    assert!(transcript.contains("hola"), "{transcript}");
    assert!(transcript.contains("hola desde el fake"), "{transcript}");
    assert!(!chat.join("inflight").exists());

    server.join().unwrap();
    std::fs::remove_dir_all(&root).unwrap();
}
