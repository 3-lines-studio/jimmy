#![allow(clippy::useless_format)]

use dioxus::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

mod api;
mod event;
mod icons;
mod vault;

use event::{Event, Transcript};
use icons::Icon;
use vault::{open_vault, Pane as VaultPane};

const TAIL: usize = 1_000_000;
const VISIBLE: usize = 10;

type Sources = Rc<RefCell<HashMap<String, web_sys::EventSource>>>;

#[derive(Clone, PartialEq, Default)]
struct Tab {
    events: Vec<Event>,
    count: usize,
    first: usize,
    draft: String,
    pending: Vec<String>,
    viewers: Vec<String>,
    typing: String,
    running: bool,
    done: bool,
    error: bool,
}

#[derive(Clone, PartialEq)]
enum Content {
    Empty,
    Note(String),
    Text(String),
    Markdown(String),
    Image(String),
}

#[derive(Clone, PartialEq)]
struct FilesTab {
    project: String,
    root: Vec<api::Entry>,
    dirs: HashMap<String, Vec<api::Entry>>,
    open: Vec<String>,
    file: Option<String>,
    content: Content,
    showing: bool,
}

impl FilesTab {
    fn new(project: String) -> Self {
        Self {
            project,
            root: Vec::new(),
            dirs: HashMap::new(),
            open: Vec::new(),
            file: None,
            content: Content::Empty,
            showing: false,
        }
    }
}

#[derive(Clone, PartialEq, Default)]
pub(crate) struct App {
    pub(crate) state: Option<api::State>,
    pub(crate) error: Option<String>,
    pub(crate) tabs: Vec<String>,
    pub(crate) active: Option<String>,
    pub(crate) data: HashMap<String, Tab>,
    pub(crate) theme: String,
    pub(crate) toast: Option<String>,
    pub(crate) online: String,
    pub(crate) new_project: String,
    pub(crate) opened: Vec<String>,
    pub(crate) expanded: Vec<String>,
    pub(crate) menu: Option<String>,
    pub(crate) files: HashMap<String, FilesTab>,
    pub(crate) share: Option<String>,
    pub(crate) vault: vault::Vault,
    pub(crate) agenda: Option<Vec<api::Task>>,
    pub(crate) agenda_open: Vec<String>,
    pub(crate) agenda_pending: Vec<(String, i64)>,
}

fn main() {
    dioxus::launch(app);
}

fn stored(key: &str) -> Option<String> {
    web_sys::window()?
        .local_storage()
        .ok()??
        .get_item(key)
        .ok()?
}

pub(crate) fn toast(app: Signal<App>, text: &str) {
    let mut app = app;
    app.write().toast = Some(text.to_string());
    let mut later = app;
    spawn(async move {
        gloo_timers::future::TimeoutFuture::new(1500).await;
        later.write().toast = None;
    });
}

fn conversation_of(app: &App, key: &str) -> Option<api::Conversation> {
    app.state
        .as_ref()?
        .projects
        .iter()
        .flat_map(|project| project.conversations.iter())
        .find(|conversation| conversation.key == key)
        .cloned()
}

fn title_of(app: &App, key: &str) -> String {
    if key == "agenda" {
        return "Agenda".to_string();
    }
    if key == "vault" {
        return "Vault".to_string();
    }
    if let Some(title) = uicore::files::title(key) {
        return title;
    }
    conversation_of(app, key)
        .map(|conversation| conversation.title)
        .filter(|title| !title.is_empty())
        .or_else(|| event::title_of(&event::blocks(&app.data.get(key)?.events)))
        .unwrap_or_else(|| key.to_string())
}

fn project_of(app: &App, key: &str) -> String {
    if let Some(project) = uicore::files::project_of(key) {
        return project;
    }
    conversation_of(app, key)
        .map(|conversation| conversation.project)
        .unwrap_or_default()
}

fn avatar_state(app: &App, project: &api::Project, active: Option<&str>) -> String {
    let mut state = if app.opened.contains(&project.name) {
        "idle"
    } else {
        "sleeping"
    };
    for conversation in &project.conversations {
        if Some(conversation.key.as_str()) == active {
            state = "focused";
        }
        if let Some(tab) = app.data.get(&conversation.key) {
            if tab.error {
                return "error".to_string();
            }
            if tab.done {
                return "done".to_string();
            }
            if tab.running {
                state = "working";
            }
        } else if conversation.running && state == "idle" {
            state = "waiting";
        }
    }
    state.to_string()
}

fn toggle(list: &mut Vec<String>, name: &str) {
    match list.iter().position(|item| item == name) {
        Some(index) => {
            list.remove(index);
        }
        None => list.push(name.to_string()),
    }
}

fn remember_open(opened: &[String]) {
    let Some(storage) = web_sys::window().and_then(|window| window.local_storage().ok().flatten())
    else {
        return;
    };
    let value = serde_json::json!(opened);
    let _ = storage.set_item("jimmy-open-projects", &value.to_string());
}

fn ask(text: &str, fallback: &str) -> Option<String> {
    let window = web_sys::window()?;
    let answer = window
        .prompt_with_message_and_default(text, fallback)
        .ok()??;
    let answer = answer.trim().to_string();
    (!answer.is_empty()).then_some(answer)
}

fn confirm(text: &str) -> bool {
    web_sys::window()
        .and_then(|window| window.confirm_with_message(text).ok())
        .unwrap_or(false)
}

fn load_earlier(app: Signal<App>, key: String) {
    let before = app.read().data.get(&key).map(|tab| tab.first).unwrap_or(0);
    if before == 0 {
        return;
    }
    spawn(async move {
        let mut app = app;
        let Ok(history) = api::get::<api::History>(&api::history_path(&key, before)).await else {
            return;
        };
        let mut state = app.write();
        if let Some(tab) = state.data.get_mut(&key) {
            let mut older: Vec<Event> = history.events.iter().filter_map(Event::parse).collect();
            older.extend(tab.events.clone());
            tab.events = older;
            tab.first = history.first;
        }
    });
}

fn scroll_bottom(key: &str) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    if let Some(node) = document.get_element_by_id(&format!("transcript-{key}")) {
        node.set_scroll_top(node.scroll_height());
    }
}

fn date_of(date: &js_sys::Date) -> String {
    format!(
        "{:04}-{:02}-{:02}",
        date.get_full_year(),
        date.get_month() + 1,
        date.get_date()
    )
}

fn today() -> String {
    date_of(&js_sys::Date::new(&wasm_bindgen::JsValue::from(
        js_sys::Date::now(),
    )))
}

fn yesterday() -> String {
    date_of(&js_sys::Date::new(&wasm_bindgen::JsValue::from(
        js_sys::Date::now() - 86_400_000.0,
    )))
}

fn clock_of(ts: i64) -> String {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from(ts as f64 * 1000.0));
    format!("{:02}:{:02}", date.get_hours(), date.get_minutes())
}

fn ago_of(ts: i64) -> String {
    let now = (js_sys::Date::now() / 1000.0).round() as i64;
    uicore::agenda::ago(now - ts)
}

fn mark_of(app: &App, task: &api::Task) -> uicore::agenda::Mark {
    uicore::agenda::Mark {
        paused: task.paused,
        blocked: task.blocked,
        unread: task.unread,
        last_ok: task.runs.first().map(|run| run.ok),
        running: app
            .agenda_pending
            .iter()
            .any(|(name, _)| name == &task.name),
    }
}

