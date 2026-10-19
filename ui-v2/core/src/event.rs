use crate::tool;
use serde::Deserialize;

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

#[derive(Clone, PartialEq, Debug)]
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

#[derive(Clone, PartialEq, Debug)]
pub struct Steps {
    pub items: Vec<Item>,
    pub ms: u64,
    pub failed: bool,
}

#[derive(Clone, PartialEq, Debug)]
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
                    streaming,
                }) => {
                    *body = text.clone();
                    *streaming = false;
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


#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(line: &str) -> Event {
        Event::parse(&serde_json::from_str(line).unwrap()).unwrap()
    }

    #[test]
    fn the_events_parse_like_the_v1_protocol() {
        assert_eq!(
            parsed(r#"{"event":"user","text":"hola","author":"bob","images":[]}"#),
            Event::User {
                text: "hola".to_string(),
                author: "bob".to_string(),
                images: Vec::new()
            }
        );
        assert_eq!(
            parsed(r#"{"event":"tool_result","id":"t1","text":"ok","ms":7,"failed":false}"#),
            Event::ToolResult {
                id: "t1".to_string(),
                text: "ok".to_string(),
                ms: 7,
                failed: false
            }
        );
        assert_eq!(parsed(r#"{"event":"delta","text":"ho"}"#), Event::Delta { text: "ho".to_string() });
        assert!(Event::parse(&serde_json::json!({"event": "inventado"})).is_none());
        assert!(Event::parse(&serde_json::json!({"sin": "evento"})).is_none());
    }

    #[test]
    fn only_the_logged_events_count() {
        assert!(!Event::Delta { text: String::new() }.logged());
        assert!(!Event::ToolDelta { id: String::new(), text: String::new() }.logged());
        assert!(!Event::Typing { user: String::new() }.logged());
        assert!(!Event::Synced { count: 1, first: 0 }.logged());
        assert!(!Event::Ready.logged());
        assert!(Event::Assistant { text: "hola".to_string() }.logged());
        assert!(Event::User { text: "hola".to_string(), author: "bob".to_string(), images: vec![] }.logged());
    }

    #[test]
    fn the_final_answer_stops_streaming_so_it_gets_rendered() {
        let events = vec![
            Event::Delta { text: "Mirá: ```ru".to_string() },
            Event::Assistant {
                text: "Mirá: ```rust\nfn main() {}\n```".to_string(),
            },
        ];
        let list = blocks(&events);
        match &list[0] {
            Block::Assistant { streaming, .. } => assert!(!streaming, "{list:?}"),
            other => panic!("esperaba la respuesta, vino {other:?}"),
        }
        let later = blocks(&[
            events[0].clone(),
            events[1].clone(),
            Event::Delta { text: "más".to_string() },
        ]);
        assert_eq!(later.len(), 2, "{later:?}");
    }

    #[test]
    fn the_blocks_group_the_steps_and_replace_the_streaming_text() {
        let events = vec![
            Event::User {
                text: "arreglá el bug".to_string(),
                author: "bob".to_string(),
                images: Vec::new(),
            },
            Event::Delta { text: "mi".to_string() },
            Event::Delta { text: "rando".to_string() },
            Event::Assistant { text: "mirando".to_string() },
            Event::ToolStart {
                id: "t1".to_string(),
                name: "bash".to_string(),
                args: "{}".to_string(),
            },
            Event::ToolResult {
                id: "t1".to_string(),
                text: "ok".to_string(),
                ms: 5,
                failed: false,
            },
            Event::Done { text: String::new() },
        ];
        let blocks = blocks(&events);
        assert_eq!(blocks.len(), 3, "{blocks:?}");
        assert_eq!(title_of(&blocks).as_deref(), Some("arreglá el bug"));
        match &blocks[2] {
            Block::Steps(steps) => {
                assert_eq!(steps.ms, 5);
                assert_eq!(steps.items.len(), 1);
                assert_eq!(steps.items[0].output, "ok");
            }
            other => panic!("esperaba pasos, vino {other:?}"),
        }
    }
}
