use crate::api;
use crate::icons::Icon;
use dioxus::prelude::*;
pub use uicore::event::{blocks, title_of, Block, Event, Item};
use uicore::markdown;
use uicore::tool::{self, Line};
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
                    Block::Assistant { text, streaming } => match streaming {
                        true => rsx! {
                            div {
                                key: "{index}",
                                class: "event assistant streaming",
                                "{text}"
                            }
                        },
                        false => rsx! {
                            div {
                                key: "{index}",
                                class: "event assistant",
                                dangerous_inner_html: markdown::render(text),
                            }
                        },
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