fn open_agenda(mut app: Signal<App>) {
    {
        let mut state = app.write();
        if !state.tabs.contains(&"agenda".to_string()) {
            state.tabs.push("agenda".to_string());
        }
        state.active = Some("agenda".to_string());
    }
    load_agenda(app);
    remember_tabs(&app.read());
    remember_hash(Some("agenda"));
}

fn load_agenda(app: Signal<App>) {
    spawn(async move {
        let mut app = app;
        let Ok(agenda) = api::get::<api::Agenda>("/api/agenda").await else {
            return;
        };
        let mut state = app.write();
        state.agenda = Some(agenda.tasks.clone());
        state.agenda_pending.retain(|(name, baseline)| {
            agenda.tasks.iter().any(|task| {
                &task.name == name && task.runs.first().map(|run| run.ts) == Some(*baseline)
            })
        });
    });
}

fn run_agenda(app: Signal<App>, name: String) {
    spawn(async move {
        let mut app = app;
        let baseline = app
            .read()
            .agenda
            .as_ref()
            .and_then(|tasks| tasks.iter().find(|task| task.name == name))
            .and_then(|task| task.runs.first().map(|run| run.ts))
            .unwrap_or(0);
        {
            let mut state = app.write();
            if !state.agenda_pending.iter().any(|(item, _)| item == &name) {
                state.agenda_pending.push((name.clone(), baseline));
            }
        }
        let body = serde_json::json!({ "name": name });
        if let Err(message) = api::post_quiet("/api/agenda/run", &body).await {
            app.write().agenda_pending.retain(|(item, _)| item != &name);
            toast(app, &message);
            return;
        }
        gloo_timers::future::TimeoutFuture::new(5000).await;
        load_agenda(app);
    });
}

fn pause_agenda(app: Signal<App>, name: String, paused: bool) {
    spawn(async move {
        let app = app;
        let body = serde_json::json!({ "name": name, "paused": paused });
        match api::post_quiet("/api/agenda/pause", &body).await {
            Ok(()) => load_agenda(app),
            Err(message) => toast(app, &message),
        }
    });
}

fn read_agenda(app: Signal<App>, name: Option<String>) {
    spawn(async move {
        let app = app;
        let body = match name {
            Some(name) => serde_json::json!({ "name": name }),
            None => serde_json::json!({}),
        };
        match api::post_quiet("/api/agenda/read", &body).await {
            Ok(()) => load_agenda(app),
            Err(message) => toast(app, &message),
        }
    });
}

fn rename_conversation(app: Signal<App>, key: String) {
    let current = conversation_of(&app.read(), &key)
        .map(|conversation| conversation.title)
        .unwrap_or_default();
    let Some(title) = ask("Título de la conversación", &current) else {
        return;
    };
    if title == current {
        return;
    }
    spawn(async move {
        let app = app;
        let body = serde_json::json!({ "conversation": key, "title": title });
        match api::post_quiet("/api/rename", &body).await {
            Ok(()) => refresh(app),
            Err(message) => toast(app, &message),
        }
    });
}

fn remove_conversation(app: Signal<App>, sources: Sources, key: String) {
    let title = conversation_of(&app.read(), &key)
        .map(|conversation| conversation.title)
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| key.clone());
    if !confirm(&format!(
        "¿Borrar la conversación \"{title}\"? Se pierde el historial."
    )) {
        return;
    }
    spawn(async move {
        let app = app;
        let body = serde_json::json!({ "conversation": key });
        match api::post_quiet("/api/delete-conversation", &body).await {
            Ok(()) => {
                close_tab(app, sources, &key);
                refresh(app);
            }
            Err(message) => toast(app, &message),
        }
    });
}

fn take_share() -> Option<String> {
    let window = web_sys::window()?;
    let search = window.location().search().ok()?;
    let query = search.trim_start_matches('?');
    if query.is_empty() {
        return None;
    }
    let mut text = String::new();
    let mut url = String::new();
    for pair in query.split('&') {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = js_sys::decode_uri_component(value)
            .ok()
            .and_then(|value| value.as_string())
            .unwrap_or_default();
        match name {
            "text" => text = value,
            "url" => url = value,
            _ => {}
        }
    }
    let _ = window.history().and_then(|history| {
        history.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some("/v2"))
    });
    let text = text.trim().to_string();
    let url = url.trim().to_string();
    let combined = match (text.is_empty(), url.is_empty()) {
        (true, true) => return None,
        (_, true) => text,
        (true, _) => url,
        (false, false) if text.contains(&url) => text,
        (false, false) => format!("{text}\n{url}"),
    };
    Some(combined)
}

fn rename_project(app: Signal<App>, from: String) {
    let Some(name) = ask("Nombre del proyecto", &from) else {
        return;
    };
    if name == from {
        return;
    }
    spawn(async move {
        let app = app;
        let body = serde_json::json!({ "project": from, "name": name });
        match api::post_quiet("/api/rename-project", &body).await {
            Ok(()) => refresh(app),
            Err(message) => toast(app, &message),
        }
    });
}

fn duplicate_project(app: Signal<App>, from: String) {
    let Some(name) = ask("Nombre del proyecto nuevo", &format!("{from}-2")) else {
        return;
    };
    spawn(async move {
        let app = app;
        let body = serde_json::json!({ "project": from, "name": name });
        match api::post_quiet("/api/duplicate-project", &body).await {
            Ok(()) => refresh(app),
            Err(message) => toast(app, &message),
        }
    });
}

fn remove_project(app: Signal<App>, project: api::Project) {
    let text = format!("¿Quitar \"{}\" y todas sus conversaciones?", project.name);
    if !confirm(&text) {
        return;
    }
    if project.unversioned
        && !confirm(&format!(
            "\"{}\" no está en git. ¿Lo borro igual?",
            project.name
        ))
    {
        return;
    }
    spawn(async move {
        let app = app;
        let body = if project.unversioned {
            serde_json::json!({ "project": project.name, "force": true })
        } else {
            serde_json::json!({ "project": project.name })
        };
        match api::post_quiet("/api/delete-project", &body).await {
            Ok(()) => refresh(app),
            Err(message) => toast(app, &message),
        }
    });
}

fn apply(app: Signal<App>, key: &str, event: Event) {
    let mut app = app;
    let mut app = app.write();
    let Some(tab) = app.data.get_mut(key) else {
        return;
    };
    match event {
        Event::Synced { count, first } => {
            if count >= tab.count {
                tab.count = count;
                tab.first = first;
            }
        }
        Event::Presence { users } => tab.viewers = users,
        Event::Typing { user } => tab.typing = user,
        Event::Delta { .. } | Event::ToolDelta { .. } => {
            tab.typing.clear();
            tab.events.push(event);
        }
        Event::Done { .. } => {
            tab.running = false;
            tab.done = true;
            tab.events.push(event);
        }
        Event::Error { .. } => {
            tab.running = false;
            tab.error = true;
            tab.events.push(event);
        }
        Event::Assistant { .. } | Event::Stopped { .. } => {
            tab.running = false;
            tab.events.push(event);
        }
        event => {
            tab.count += usize::from(event.logged());
            tab.events.push(event);
        }
    }
}

