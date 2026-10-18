#![allow(clippy::useless_format)]

use dioxus::prelude::*;
use gloo_net::http::Request;
use serde::Deserialize;

const TAIL: usize = 1_000_000;

fn main() {
    dioxus::launch(app);
}

#[derive(Deserialize, Clone, PartialEq)]
struct Conversation {
    key: String,
    title: String,
    project: String,
    running: bool,
    read_only: bool,
}

#[derive(Deserialize, Clone, PartialEq)]
struct Project {
    name: String,
    conversations: Vec<Conversation>,
}

#[derive(Deserialize)]
struct State {
    projects: Vec<Project>,
}

#[derive(Deserialize)]
struct History {
    events: Vec<serde_json::Value>,
}

async fn state() -> Result<Vec<Project>, String> {
    let answer = Request::get("/api/state")
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if answer.status() != 200 {
        return Err(match answer.status() {
            401 => "inicia sesión en la v1 primero".to_string(),
            status => format!("estado {status}"),
        });
    }
    let state: State = answer.json().await.map_err(|error| error.to_string())?;
    Ok(state.projects)
}

async fn history(key: &str) -> Result<Vec<serde_json::Value>, String> {
    let answer = Request::get(&format!("/api/history?conversation={key}&before={TAIL}"))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if answer.status() != 200 {
        return Err(format!("estado {}", answer.status()));
    }
    let history: History = answer.json().await.map_err(|error| error.to_string())?;
    Ok(history.events)
}

async fn send(key: &str, text: &str) -> Result<(), String> {
    let body = serde_json::json!({ "conversation": key, "text": text });
    let answer = Request::post("/api/send")
        .json(&body)
        .map_err(|error| error.to_string())?
        .send()
        .await
        .map_err(|error| error.to_string())?;
    match answer.status() {
        200 | 202 => Ok(()),
        status => Err(format!("estado {status}")),
    }
}

fn text_of<'a>(event: &'a serde_json::Value, name: &str) -> &'a str {
    event
        .get(name)
        .and_then(|value| value.as_str())
        .unwrap_or_default()
}

fn line(event: &serde_json::Value) -> Option<String> {
    Some(match event.get("event")?.as_str()? {
        "user" => format!("{}: {}", text_of(event, "author"), text_of(event, "text")),
        "assistant" => format!("jimmy: {}", text_of(event, "text")),
        "image" => format!("imagen: {}", text_of(event, "name")),
        "tool_start" => format!("· {}", text_of(event, "name")),
        "tool_result" => format!("· {}", text_of(event, "text")),
        "error" => text_of(event, "message").to_string(),
        "stopped" => format!("· {}", text_of(event, "author")),
        _ => return None,
    })
}

fn app() -> Element {
    let mut projects = use_signal(|| None::<Vec<Project>>);
    let mut events = use_signal(Vec::<serde_json::Value>::new);
    let mut selected = use_signal(|| None::<String>);
    let mut draft = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);

    use_future(move || async move {
        match state().await {
            Ok(list) => projects.set(Some(list)),
            Err(message) => error.set(Some(message)),
        }
    });

    let open = move |key: String| async move {
        selected.set(Some(key.clone()));
        match history(&key).await {
            Ok(list) => {
                events.set(list);
                error.set(None);
            }
            Err(message) => error.set(Some(message)),
        }
    };

    let submit = move |event: FormEvent| {
        event.prevent_default();
        let (Some(key), text) = (selected(), draft()) else {
            return;
        };
        if text.trim().is_empty() {
            return;
        }
        spawn(async move {
            match send(&key, &text).await {
                Ok(()) => {
                    draft.set(String::new());
                    match history(&key).await {
                        Ok(list) => events.set(list),
                        Err(message) => error.set(Some(message)),
                    }
                }
                Err(message) => error.set(Some(message)),
            }
        });
    };

    rsx! {
        div { class: "shell",
            aside { class: "sidebar",
                h1 { "jimmy v2" }
                for project in projects().unwrap_or_default() {
                    div { class: "project",
                        span { class: "name", "{project.name}" }
                        for conversation in project.conversations.clone() {
                            button {
                                class: if selected() == Some(conversation.key.clone()) { "conversation active" } else { "conversation" },
                                onclick: move |_| {
                                    let key = conversation.key.clone();
                                    spawn(open(key));
                                },
                                "{conversation.title}" }
                        }
                    }
                }
            }
            main { class: "pane",
                if let Some(message) = error() {
                    p { class: "error", "{message}" }
                }
                section { class: "events",
                    for event in events() {
                        if let Some(line) = line(&event) {
                            p { "{line}" }
                        }
                    }
                }
                form {
                    onsubmit: submit,
                    input {
                        value: "{draft}",
                        placeholder: "escribe",
                        oninput: move |event| draft.set(event.value()),
                    }
                    button { r#type: "submit", "enviar" }
                }
            }
        }
    }
}
