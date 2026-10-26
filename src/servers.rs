//! Los servidores MCP conectados: qué URL, con qué cuenta y qué tokens. La
//! lista vive en `state/mcp.json`, al lado de la agenda, y los secretos en el
//! vault, un `env` por cuenta.
//!
//! Conectar uno arranca en `/mcp/start`, pasa por el proveedor y vuelve a
//! `/oauth/callback`; a partir de ahí el token sale del vault y `tools/call`
//! viaja por `src/mcp.rs`.

use crate::oauth;
use heimdall::store::Store;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const ACTOR: &str = "mcp";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Server {
    pub name: String,
    pub url: String,
    pub account: String,
    pub resource: String,
    pub issuer: String,
    pub authorize: String,
    pub token: String,
    pub register: Option<String>,
    pub client_id: String,
    pub scopes: Vec<String>,
}

pub struct Connected {
    pub registration: oauth::Registration,
    pub metadata: oauth::Metadata,
}

pub fn file(root: &Path) -> PathBuf {
    root.join("state/mcp.json")
}

pub fn list(root: &Path) -> Vec<Server> {
    let Ok(text) = std::fs::read_to_string(file(root)) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn find(root: &Path, name: &str) -> Option<Server> {
    list(root).into_iter().find(|server| server.name == name)
}

pub fn save(root: &Path, server: &Server) -> Result<(), String> {
    let mut servers = list(root);
    servers.retain(|entry| entry.name != server.name);
    servers.push(server.clone());
    servers.sort_by(|left, right| left.name.cmp(&right.name));
    write(root, &servers)
}

pub fn write(root: &Path, servers: &[Server]) -> Result<(), String> {
    let dir = root.join("state");
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let body = serde_json::to_string_pretty(servers).map_err(|error| error.to_string())?;
    std::fs::write(file(root), format!("{body}\n")).map_err(|error| error.to_string())
}

/// Deja el cliente registrado en el vault, con la cuenta elegida.
pub fn remember(
    store: &Store,
    server: &str,
    account: &str,
    registration: &oauth::Registration,
) -> Result<(), String> {
    store
        .create_environment(server, account, ACTOR)
        .map_err(crate::vault::text)?;
    store
        .set(
            server,
            account,
            "OAUTH_CLIENT_ID",
            &registration.client_id,
            ACTOR,
        )
        .map_err(crate::vault::text)?;
    if let Some(secret) = &registration.client_secret {
        store
            .set(server, account, "OAUTH_CLIENT_SECRET", secret, ACTOR)
            .map_err(crate::vault::text)?;
    }
    Ok(())
}

pub fn save_tokens(
    store: &Store,
    server: &str,
    account: &str,
    tokens: &oauth::Tokens,
) -> Result<(), String> {
    let mut pairs = vec![
        ("OAUTH_ACCESS_TOKEN", tokens.access.clone()),
        ("OAUTH_EXPIRES_AT", tokens.expires_at.to_string()),
    ];
    if let Some(refresh) = &tokens.refresh {
        pairs.push(("OAUTH_REFRESH_TOKEN", refresh.clone()));
    }
    for (name, value) in pairs {
        store
            .set(server, account, name, &value, ACTOR)
            .map_err(crate::vault::text)?;
    }
    Ok(())
}

/// Reconstruye el servidor guardado a partir de la config y el vault.
pub fn connected(store: &Store, server: &Server) -> Result<Connected, String> {
    let secrets = store
        .secrets(&server.name, &server.account)
        .map_err(crate::vault::text)?;
    let client_id = secrets
        .get("OAUTH_CLIENT_ID")
        .ok_or_else(|| format!("{} no tiene cliente registrado", server.name))?
        .clone();
    let registration = oauth::Registration {
        client_id,
        client_secret: secrets.get("OAUTH_CLIENT_SECRET").cloned(),
    };
    let metadata = oauth::Metadata {
        issuer: server.issuer.clone(),
        authorization_endpoint: server.authorize.clone(),
        token_endpoint: server.token.clone(),
        registration_endpoint: server.register.clone(),
        methods: Vec::new(),
    };
    Ok(Connected {
        registration,
        metadata,
    })
}

/// El access token vivo, refrescándolo si ya venció.
pub fn access_token(store: &Store, server: &Server) -> Result<String, String> {
    let connected = connected(store, server)?;
    let secrets = store
        .secrets(&server.name, &server.account)
        .map_err(crate::vault::text)?;
    let expires_at = secrets
        .get("OAUTH_EXPIRES_AT")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    let access = secrets
        .get("OAUTH_ACCESS_TOKEN")
        .cloned()
        .unwrap_or_default();
    if !access.is_empty() && (expires_at == 0 || expires_at > oauth::now() + 30) {
        return Ok(access);
    }
    let Some(refresh) = secrets.get("OAUTH_REFRESH_TOKEN") else {
        return Ok(access);
    };
    let tokens = oauth::refresh(
        &connected.metadata,
        &connected.registration,
        refresh,
        &server.resource,
    )?;
    save_tokens(store, &server.name, &server.account, &tokens)?;
    Ok(tokens.access)
}

#[cfg(test)]
mod tests {
    use super::*;
    use heimdall::crypto::Key;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jimmy-mcp-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn store(dir: &Path) -> Store {
        Store::open(
            dir.join("heimdall.db"),
            Key::from_hex(&"cd".repeat(32)).unwrap(),
        )
        .unwrap()
    }

    fn example() -> Server {
        Server {
            name: "notion".into(),
            url: "https://mcp.notion.com/mcp".into(),
            account: "manu".into(),
            resource: "https://mcp.notion.com/mcp".into(),
            issuer: "https://mcp.notion.com".into(),
            authorize: "https://mcp.notion.com/authorize".into(),
            token: "https://mcp.notion.com/token".into(),
            register: Some("https://mcp.notion.com/register".into()),
            client_id: "abc".into(),
            scopes: vec!["default".into()],
        }
    }

    #[test]
    fn a_server_is_saved_and_read_back() {
        let dir = temp("roundtrip");
        save(&dir, &example()).unwrap();
        let servers = list(&dir);
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "notion");
        assert_eq!(find(&dir, "notion").unwrap().account, "manu");
        assert!(find(&dir, "otro").is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn saving_the_same_name_replaces_and_keeps_the_order() {
        let dir = temp("replace");
        save(&dir, &example()).unwrap();
        let mut zeta = example();
        zeta.name = "zeta".into();
        save(&dir, &zeta).unwrap();
        let mut notion = example();
        notion.account = "trabajo".into();
        save(&dir, &notion).unwrap();

        let names: Vec<String> = list(&dir).into_iter().map(|s| s.name).collect();
        assert_eq!(names, vec!["notion", "zeta"]);
        assert_eq!(find(&dir, "notion").unwrap().account, "trabajo");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_client_lands_in_the_vault_under_the_account() {
        let dir = temp("vault");
        let store = store(&dir);
        let registration = oauth::Registration {
            client_id: "abc".into(),
            client_secret: Some("shh".into()),
        };

        remember(&store, "notion", "manu", &registration).unwrap();

        let secrets = store.secrets("notion", "manu").unwrap();
        assert_eq!(secrets["OAUTH_CLIENT_ID"], "abc");
        assert_eq!(secrets["OAUTH_CLIENT_SECRET"], "shh");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_stored_server_rebuilds_the_registration() {
        let dir = temp("connected");
        let store = store(&dir);
        remember(
            &store,
            "notion",
            "manu",
            &oauth::Registration {
                client_id: "abc".into(),
                client_secret: None,
            },
        )
        .unwrap();

        let connected = connected(&store, &example()).unwrap();
        assert_eq!(connected.registration.client_id, "abc");
        assert!(connected.registration.client_secret.is_none());
        assert_eq!(
            connected.metadata.token_endpoint,
            "https://mcp.notion.com/token"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_server_without_a_registered_client_is_an_error() {
        let dir = temp("unregistered");
        let store = store(&dir);
        assert!(connected(&store, &example()).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_token_that_is_not_expired_is_used_as_is() {
        let dir = temp("fresh");
        let store = store(&dir);
        remember(
            &store,
            "notion",
            "manu",
            &oauth::Registration {
                client_id: "abc".into(),
                client_secret: None,
            },
        )
        .unwrap();
        store
            .set("notion", "manu", "OAUTH_ACCESS_TOKEN", "vivo", ACTOR)
            .unwrap();
        store
            .set(
                "notion",
                "manu",
                "OAUTH_EXPIRES_AT",
                &(oauth::now() + 3600).to_string(),
                ACTOR,
            )
            .unwrap();

        assert_eq!(access_token(&store, &example()).unwrap(), "vivo");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