fn subscribe(app: Signal<App>, sources: Sources, key: String) {
    if sources.borrow().contains_key(&key) {
        return;
    }
    let since = app.read().data.get(&key).map(|tab| tab.count).unwrap_or(0);
    let Ok(source) = web_sys::EventSource::new(&api::stream_path(&key, since)) else {
        return;
    };
    let signal = app;
    let scope = key.clone();
    let on =
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |message: web_sys::MessageEvent| {
            let Some(data) = message.data().as_string() else {
                return;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&data) else {
                return;
            };
            let Some(event) = Event::parse(&value) else {
                return;
            };
            apply(signal, &scope, event);
        });
    source.set_onmessage(Some(on.as_ref().unchecked_ref()));
    on.forget();
    sources.borrow_mut().insert(key, source);
}

fn unsubscribe(sources: &Sources, key: &str) {
    if let Some(source) = sources.borrow_mut().remove(key) {
        source.close();
    }
}

fn create_tab(mut app: Signal<App>, key: &str) {
    let mut state = app.write();
    if !state.tabs.contains(&key.to_string()) {
        state.tabs.push(key.to_string());
    }
    state.data.entry(key.to_string()).or_default();
}

fn load_tab(app: Signal<App>, sources: Sources, key: String) {
    let mut app_mut = app;
    let scope = key.clone();
    let loaded = app
        .read()
        .data
        .get(&key)
        .map(|tab| !tab.events.is_empty())
        .unwrap_or(false);
    if loaded {
        return;
    }
    spawn(async move {
        let Ok(history) = api::get::<api::History>(&api::history_path(&scope, TAIL)).await else {
            return;
        };
        let mut app_write = app_mut.write();
        let Some(tab) = app_write.data.get_mut(&scope) else {
            return;
        };
        if tab.events.is_empty() {
            tab.events = history
                .events
                .iter()
                .filter_map(Event::parse)
                .collect::<Vec<_>>();
            tab.first = history.first;
            tab.count = tab.events.iter().filter(|event| event.logged()).count();
        }
        drop(app_write);
        subscribe(app_mut, sources, scope);
    });
}

fn open_tab(app: Signal<App>, sources: Sources, key: String) {
    create_tab(app, &key);
    activate(app, sources, &key);
}

pub(crate) fn remember_tabs(app: &App) {
    let Some(storage) = web_sys::window().and_then(|window| window.local_storage().ok().flatten())
    else {
        return;
    };
    let value = serde_json::json!({ "open": app.tabs });
    let _ = storage.set_item("jimmy-tabs", &value.to_string());
}

pub(crate) fn remember_hash(key: Option<&str>) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let target = match key {
        Some(key) => format!("#{}", js_sys::encode_uri_component(key)),
        None => window
            .location()
            .pathname()
            .unwrap_or_else(|_| "/v2".to_string()),
    };
    let _ = window.history().and_then(|history| {
        history.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&target))
    });
}

fn restore_tabs(app: Signal<App>, sources: Sources) {
    let saved = stored("jimmy-tabs")
        .and_then(|value| serde_json::from_str::<serde_json::Value>(&value).ok())
        .and_then(|value| value.get("open").and_then(|open| open.as_array()).cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|value| value.as_str().map(str::to_string))
        .collect::<Vec<_>>();
    let hash = web_sys::window()
        .and_then(|window| window.location().hash().ok())
        .map(|hash| hash.trim_start_matches('#').to_string())
        .unwrap_or_default();
    let hash = js_sys::decode_uri_component(&hash)
        .ok()
        .and_then(|value| value.as_string())
        .unwrap_or_default();
    let mut wanted = saved.clone();
    if !hash.is_empty() && !wanted.contains(&hash) {
        wanted.push(hash.clone());
    }
    for key in wanted {
        if uicore::files::is_files(&key) {
            continue;
        }
        create_tab(app, &key);
    }
    let target = match hash.is_empty() {
        true => saved.last().cloned(),
        false => Some(hash),
    };
    if let Some(key) = target {
        if !key.is_empty() {
            activate(app, sources, &key);
        }
    }
}

fn close_tab(mut app: Signal<App>, sources: Sources, key: &str) {
    unsubscribe(&sources, key);
    {
        let mut state = app.write();
        state.tabs.retain(|tab| tab != key);
        state.data.remove(key);
        if state.active.as_deref() == Some(key) {
            state.active = state.tabs.last().cloned();
        }
        let active = state.active.clone();
        drop(state);
        remember_tabs(&app.read());
        remember_hash(active.as_deref());
    }
}

fn activate(mut app: Signal<App>, sources: Sources, key: &str) {
    let previous = app.read().active.clone();
    if let Some(previous) = previous.filter(|previous| previous != key) {
        unsubscribe(&sources, &previous);
    }
    let share = {
        let mut state = app.write();
        state.active = Some(key.to_string());
        if let Some(tab) = state.data.get_mut(key) {
            tab.done = false;
            tab.error = false;
            if tab.draft.is_empty() {
                let shared = state.share.take();
                if let Some(shared) = shared {
                    if let Some(tab) = state.data.get_mut(key) {
                        tab.draft = shared;
                    }
                }
            }
        }
        state.share.clone()
    };
    if share.is_some() {
        toast(app, "Compartiste algo: abrí una conversación y pegalo");
    }
    create_tab(app, key);
    load_tab(app, sources, key.to_string());
    remember_tabs(&app.read());
    remember_hash(Some(key));
}

fn refresh(app: Signal<App>) {
    spawn(async move {
        let mut app = app;
        match api::get::<api::State>(api::state_path()).await {
            Ok(state) => {
                let mut write = app.write();
                write.error = None;
                write.state = Some(state);
            }
            Err(message) if message == "no estás adentro" => {
                if let Some(window) = web_sys::window() {
                    let _ = window.location().set_href("/login");
                }
            }
            Err(message) => app.write().error = Some(message),
        }
    });
}

fn send(app: Signal<App>, key: String) {
    let (text, images) = {
        let state = app.read();
        match state.data.get(&key) {
            Some(tab) => (tab.draft.trim().to_string(), tab.pending.clone()),
            None => return,
        }
    };
    if text.is_empty() && images.is_empty() {
        return;
    }
    let request_id = js_sys::Reflect::get(&js_sys::global(), &js_sys::JsString::from("crypto"))
        .ok()
        .and_then(|crypto| js_sys::Reflect::get(&crypto, &"randomUUID".into()).ok())
        .and_then(|function| function.dyn_into::<js_sys::Function>().ok())
        .and_then(|function| {
            function
                .call0(&js_sys::Reflect::get(&js_sys::global(), &"crypto".into()).ok()?)
                .ok()
        })
        .and_then(|value| value.as_string())
        .unwrap_or_default();
    spawn(async move {
        let mut app = app;
        let body = serde_json::json!({
            "conversation": key,
            "text": text,
            "images": images,
            "request_id": request_id,
        });
        match api::post::<api::Sent>("/api/send", &body).await {
            Ok(sent) if sent.started => {
                let mut write = app.write();
                if let Some(tab) = write.data.get_mut(&key) {
                    tab.draft.clear();
                    tab.pending.clear();
                    tab.running = true;
                }
            }
            Ok(_) => {}
            Err(message) => toast(app, &message),
        }
        refresh(app);
    });
}

fn cancel(app: Signal<App>, key: String) {
    spawn(async move {
        let app = app;
        let body = serde_json::json!({ "conversation": key });
        if let Err(message) = api::post_quiet("/api/cancel", &body).await {
            toast(app, &message);
        }
        refresh(app);
    });
}

