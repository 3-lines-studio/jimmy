use crate::api;
use crate::icons::Icon;
use dioxus::prelude::*;
use serde::Deserialize;
use uicore::markdown;
use uicore::tool::{self, Line};

#[derive(Deserialize, Clone, PartialEq, Debug)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Ready,
    Presence {
        #[serde(default)]
        users: Vec<String>,
    },
    Online {
        #[serde(default)]
        users: Vec<String>,
    },
    Typing {
        #[serde(default)]
        user: String,
    },
    Stopped {
        #[serde(default)]
        author: String,
    },
    User {
        #[serde(default)]
        text: String,
        #[serde(default)]
        author: String,
        #[serde(default)]
        images: Vec<String>,
    },
    Image {
        #[serde(default)]
        name: String,
        #[serde(default)]
        caption: String,
    },
    Delta {
        #[serde(default)]
        text: String,
    },
    Assistant {
        #[serde(default)]
        text: String,
    },
    ToolStart {
        #[serde(default)]
        id: String,
        #[serde(default)]
        name: String,
        #[serde(default)]
        args: String,
    },
    ToolDelta {
        #[serde(default)]
        id: String,
        #[serde(default)]
        text: String,
    },
    ToolResult {
        #[serde(default)]
        id: String,
        #[serde(default)]
        text: String,
        #[serde(default)]
        ms: u64,
        #[serde(default)]
        failed: bool,
    },
    Done {
        #[serde(default)]
        text: String,
    },
    Error {
        #[serde(default)]
        message: String,
    },
    Synced {
        #[serde(default)]
        count: usize,
        #[serde(default)]
        first: usize,
    },
}

impl Event {
    pub fn parse(value: &serde_json::Value) -> Option<Self> {
        serde_json::from_value(value.clone()).ok()
    }

    pub fn logged(&self) -> bool {
        !matches!(
            self,
            Event::Delta { .. }
                | Event::ToolDelta { .. }
                | Event::Typing { .. }
                | Event::Presence { .. }
                | Event::Online { .. }
                | Event::Ready
                | Event::Synced { .. }
        )
    }
}

#[derive(Clone, PartialEq)]
pub struct Item {
    pub id: String,
    pub name: String,
    pub args: String,
    pub output: String,
    pub streamed: String,
    pub ms: Option<u64>,
    pub failed: bool,
    pub stat: String,
}

#[derive(Clone, PartialEq)]
pub struct Steps {
    pub items: Vec<Item>,
    pub ms: u64,
    pub failed: bool,
}

#[derive(Clone, PartialEq)]
pub enum Block {
    User {
        text: String,
        author: String,
        images: Vec<String>,
    },
    Image {
        name: String,
        caption: String,
    },
    Assistant {
        text: String,
        streaming: bool,
    },
    Steps(Steps),
    Error(String),
    Stopped(String),
}

pub fn blocks(events: &[Event]) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    for event in events {
        match event {
            Event::User {
                text,
                author,
                images,
            } => blocks.push(Block::User {
                text: text.clone(),
                author: author.clone(),
                images: images.clone(),
            }),
            Event::Image { name, caption } => blocks.push(Block::Image {
                name: name.clone(),
                caption: caption.clone(),
            }),
            Event::Delta { text } => match blocks.last_mut() {
                Some(Block::Assistant {
                    text: body,
                    streaming: true,
                }) => body.push_str(text),
                _ => blocks.push(Block::Assistant {
                    text: text.clone(),
                    streaming: true,
                }),
            },
            Event::Assistant { text } => match blocks.last_mut() {
                Some(Block::Assistant {
                    text: body,
                    streaming: _,
                }) => {
                    *body = text.clone();
                }
                _ => blocks.push(Block::Assistant {
                    text: text.clone(),
                    streaming: false,
                }),
            },
            Event::ToolStart { id, name, args } => {
                let (_, stat) = match name.as_str() {
                    "edit" => tool::diff_lines(args),
                    "write" => tool::content_lines(args),
                    _ => (Vec::new(), String::new()),
                };
                let item = Item {
                    id: id.clone(),
                    name: name.clone(),
                    args: args.clone(),
                    output: String::new(),
                    streamed: String::new(),
                    ms: None,
                    failed: false,
                    stat,
                };
                match blocks.last_mut() {
                    Some(Block::Steps(steps)) => steps.items.push(item),
                    _ => blocks.push(Block::Steps(Steps {
                        items: vec![item],
                        ms: 0,
                        failed: false,
                    })),
                }
            }
            Event::ToolDelta { id, text } => {
                if let Some(Block::Steps(steps)) = blocks.last_mut() {
                    if let Some(item) = steps.items.iter_mut().find(|item| &item.id == id) {
                        item.streamed.push_str(text);
                    }
                }
            }
            Event::ToolResult {
                id,
                text,
                ms,
                failed,
            } => {
                if let Some(Block::Steps(steps)) = blocks.last_mut() {
                    if let Some(item) = steps.items.iter_mut().find(|item| &item.id == id) {
                        item.output = text.clone();
                        item.ms = Some(*ms);
                        item.failed = *failed;
                    }
                    steps.ms += ms;
                    steps.failed = steps.failed || *failed;
                }
            }
            Event::Error { message } => blocks.push(Block::Error(message.clone())),
            Event::Stopped { author } => blocks.push(Block::Stopped(author.clone())),
            _ => {}
        }
    }
    blocks
}

