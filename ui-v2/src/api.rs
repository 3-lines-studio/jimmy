use gloo_net::http::Request;
use serde::Deserialize;

#[derive(Deserialize, Clone, PartialEq)]
pub struct Conversation {
    pub key: String,
    pub project: String,
    pub title: String,
    pub read_only: bool,
    pub running: bool,
}

#[derive(Deserialize, Clone, PartialEq)]
pub struct Project {
    pub name: String,
    pub path: String,
    pub unversioned: bool,
    pub size: u64,
    pub conversations: Vec<Conversation>,
}

#[derive(Deserialize, Clone, PartialEq)]
pub struct Amount {
    pub used: u64,
    pub total: u64,
}

#[derive(Deserialize, Clone, PartialEq)]
pub struct Machine {
    pub memory: Amount,
    pub disk: Amount,
}

#[derive(Deserialize, Clone, PartialEq)]
pub struct Preview {
    pub name: String,
    pub port: u16,
    pub seconds: u64,
    pub path: String,
}

#[derive(Deserialize, Clone, PartialEq)]
pub struct State {
    pub user: String,
    pub workspace: String,
    pub machine: Machine,
    #[serde(default)]
    pub previews: Vec<Preview>,
    pub projects: Vec<Project>,
}

#[derive(Deserialize)]
pub struct History {
    pub events: Vec<serde_json::Value>,
    pub first: usize,
}

#[derive(Deserialize)]
pub struct Created {
    pub key: String,
}

#[derive(Deserialize)]
pub struct Sent {
    pub started: bool,
}

async fn check(answer: gloo_net::http::Response) -> Result<gloo_net::http::Response, String> {
    match answer.status() {
        200 => Ok(answer),
        401 => Err("no estás adentro".to_string()),
        status => Err(format!("estado {status}")),
    }
}

pub async fn get<T: for<'de> Deserialize<'de>>(path: &str) -> Result<T, String> {
    let answer = Request::get(path)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    check(answer)
        .await?
        .json()
        .await
        .map_err(|error| error.to_string())
}

pub async fn post<T: for<'de> Deserialize<'de>>(
    path: &str,
    body: &serde_json::Value,
) -> Result<T, String> {
    let answer = Request::post(path)
        .json(body)
        .map_err(|error| error.to_string())?
        .send()
        .await
        .map_err(|error| error.to_string())?;
    check(answer)
        .await?
        .json()
        .await
        .map_err(|error| error.to_string())
}

pub async fn post_quiet(path: &str, body: &serde_json::Value) -> Result<(), String> {
    let answer = Request::post(path)
        .json(body)
        .map_err(|error| error.to_string())?
        .send()
        .await
        .map_err(|error| error.to_string())?;
    check(answer).await.map(|_| ())
}

pub fn state_path() -> &'static str {
    uicore::api::state_path()
}

pub fn history_path(key: &str, before: usize) -> String {
    uicore::api::history_path(key, before)
}

pub fn stream_path(key: &str, since: usize) -> String {
    uicore::api::stream_path(key, since)
}

pub fn file_path(conversation: &str, name: &str) -> String {
    uicore::api::file_path(conversation, name)
}

#[derive(Deserialize, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
    pub size: u64,
    pub kind: String,
}

#[derive(Deserialize)]
pub struct Tree {
    pub entries: Vec<Entry>,
}

pub fn tree_path(project: &str, path: &str) -> String {
    uicore::api::tree_path(project, path)
}

pub fn raw_path(project: &str, path: &str) -> String {
    uicore::api::raw_path(project, path)
}

pub struct Raw {
    pub content_type: String,
    pub truncated: bool,
    pub bytes: Vec<u8>,
}

pub async fn raw(project: &str, path: &str) -> Result<Raw, String> {
    let answer = Request::get(&raw_path(project, path))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let answer = check(answer).await?;
    let headers = answer.headers();
    let content_type = headers.get("content-type").unwrap_or_default();
    let truncated = headers.get("x-truncated").is_some();
    let bytes = answer.binary().await.map_err(|error| error.to_string())?;
    Ok(Raw {
        content_type,
        truncated,
        bytes,
    })
}

#[derive(Deserialize, Clone, PartialEq)]
pub struct Run {
    pub ts: i64,
    pub date: String,
    pub ms: u64,
    pub ok: bool,
    pub text: String,
}

#[derive(Deserialize, Clone, PartialEq)]
pub struct Task {
    pub name: String,
    #[serde(default)]
    pub when: Option<String>,
    #[serde(default)]
    pub at: Option<String>,
    #[serde(default)]
    pub every: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub unread: u64,
    #[serde(default)]
    pub blocked: bool,
    #[serde(default)]
    pub attempts: u64,
    #[serde(default)]
    pub runs: Vec<Run>,
}

#[derive(Deserialize)]
pub struct Agenda {
    pub tasks: Vec<Task>,
}

#[derive(Deserialize, Clone, PartialEq)]
pub struct VaultEnvironment {
    pub project: String,
    pub env: String,
    pub keys: u64,
}

#[derive(Deserialize, Clone, PartialEq)]
pub struct VaultToken {
    pub id: String,
    pub name: String,
    pub project: String,
    pub env: String,
    #[serde(default)]
    pub admin: bool,
    #[serde(default)]
    pub keys: Option<Vec<String>>,
    #[serde(default)]
    pub last_used: Option<i64>,
}

#[derive(Deserialize, Clone, PartialEq)]
pub struct Audit {
    pub at: i64,
    pub actor: String,
    pub action: String,
    pub project: String,
    pub env: String,
    #[serde(default)]
    pub key: Option<String>,
}

#[derive(Deserialize, Clone, PartialEq)]
pub struct VaultPanel {
    pub enabled: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub environments: Vec<VaultEnvironment>,
    #[serde(default)]
    pub tokens: Vec<VaultToken>,
    #[serde(default)]
    pub audit: Vec<Audit>,
}

#[derive(Deserialize)]
pub struct VaultSecrets {
    pub secrets: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub plain: Vec<String>,
}

#[derive(Deserialize)]
pub struct Minted {
    #[serde(default)]
    pub plain: String,
}