fn upload(app: Signal<App>, key: String, file: web_sys::File) {
    spawn(async move {
        let mut app = app;
        let name = match file.name().is_empty() {
            true => "imagen".to_string(),
            false => file.name(),
        };
        let path = format!(
            "/api/upload?conversation={}&name={}",
            urlencode(&key),
            urlencode(&name)
        );
        let request =
            match gloo_net::http::Request::post(&path).body(wasm_bindgen::JsValue::from(file)) {
                Ok(request) => request,
                Err(error) => return toast(app, &error.to_string()),
            };
        match request.send().await {
            Ok(_) => {
                let mut write = app.write();
                if let Some(tab) = write.data.get_mut(&key) {
                    tab.pending.push(name);
                }
            }
            Err(error) => toast(app, &error.to_string()),
        }
    });
}

fn urlencode(text: &str) -> String {
    js_sys::encode_uri_component(text).into()
}

fn new_project(app: Signal<App>, name: String) {
    spawn(async move {
        let mut app = app;
        let body = serde_json::json!({ "name": name });
        match api::post_quiet("/api/projects", &body).await {
            Ok(()) => {
                app.write().new_project.clear();
                refresh(app);
            }
            Err(message) => toast(app, &message),
        }
    });
}

fn new_conversation(app: Signal<App>, sources: Sources, project: String) {
    spawn(async move {
        let app = app;
        let body = serde_json::json!({ "project": project });
        match api::post::<api::Created>("/api/conversations", &body).await {
            Ok(made) => {
                refresh(app);
                open_tab(app, sources, made.key);
            }
            Err(message) => toast(app, &message),
        }
    });
}

fn open_files_tab(mut app: Signal<App>, project: String) {
    let id = format!("files:{project}");
    {
        let mut state = app.write();
        if !state.tabs.contains(&id) {
            state.tabs.push(id.clone());
        }
        state
            .files
            .entry(id.clone())
            .or_insert_with(|| FilesTab::new(project.clone()));
        state.active = Some(id.clone());
    }
    load_dir(app, id, String::new());
}

fn load_dir(app: Signal<App>, id: String, path: String) {
    let project = match app.read().files.get(&id) {
        Some(tab) => tab.project.clone(),
        None => return,
    };
    spawn(async move {
        let mut app = app;
        let answer = api::get::<api::Tree>(&api::tree_path(&project, &path)).await;
        let mut failure = None;
        {
            let mut state = app.write();
            if let Some(tab) = state.files.get_mut(&id) {
                match answer {
                    Ok(tree) => match path.is_empty() {
                        true => tab.root = tree.entries,
                        false => {
                            tab.dirs.insert(path.clone(), tree.entries);
                            if !tab.open.contains(&path) {
                                tab.open.push(path);
                            }
                        }
                    },
                    Err(message) => failure = Some(message),
                }
            }
        }
        if let Some(message) = failure {
            toast(app, &message);
        }
    });
}

fn open_file(mut app: Signal<App>, id: String, path: String, kind: String) {
    let project = match app.read().files.get(&id) {
        Some(tab) => tab.project.clone(),
        None => return,
    };
    {
        let mut state = app.write();
        if let Some(tab) = state.files.get_mut(&id) {
            tab.file = Some(path.clone());
            tab.showing = true;
            tab.content = Content::Note("Trayendo…".to_string());
        }
    }
    spawn(async move {
        let mut app = app;
        let answer = api::raw(&project, &path).await;
        let mut failure = None;
        {
            let mut state = app.write();
            if let Some(tab) = state.files.get_mut(&id) {
                if tab.file.as_deref() != Some(path.as_str()) {
                    return;
                }
                match answer {
                    Ok(raw) => {
                        tab.content = match raw.content_type.starts_with("image/") {
                            true => Content::Image(api::raw_path(&project, &path)),
                            false => {
                                let text = String::from_utf8_lossy(&raw.bytes).to_string();
                                match kind.as_str() {
                                    "markdown" => Content::Markdown(text),
                                    _ => Content::Text(text),
                                }
                            }
                        };
                        if raw.truncated {
                            if let Content::Text(text) = &mut tab.content {
                                text.push_str(
                                    "\n\nSe ve sólo el principio: el archivo pasa los 512 KB",
                                );
                            }
                        }
                    }
                    Err(message) => {
                        tab.content = Content::Note("No pude abrir ese archivo".to_string());
                        failure = Some(message);
                    }
                }
            }
        }
        if let Some(message) = failure {
            toast(app, &message);
        }
    });
}

fn flatten(tab: &FilesTab) -> Vec<(usize, String, api::Entry)> {
    let mut rows = Vec::new();
    walk(tab, "", &tab.root, 0, &mut rows);
    rows
}

fn walk(
    tab: &FilesTab,
    path: &str,
    entries: &[api::Entry],
    depth: usize,
    rows: &mut Vec<(usize, String, api::Entry)>,
) {
    for entry in entries {
        let child = uicore::files::join(path, &entry.name);
        rows.push((depth, child.clone(), entry.clone()));
        if entry.dir && tab.open.contains(&child) {
            if let Some(kids) = tab.dirs.get(&child) {
                walk(tab, &child, kids, depth + 1, rows);
            }
        }
    }
}

fn events_hook(app: Signal<App>) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let listener =
        Closure::<dyn FnMut(web_sys::MouseEvent)>::new(move |event: web_sys::MouseEvent| {
            let Some(target) = event
                .target()
                .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
            else {
                return;
            };
            let Ok(Some(button)) = target.closest("button.copy") else {
                return;
            };
            let Some(text) = button.get_attribute("data-copy") else {
                return;
            };
            let Some(window) = web_sys::window() else {
                return;
            };
            let done = button.clone();
            let promise = window.navigator().clipboard().write_text(&text);
            spawn(async move {
                if wasm_bindgen_futures::JsFuture::from(promise).await.is_err() {
                    return;
                }
                let _ = done.class_list().add_1("copied");
                gloo_timers::future::TimeoutFuture::new(1500).await;
                let _ = done.class_list().remove_1("copied");
            });
        });
    let _ = document.add_event_listener_with_callback("click", listener.as_ref().unchecked_ref());
    listener.forget();

    let paste = Closure::<dyn FnMut(web_sys::ClipboardEvent)>::new(
        move |event: web_sys::ClipboardEvent| {
            let Some(data) = event.clipboard_data() else {
                return;
            };
            let Some(files) = data.files() else {
                return;
            };
            if files.length() == 0 {
                return;
            }
            let Some(key) = app
                .read()
                .active
                .clone()
                .filter(|key| !uicore::files::is_files(key))
            else {
                return;
            };
            event.prevent_default();
            for index in 0..files.length() {
                if let Some(file) = files.get(index) {
                    upload(app, key.clone(), file);
                }
            }
        },
    );
    let _ = document.add_event_listener_with_callback("paste", paste.as_ref().unchecked_ref());
    paste.forget();
}

fn set_theme(theme: &str) {
    if let Some(document) = web_sys::window().and_then(|window| window.document()) {
        if let Some(root) = document.document_element() {
            let _ = root.set_attribute("data-theme", theme);
        }
        if let Some(storage) =
            web_sys::window().and_then(|window| window.local_storage().ok().flatten())
        {
            let _ = storage.set_item("jimmy-theme", theme);
        }
    }
}