pub fn title_of(blocks: &[Block]) -> Option<String> {
    for block in blocks {
        if let Block::User { text, .. } = block {
            let line = text.lines().find(|line| !line.trim().is_empty())?;
            return Some(line.chars().take(120).collect());
        }
    }
    None
}

#[component]
pub fn Thumbs(conversation: String, images: Vec<String>) -> Element {
    rsx! {
        div { class: "thumbs",
            for name in images {
                a {
                    key: "{name}",
                    href: api::file_path(&conversation, &name),
                    target: "_blank",
                    rel: "noreferrer",
                    img { src: api::file_path(&conversation, &name), alt: "{name}" }
                }
            }
        }
    }
}

#[component]
fn Diff(lines: Vec<Line>) -> Element {
    rsx! {
        for (index, line) in lines.iter().enumerate() {
            match line {
                Line::Path(path) => rsx! { div { key: "{index}", class: "diff-path", "{path}" } },
                Line::Add(text) => rsx! { div { key: "{index}", class: "diff-line add", "{text}" } },
                Line::Del(text) => rsx! { div { key: "{index}", class: "diff-line del", "{text}" } },
            }
        }
    }
}

#[component]
fn Tool(item: Item, workspace: String) -> Element {
    let described = tool::describe(&item.name, &item.args, &workspace);
    let stat = if item.stat.is_empty() {
        String::new()
    } else {
        format!("{} · ", item.stat)
    };
    let meta = match item.ms {
        None => "…".to_string(),
        Some(ms) => format!(
            "{stat}{} · {ms} ms",
            if item.failed { "error" } else { "ok" }
        ),
    };
    let class = match (item.failed, item.ms.is_some()) {
        (true, _) => "tool failed",
        (false, true) => "tool ok",
        (false, false) => "tool",
    };
    let (lines, _) = match item.name.as_str() {
        "edit" => tool::diff_lines(&item.args),
        "write" => tool::content_lines(&item.args),
        _ => (Vec::new(), String::new()),
    };
    let output = match (item.output.is_empty(), item.streamed.is_empty()) {
        (false, _) => item.output.clone(),
        (true, false) => item.streamed.clone(),
        (true, true) => String::new(),
    };
    rsx! {
        details { class: "{class}", "data-tool": "{item.name}",
            summary {
                span { class: "icon", Icon { name: "{super::icons::tool_icon(&item.name)}", size: 13 } }
                span { class: "name", "{item.name}" }
                span { class: "detail",
                    if !described.dir.is_empty() {
                        span { class: "dir",
                            Icon { name: "folder", size: 12 }
                            "{described.dir}"
                        }
                    }
                    "{described.text}"
                }
                span { class: "meta", "{meta}" }
            }
            div { class: "body",
                if lines.is_empty() {
                    pre { "{output}" }
                } else {
                    Diff { lines: lines.clone() }
                    if !item.output.is_empty() {
                        div { class: "diff-result", "{item.output}" }
                    }
                }
            }
        }
    }
}

#[component]
pub fn Transcript(
    id: String,
    conversation: String,
    workspace: String,
    events: Vec<Event>,
) -> Element {
    let blocks = blocks(&events);
    rsx! {
        div { class: "transcript", id: "{id}",
            for (index, block) in blocks.iter().enumerate() {
                match block {
                    Block::User { text, author, images } => rsx! {
                        div { key: "{index}", class: "event user",
                            if !author.is_empty() {
                                div { class: "author", "{author}" }
                            }
                            "{text}"
                            if !images.is_empty() {
                                Thumbs { conversation: conversation.clone(), images: images.clone() }
                            }
                        }
                    },
                    Block::Image { name, caption } => rsx! {
                        div { key: "{index}", class: "event image",
                            Thumbs { conversation: conversation.clone(), images: vec![name.clone()] }
                            if !caption.is_empty() {
                                div { class: "caption", "{caption}" }
                            }
                        }
                    },
                    Block::Assistant { text, streaming } => rsx! {
                        div {
                            key: "{index}",
                            class: if *streaming { "event assistant streaming" } else { "event assistant" },
                            if *streaming {
                                "{text}"
                            } else {
                                div { dangerous_inner_html: markdown::render(text) }
                            }
                        }
                    },
                    Block::Steps(steps) => {
                        let names: Vec<String> = steps.items.iter().map(|item| item.name.clone()).collect();
                        let label = tool::steps_label(&names);
                        let elapsed = match steps.ms {
                            0 => String::new(),
                            ms => format!("· {}", tool::elapsed(ms)),
                        };
                        rsx! {
                            details {
                                key: "{index}",
                                class: if steps.failed { "steps failed" } else { "steps" },
                                summary {
                                    span { class: "caret", "▶" }
                                    span { class: "label", "{label}" }
                                    span { class: "elapsed", "{elapsed}" }
                                }
                                div { class: "list",
                                    for item in steps.items.clone() {
                                        Tool { key: "{item.id}", item: item.clone(), workspace: workspace.clone() }
                                    }
                                }
                            }
                        }
                    },
                    Block::Error(message) => rsx! {
                        div { key: "{index}", class: "event error", "{message}" }
                    },
                    Block::Stopped(author) => rsx! {
                        div { key: "{index}", class: "event stopped", "{author} frenó el turno" }
                    },
                }
            }
        }
    }
}