fn set_home(home: bool) {
    if let Some(body) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.body())
    {
        body.set_class_name(if home { "home" } else { "" });
    }
}

fn system_theme() -> String {
    let light = web_sys::window()
        .and_then(|window| {
            window
                .match_media("(prefers-color-scheme: light)")
                .ok()
                .flatten()
        })
        .map(|query| query.matches())
        .unwrap_or(false);
    match light {
        true => "light".to_string(),
        false => "dark".to_string(),
    }
}

#[component]
fn app() -> Element {
    let mut app = use_signal(App::default);
    let sources =
        use_hook(|| Rc::new(RefCell::new(HashMap::<String, web_sys::EventSource>::new())));

    use_hook(move || events_hook(app));

    use_hook(move || {
        let theme = stored("jimmy-theme").unwrap_or_else(system_theme);
        {
            let mut state = app.write();
            state.theme = theme.clone();
            state.opened = stored("jimmy-open-projects")
                .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
                .unwrap_or_default();
            state.share = take_share();
        }
        set_theme(&theme);
        refresh(app);
    });

    let sources_restore = sources.clone();
    use_hook(move || {
        restore_tabs(app, sources_restore);
    });

    use_effect(move || {
        let active = app.read().active.clone();
        if let Some(key) = active.filter(|key| !uicore::files::is_files(key)) {
            scroll_bottom(&key);
        }
    });

    use_effect(move || {
        let count = {
            let state = app.read();
            state
                .tabs
                .iter()
                .filter(|key| state.active.as_deref() != Some(key.as_str()))
                .filter(|key| {
                    state
                        .data
                        .get(*key)
                        .map(|tab| tab.done || tab.error)
                        .unwrap_or(false)
                })
                .count()
        };
        if let Some(document) = web_sys::window().and_then(|window| window.document()) {
            let title = match count {
                0 => "Jimmy".to_string(),
                count => format!("({count}) Jimmy"),
            };
            document.set_title(&title);
        }
    });

    use_effect(move || {
        let home = app.read().active.is_none();
        set_home(home);
    });

    use_hook(move || {
        let signal = app;
        spawn(async move {
            loop {
                gloo_timers::future::TimeoutFuture::new(15_000).await;
                refresh(signal);
            }
        });
    });

    use_hook(move || {
        let signal = app;
        load_agenda(signal);
        spawn(async move {
            loop {
                gloo_timers::future::TimeoutFuture::new(60_000).await;
                load_agenda(signal);
            }
        });
    });

    let state = app();
    let active = state.active.clone();
    let workspace = state
        .state
        .as_ref()
        .map(|state| state.workspace.clone())
        .unwrap_or_default();
    let running = active
        .as_ref()
        .and_then(|key| state.data.get(key))
        .map(|tab| tab.running)
        .unwrap_or(false);
    let showing_actions = active.is_some()
        && !active
            .as_deref()
            .map(uicore::files::is_files)
            .unwrap_or(false);

    let open = {
        let app = app;
        let sources = sources.clone();
        move |key: String| open_tab(app, sources.clone(), key)
    };

    rsx! {
        div { id: "app",
            aside { id: "sidebar",
                form {
                    id: "new-project",
                    onsubmit: move |event: FormEvent| {
                        event.prevent_default();
                        let name = app.read().new_project.trim().to_string();
                        if !name.is_empty() {
                            new_project(app, name);
                        }
                    },
                    input {
                        id: "project-name",
                        placeholder: "Nuevo proyecto",
                        "aria-label": "Nuevo proyecto",
                        spellcheck: "false",
                        value: "{state.new_project}",
                        oninput: move |event| app.write().new_project = event.value(),
                    }
                    button { "aria-label": "Agregar proyecto", title: "Agregar proyecto",
                        Icon { name: "plus", size: 14 }
                    }
                }
                div { id: "online", "{state.online}" }
                nav { id: "projects",
                    for project in state.state.as_ref().map(|state| state.projects.clone()).unwrap_or_default() {
                        {
                            let collapsed = !state.opened.contains(&project.name);
                            let selected_here = project
                                .conversations
                                .iter()
                                .any(|conversation| active.as_deref() == Some(conversation.key.as_str()));
                            let group_class = match (collapsed, selected_here) {
                                (true, true) => "group collapsed active",
                                (true, false) => "group collapsed",
                                (false, true) => "group active",
                                (false, false) => "group",
                            };
                            let (body, eyes) = uicore::project::face(&project.name);
                            let hue = uicore::project::hue(&project.name);
                            let avatar = avatar_state(&state, &project, active.as_deref());
                            let more = project.conversations.len() > VISIBLE;
                            let all = state.expanded.contains(&project.name);
                            let shown = match all {
                                true => project.conversations.clone(),
                                false => project.conversations.iter().take(VISIBLE).cloned().collect(),
                            };
                            let caret = match collapsed {
                                true => "Mostrar los hilos",
                                false => "Ocultar los hilos",
                            };
                            let name = project.name.clone();
                            rsx! {
                                div { key: "{project.name}", class: "{group_class}",
                                    div { class: "head",
                                        div {
                                            class: "project",
                                            onclick: {
                                                let name = name.clone();
                                                let sources = sources.clone();
                                                move |_| {
                                                    let newest = app
                                                        .read()
                                                        .state
                                                        .as_ref()
                                                        .and_then(|state| state.projects.iter().find(|project| project.name == name))
                                                        .and_then(|project| project.conversations.first().map(|conversation| conversation.key.clone()));
                                                    match newest {
                                                        Some(key) => {
                                                            let known = app.read().data.contains_key(&key);
                                                            match known {
                                                                true => activate(app, sources.clone(), &key),
                                                                false => open_tab(app, sources.clone(), key),
                                                            }
                                                        }
                                                        None => new_conversation(app, sources.clone(), name.clone()),
                                                    }
                                                }
                                            },
                                            span {
                                                class: "avatar bot body-{body} eyes-{eyes} {avatar}",
                                                style: "--h: {hue}",
                                                span { class: "eye", i {} }
                                                span { class: "eye", i {} }
                                            }
                                            span { class: "text",
                                                span { class: "name", title: "{project.path}",
                                                    "{uicore::project::title(&project.name)}"
                                                }
                                            }
                                            if project.size > 0 {
                                                span { class: "size", "{uicore::project::size(project.size)}" }
                                            }
                                            button {
                                                class: "icon-btn caret",
                                                title: "{caret}",
                                                "aria-label": "{caret}",
                                                onclick: {
                                                    let name = name.clone();
                                                    move |event: MouseEvent| {
                                                        event.stop_propagation();
                                                        let mut state = app.write();
                                                        toggle(&mut state.opened, &name);
                                                        let opened = state.opened.clone();
                                                        drop(state);
                                                        remember_open(&opened);
                                                    }
                                                },
                                                Icon { name: "down", size: 15 }
                                            }
                                            button {
                                                class: "icon-btn kebab",
                                                title: "Opciones del Proyecto",
                                                "aria-label": "Opciones del Proyecto",
                                                onclick: {
                                                    let name = name.clone();
                                                    move |event: MouseEvent| {
                                                        event.stop_propagation();
                                                        let mut state = app.write();
                                                        state.menu = match state.menu.as_deref() {
                                                            Some(current) if current == name => None,
                                                            _ => Some(name.clone()),
                                                        };
                                                    }
                                                },
                                                Icon { name: "dots", size: 15 }
                                            }
                                        }
                                        div { class: "menu", hidden: state.menu.as_deref() != Some(project.name.as_str()),
                                            button {
                                                class: "item",
                                                onclick: {
                                                    let name = name.clone();
                                                    let sources = sources.clone();
                                                    move |_| {
                                                        app.write().menu = None;
                                                        new_conversation(app, sources.clone(), name.clone());
                                                    }
                                                },
                                                "Nueva conversación"
                                            }
                                            button {
                                                class: "item",
                                                onclick: {
                                                    let name = name.clone();
                                                    move |_| {
                                                        app.write().menu = None;
                                                        open_files_tab(app, name.clone());
                                                    }
                                                },
                                                "Ver archivos"
                                            }
                                            if project.name != "general" {
                                                button {
                                                    class: "item",
                                                    onclick: {
                                                        let name = name.clone();
                                                        move |_| {
                                                            app.write().menu = None;
                                                            rename_project(app, name.clone());
                                                        }
                                                    },
                                                    "Renombrar proyecto"
                                                }
                                                button {
                                                    class: "item",
                                                    onclick: {
                                                        let name = name.clone();
                                                        move |_| {
                                                            app.write().menu = None;
                                                            duplicate_project(app, name.clone());
                                                        }
                                                    },
                                                    "Duplicar proyecto"
                                                }
                                                button {
                                                    class: "item danger",
                                                    onclick: {
                                                        let project = project.clone();
                                                        move |_| {
                                                            app.write().menu = None;
                                                            remove_project(app, project.clone());
                                                        }
                                                    },
                                                    "Quitar proyecto"
                                                }
                                            }
                                        }
                                    }
                                    div { class: "threads",
                                        for conversation in shown {
                                            {
                                                let selected = active.as_deref() == Some(conversation.key.as_str());
                                                let tab = state.data.get(&conversation.key);
                                                let status = match tab {
                                                    Some(tab) if tab.error => "dot error",
                                                    Some(tab) if tab.done => "dot done",
                                                    _ if conversation.running => "dot running",
                                                    _ => "dot",
                                                };
                                                rsx! {
                                                    div {
                                                        key: "{conversation.key}",
                                                        class: if selected { "conversation active" } else { "conversation" },
                                                        onclick: {
                                                            let key = conversation.key.clone();
                                                            let open = open.clone();
                                                            let sources = sources.clone();
                                                            move |_| {
                                                                let known = app.read().data.contains_key(&key);
                                                                match known {
                                                                    true => activate(app, sources.clone(), &key),
                                                                    false => open(key.clone()),
                                                                }
                                                            }
                                                        },
                                                        span { class: "slot",
                                                            span { class: "{status}" }
                                                        }
                                                        span { class: "title",
                                                            if conversation.title.is_empty() {
                                                                "{conversation.key}"
                                                            } else {
                                                                "{conversation.title}"
                                                            }
                                                        }
                                                        if !conversation.read_only {
                                                            button {
                                                                class: "icon-btn",
                                                                title: "Renombrar",
                                                                "aria-label": "Renombrar",
                                                                onclick: {
                                                                    let key = conversation.key.clone();
                                                                    move |event: MouseEvent| {
                                                                        event.stop_propagation();
                                                                        rename_conversation(app, key.clone());
                                                                    }
                                                                },
                                                                Icon { name: "pencil", size: 14 }
                                                            }
                                                            button {
                                                                class: "icon-btn danger",
                                                                title: "Borrar conversación",
                                                                "aria-label": "Borrar conversación",
                                                                onclick: {
                                                                    let key = conversation.key.clone();
                                                                    let sources = sources.clone();
                                                                    move |event: MouseEvent| {
                                                                        event.stop_propagation();
                                                                        remove_conversation(app, sources.clone(), key.clone());
                                                                    }
                                                                },
                                                                Icon { name: "close", size: 14 }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        if more {
                                            button {
                                                class: "more",
                                                onclick: {
                                                    let name = name.clone();
                                                    move |_| toggle(&mut app.write().expanded, &name)
                                                },
                                                if all { "Ver menos" } else { "Ver más" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                div { id: "chips",
                    {
                        let marks: Vec<uicore::agenda::Mark> = state
                            .agenda
                            .clone()
                            .unwrap_or_default()
                            .iter()
                            .map(|task| mark_of(&state, task))
                            .collect();
                        let unread = uicore::agenda::unread(&marks);
                        let bad = uicore::agenda::failed(&marks) > 0;
                        rsx! {
                            button {
                                id: "vault",
                                "aria-label": "Vault",
                                onclick: move |_| open_vault(app),
                                span { class: "label", "Vault" }
                            }
                            button {
                                id: "agenda",
                                "aria-label": "Agenda",
                                onclick: move |_| open_agenda(app),
                                span { class: "label", "Agenda" }
                                span {
                                    id: "agenda-badge",
                                    hidden: unread == 0,
                                    class: if bad { "error" } else { "" },
                                    "{unread}"
                                }
                            }
                        }
                    }
                }
                div { id: "base",
                    div { id: "machine",
                        if let Some(machine) = state.state.as_ref().map(|state| state.machine.clone()) {
                            span {
                                class: "stat",
                                title: "{uicore::machine::ratio(machine.memory.used, machine.memory.total)} · la memoria del contenedor, cache y kernel incluidos",
                                Icon { name: "cpu", size: 13 }
                                span { class: "value", "{uicore::machine::size(machine.memory.used)}" }
                            }
                            span {
                                class: "stat",
                                title: "{uicore::machine::ratio(machine.disk.used, machine.disk.total)} · el volumen {workspace}",
                                Icon { name: "disk", size: 13 }
                                span { class: "value", "{uicore::machine::percent(machine.disk.used, machine.disk.total)}" }
                            }
                        }
                    }
                    div { id: "footer",
                        button {
                            id: "theme",
                            title: if state.theme == "light" { "Pasar al tema oscuro" } else { "Pasar al tema claro" },
                            "aria-label": "Cambiar el tema",
                            onclick: move |_| {
                                let next = match app.read().theme.as_str() {
                                    "light" => "dark",
                                    _ => "light",
                                };
                                app.write().theme = next.to_string();
                                set_theme(next);
                            },
                            Icon { name: if state.theme == "light" { "moon" } else { "sun" }, size: 18 }
                        }
                        form { id: "logout", method: "post", action: "/api/logout",
                            button { "aria-label": "Salir", title: "Salir",
                                Icon { name: "logout", size: 16 }
                            }
                        }
                    }
                }
            }
            main {
                div { id: "tabbar",
                    button {
                        id: "menu",
                        "aria-label": "Volver",
                        title: "Volver",
                        onclick: move |_| {
                            app.write().active = None;
                            remember_hash(None);
                        },
                        Icon { name: "left", size: 16 }
                    }
                    div { id: "tabs",
                        for key in state.tabs.clone() {
                            {
                                let selected = active.as_deref() == Some(key.as_str());
                                let tab = state.data.get(&key);
                                let status = match tab {
                                    Some(tab) if tab.error => "dot error",
                                    Some(tab) if tab.done => "dot done",
                                    Some(tab) if tab.running => "dot running",
                                    _ => "dot",
                                };
                                let title = title_of(&state, &key);
                                let project = project_of(&state, &key);
                                rsx! {
                                    div { key: "{key}", class: if selected { "tab active" } else { "tab" },
                                        span { class: "pill",
                                            if !project.is_empty() && project != "general" {
                                                "{uicore::project::initials(&project)}"
                                            }
                                        }
                                        span { class: "{status}" }
                                        span { class: "title", "{title}" }
                                        button {
                                            class: "close",
                                            title: "Cerrar pestaña",
                                            "aria-label": "Cerrar pestaña",
                                            onclick: {
                                                let key = key.clone();
                                                let sources = sources.clone();
                                                move |_| close_tab(app, sources.clone(), &key)
                                            },
                                            Icon { name: "close", size: 15 }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    div { id: "tab-actions", hidden: !showing_actions,
                        span { id: "viewers",
                            if let Some(tab) = active.as_ref().and_then(|key| state.data.get(key)) {
                                if tab.viewers.len() > 1 {
                                    "{tab.viewers.join(\", \")} Mirando"
                                }
                            }
                        }
                        button {
                            id: "cancel",
                            hidden: !running,
                            onclick: {
                                let key = active.clone().unwrap_or_default();
                                move |_| cancel(app, key.clone())
                            },
                            "Cancelar"
                        }
                    }
                }
                if let Some(message) = state.error.clone() {
                    div { id: "refresh", "{message}" }
                }
                div { id: "panes",
                    div { id: "placeholder", hidden: active.is_some(), "Elegí una conversación o creá una nueva." }
                    for key in state.tabs.clone() {
                        {
                            let selected = active.as_deref() == Some(key.as_str());
                            if key == "vault" {
                                rsx! {
                                    div { key: "{key}", class: "pane vault", hidden: !selected,
                                        VaultPane { app }
                                    }
                                }
                            } else if key == "agenda" {
                                let tasks = state.agenda.clone().unwrap_or_default();
                                let marks: Vec<uicore::agenda::Mark> =
                                    tasks.iter().map(|task| mark_of(&state, task)).collect();
                                let unread = uicore::agenda::unread(&marks);
                                let open = state.agenda_open.clone();
                                rsx! {
                                    div { key: "{key}", class: "pane agenda", hidden: !selected,
                                        div { class: "agenda-inner",
                                            if tasks.is_empty() {
                                                div { class: "agenda-empty", "No hay tareas agendadas." }
                                            }
                                            for task in tasks {
                                                {
                                                    let mark = mark_of(&state, &task);
                                                    let state_name = uicore::agenda::state(mark);
                                                    let expanded = open.contains(&task.name);
                                                    let when = uicore::agenda::when(
                                                        task.when.as_deref(),
                                                        task.at.as_deref(),
                                                        task.every.as_deref(),
                                                    );
                                                    let last = match task.runs.first() {
                                                        None => "nunca corrió".to_string(),
                                                        Some(run) => format!(
                                                            "{} · {}",
                                                            uicore::agenda::moment(&run.date, &clock_of(run.ts), &today(), &yesterday()),
                                                            uicore::agenda::duration(run.ms)
                                                        ),
                                                    };
                                                    rsx! {
                                                        article {
                                                            key: "{task.name}",
                                                            class: if expanded { "task open" } else { "task" },
                                                            div {
                                                                class: "task-head",
                                                                onclick: {
                                                                    let name = task.name.clone();
                                                                    move |_| {
                                                                        let mut state = app.write();
                                                                        let was_open = state.agenda_open.contains(&name);
                                                                        if was_open {
                                                                            state.agenda_open.retain(|item| item != &name);
                                                                        } else {
                                                                            state.agenda_open.push(name.clone());
                                                                        }
                                                                        drop(state);
                                                                        if !was_open {
                                                                            read_agenda(app, Some(name.clone()));
                                                                        }
                                                                    }
                                                                },
                                                                div { class: "task-title",
                                                                    span { class: if state_name.is_empty() { "dot".to_string() } else { format!("dot {state_name}") } }
                                                                    span { class: "task-name", "{task.name}" }
                                                                    span { class: "task-when", "{when}" }
                                                                    if task.unread > 0 {
                                                                        span { class: "pill unread", "{task.unread}" }
                                                                    }
                                                                    if task.paused {
                                                                        span { class: "pill", "pausada" }
                                                                    }
                                                                    if task.blocked {
                                                                        span { class: "pill blocked", "bloqueada ({task.attempts})" }
                                                                    }
                                                                }
                                                                div { class: "task-meta",
                                                                    span { class: "task-last", "{last}" }
                                                                    if !task.paused {
                                                                        button {
                                                                            class: "icon-btn",
                                                                            title: "Correr ahora",
                                                                            "aria-label": "Correr ahora",
                                                                            onclick: {
                                                                                let name = task.name.clone();
                                                                                move |event: MouseEvent| {
                                                                                    event.stop_propagation();
                                                                                    run_agenda(app, name.clone());
                                                                                }
                                                                            },
                                                                            Icon { name: "play", size: 14 }
                                                                        }
                                                                    }
                                                                    button {
                                                                        class: "icon-btn",
                                                                        title: if task.paused { "Reanudar" } else { "Pausar" },
                                                                        "aria-label": if task.paused { "Reanudar" } else { "Pausar" },
                                                                        onclick: {
                                                                            let name = task.name.clone();
                                                                            let paused = task.paused;
                                                                            move |event: MouseEvent| {
                                                                                event.stop_propagation();
                                                                                pause_agenda(app, name.clone(), !paused);
                                                                            }
                                                                        },
                                                                        Icon { name: if task.paused { "play" } else { "pause" }, size: 14 }
                                                                    }
                                                                }
                                                            }
                                                            if expanded {
                                                                div { class: "task-runs",
                                                                    if task.runs.is_empty() {
                                                                        div { class: "run-empty", "Todavía no corrió." }
                                                                    }
                                                                    for run in task.runs.clone() {
                                                                        div {
                                                                            key: "{run.ts}",
                                                                            class: if run.ok { "run" } else { "run failed" },
                                                                            span {
                                                                                class: "run-time",
                                                                                title: "{ago_of(run.ts)} · {uicore::agenda::duration(run.ms)}",
                                                                                "{uicore::agenda::moment(&run.date, &clock_of(run.ts), &today(), &yesterday())}"
                                                                            }
                                                                            div {
                                                                                class: "run-text",
                                                                                if run.text.trim().is_empty() {
                                                                                    "sin novedades"
                                                                                } else {
                                                                                    div { dangerous_inner_html: uicore::markdown::render(&run.text) }
                                                                                }
                                                                            }
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            if unread > 0 {
                                                div { class: "agenda-foot",
                                                    button {
                                                        onclick: move |_| read_agenda(app, None),
                                                        "Marcar todo leído"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            } else if uicore::files::is_files(&key) {
                                let tab = state
                                    .files
                                    .get(&key)
                                    .cloned()
                                    .unwrap_or_else(|| FilesTab::new(String::new()));
                                let rows = flatten(&tab);
                                let file = tab.file.clone().unwrap_or_default();
                                rsx! {
                                    div {
                                        key: "{key}",
                                        class: if tab.showing { "pane files showing" } else { "pane files" },
                                        hidden: !selected,
                                        div { class: "files-view",
                                            div { class: "files-head", hidden: !tab.showing,
                                                button {
                                                    class: "files-back",
                                                    onclick: {
                                                        let key = key.clone();
                                                        move |_| {
                                                            if let Some(tab) = app.write().files.get_mut(&key) {
                                                                tab.showing = false;
                                                            }
                                                        }
                                                    },
                                                    Icon { name: "left", size: 15 }
                                                    span { "Archivos" }
                                                }
                                                span { class: "files-path", "{file}" }
                                            }
                                            div { class: "files-body",
                                                match tab.content.clone() {
                                                    Content::Empty => rsx! { div { class: "files-note", "Elegí un archivo para leerlo." } },
                                                    Content::Note(text) => rsx! { div { class: "files-note", "{text}" } },
                                                    Content::Image(src) => rsx! { img { class: "files-image", src: "{src}", alt: "{file}" } },
                                                    Content::Markdown(text) => rsx! {
                                                        div { class: "event assistant files-markdown",
                                                            div { dangerous_inner_html: uicore::markdown::render(&text) }
                                                        }
                                                    },
                                                    Content::Text(text) => rsx! {
                                                        div { class: "files-code",
                                                            pre { "{text}" }
                                                            button {
                                                                class: "copy",
                                                                "data-copy": "{text}",
                                                                title: "Copiar el archivo",
                                                                "aria-label": "Copiar el archivo",
                                                                Icon { name: "copy", size: 14 }
                                                            }
                                                        }
                                                    },
                                                }
                                            }
                                        }
                                        div { class: "files-tree",
                                            if rows.is_empty() {
                                                div { class: "files-note", "Vacía" }
                                            }
                                            for (depth, path, entry) in rows {
                                                {
                                                    let mut class = match entry.dir {
                                                        true => "files-row files-folder".to_string(),
                                                        false => "files-row button".to_string(),
                                                    };
                                                    if tab.file.as_deref() == Some(path.as_str()) {
                                                        class.push_str(" active");
                                                    }
                                                    let pad = depth * 12;
                                                    rsx! {
                                                        div {
                                                            key: "{path}",
                                                            class: "{class}",
                                                            style: "padding-left: {pad}px",
                                                            onclick: {
                                                                let path = path.clone();
                                                                let kind = entry.kind.clone();
                                                                let dir = entry.dir;
                                                                let key = key.clone();
                                                                move |_| match dir {
                                                                    true => load_dir(app, key.clone(), path.clone()),
                                                                    false => open_file(app, key.clone(), path.clone(), kind.clone()),
                                                                }
                                                            },
                                                            Icon { name: if entry.dir { "folder" } else { "file" }, size: 15 }
                                                            span { class: "files-label", "{entry.name}" }
                                                            if !entry.dir && entry.size > 0 {
                                                                span { class: "files-size", "{uicore::files::size(entry.size)}" }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            } else {
                                let tab = state.data.get(&key).cloned().unwrap_or_default();
                                rsx! {
                                    div { key: "{key}", class: "pane", hidden: !selected,
                                        Transcript {
                                            id: format!("transcript-{key}"),
                                            conversation: key.clone(),
                                            workspace: workspace.clone(),
                                            events: tab.events.clone(),
                                        }
                                        div { class: "jump",
                                            if tab.first > 0 {
                                                button {
                                                    class: "up",
                                                    "aria-label": "Ver anteriores",
                                                    onclick: {
                                                        let key = key.clone();
                                                        move |_| load_earlier(app, key.clone())
                                                    },
                                                    Icon { name: "up", size: 15 }
                                                }
                                            }
                                            button {
                                                class: "down",
                                                "aria-label": "Ir al final",
                                                onclick: {
                                                    let key = key.clone();
                                                    move |_| scroll_bottom(&key)
                                                },
                                                Icon { name: "arrow-down", size: 15 }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                if let Some(key) = active
                    .clone()
                    .filter(|key| !uicore::files::is_files(key) && key != "agenda" && key != "vault")
                {
                    form {
                        id: "composer",
                        onsubmit: {
                            let key = key.clone();
                            move |event: FormEvent| {
                                event.prevent_default();
                                send(app, key.clone());
                            }
                        },
                        div { id: "typing",
                            hidden: state.data.get(&key).map(|tab| tab.typing.is_empty()).unwrap_or(true),
                            if let Some(tab) = state.data.get(&key) {
                                "{tab.typing} está escribiendo…"
                            }
                        }
                        div { id: "pending",
                            hidden: state.data.get(&key).map(|tab| tab.pending.is_empty()).unwrap_or(true),
                            if let Some(tab) = state.data.get(&key) {
                                for name in tab.pending.clone() {
                                    div { key: "{name}", class: "thumb",
                                        img { src: api::file_path(&key, &name) }
                                        button {
                                            r#type: "button",
                                            "aria-label": "Quitar adjunto",
                                            onclick: {
                                                let name = name.clone();
                                                let conversation = key.clone();
                                                move |_| {
                                                    if let Some(tab) = app.write().data.get_mut(&conversation) {
                                                        tab.pending.retain(|item| item != &name);
                                                    }
                                                }
                                            },
                                            Icon { name: "close", size: 12 }
                                        }
                                    }
                                }
                            }
                        }
                        div { class: "box",
                            textarea {
                                id: "input",
                                rows: "1",
                                placeholder: "Escribí un mensaje…",
                                value: state.data.get(&key).map(|tab| tab.draft.clone()).unwrap_or_default(),
                                oninput: {
                                    let conversation = key.clone();
                                    move |event: FormEvent| {
                                        if let Some(tab) = app.write().data.get_mut(&conversation) {
                                            tab.draft = event.value();
                                        }
                                    }
                                },
                                onkeydown: {
                                    let conversation = key.clone();
                                    move |event: KeyboardEvent| {
                                        if event.key().to_string() != "Enter" || event.modifiers().shift() {
                                            return;
                                        }
                                        event.prevent_default();
                                        send(app, conversation.clone());
                                    }
                                },
                            }
                            label { id: "attach", title: "Adjuntar imagen",
                                input {
                                    id: "file",
                                    r#type: "file",
                                    accept: "image/*",
                                    multiple: true,
                                    hidden: true,
                                    onchange: {
                                        let conversation = key.clone();
                                        move |_| {
                                            let Some(document) = web_sys::window().and_then(|window| window.document()) else {
                                                return;
                                            };
                                            let Some(input) = document
                                                .get_element_by_id("file")
                                                .and_then(|element| element.dyn_into::<web_sys::HtmlInputElement>().ok())
                                            else {
                                                return;
                                            };
                                            let Some(files) = input.files() else {
                                                return;
                                            };
                                            for index in 0..files.length() {
                                                if let Some(file) = files.get(index) {
                                                    upload(app, conversation.clone(), file);
                                                }
                                            }
                                            input.set_value("");
                                        }
                                    }
                                }
                            }
                            button {
                                class: if running { "send stop" } else { "send" },
                                "aria-label": if running { "Frenar el turno" } else { "Enviar" },
                                onclick: {
                                    let conversation = key.clone();
                                    move |event: MouseEvent| {
                                        if running {
                                            event.prevent_default();
                                            cancel(app, conversation.clone());
                                        }
                                    }
                                },
                                span { class: "send-icon", Icon { name: "up", size: 17 } }
                                span { class: "stop-icon", Icon { name: "stop", size: 15 } }
                            }
                        }
                    }
                }
            }
        }
        div { id: "toast", hidden: state.toast.is_none(),
            if let Some(text) = state.toast.clone() {
                "{text}"
            }
        }
    }
}
