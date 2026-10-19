use crate::api;
use crate::icons::Icon;
use crate::{remember_hash, remember_tabs, toast, App};
use dioxus::prelude::*;
use uicore::vault;

#[derive(Clone, PartialEq, Default)]
pub struct Vault {
    pub panel: Option<api::VaultPanel>,
    pub open: Option<String>,
    pub secrets: Option<Vec<(String, String)>>,
    pub plain: Vec<String>,
    pub revealed: Vec<String>,
    pub editing: Option<String>,
    pub minted: Option<String>,
    pub env_project: String,
    pub env_name: String,
    pub key_name: String,
    pub key_value: String,
    pub key_secret: bool,
    pub token_name: String,
    pub token_project: String,
    pub token_env: String,
    pub token_keys: String,
    pub edit_value: String,
    pub edit_secret: bool,
}

pub fn open_vault(mut app: Signal<App>) {
    {
        let mut state = app.write();
        if !state.tabs.contains(&"vault".to_string()) {
            state.tabs.push("vault".to_string());
        }
        state.active = Some("vault".to_string());
        state.vault.key_secret = true;
    }
    load(app);
    remember_tabs(&app.read());
    remember_hash(Some("vault"));
}

pub fn load(mut app: Signal<App>) {
    spawn(async move {
        let Ok(panel) = api::get::<api::VaultPanel>("/api/vault").await else {
            return;
        };
        let mut state = app.write();
        let open = state.vault.open.clone();
        if let Some(open) = open {
            if !panel
                .environments
                .iter()
                .any(|environment| vault::key(&environment.project, &environment.env) == open)
            {
                state.vault.open = None;
                state.vault.secrets = None;
            }
        }
        state.vault.panel = Some(panel);
    });
}

pub fn open_environment(mut app: Signal<App>, project: String, env: String) {
    let key = vault::key(&project, &env);
    let reopen = app.read().vault.open.as_deref() == Some(key.as_str());
    {
        let mut state = app.write();
        state.vault.editing = None;
        state.vault.revealed.clear();
        if reopen {
            state.vault.open = None;
            state.vault.secrets = None;
            return;
        }
        state.vault.open = Some(key.clone());
        state.vault.secrets = None;
    }
    load_secrets(app, project, env);
}

fn load_secrets(mut app: Signal<App>, project: String, env: String) {
    spawn(async move {
        let path = format!("/api/vault/secrets?project={project}&env={env}");
        let Ok(answer) = api::get::<api::VaultSecrets>(&path).await else {
            return;
        };
        let key = vault::key(&project, &env);
        let mut state = app.write();
        if state.vault.open.as_deref() != Some(key.as_str()) {
            return;
        }
        state.vault.secrets = Some(answer.secrets.into_iter().collect());
        state.vault.plain = answer.plain;
    });
}

pub fn order(app: Signal<App>, path: &'static str, body: serde_json::Value) {
    spawn(async move {
        let app = app;
        if let Err(message) = api::post_quiet(path, &body).await {
            toast(app, &message);
            return;
        }
        load(app);
        let open = app.read().vault.open.clone();
        if let Some((project, env)) = open.as_deref().and_then(vault::split) {
            load_secrets(app, project, env);
        }
    });
}

fn order_secrets(app: Signal<App>, path: &'static str, body: serde_json::Value) {
    spawn(async move {
        let mut app = app;
        if let Err(message) = api::post_quiet(path, &body).await {
            toast(app, &message);
            return;
        }
        app.write().vault.editing = None;
        load(app);
        let open = app.read().vault.open.clone();
        if let Some((project, env)) = open.as_deref().and_then(vault::split) {
            load_secrets(app, project, env);
        }
    });
}

fn create_token(app: Signal<App>, body: serde_json::Value) {
    spawn(async move {
        let mut app = app;
        match api::post::<api::Minted>("/api/vault/token", &body).await {
            Ok(minted) => {
                app.write().vault.minted = Some(minted.plain);
                load(app);
            }
            Err(message) => toast(app, &message),
        }
    });
}

fn copy(text: &str, label: &str) -> Element {
    rsx! {
        button {
            class: "copy",
            "data-copy": "{text}",
            title: "{label}",
            "aria-label": "{label}",
            Icon { name: "copy", size: 14 }
        }
    }
}

fn note(text: &str) -> Element {
    rsx! {
        div { class: "vault-note", "{text}" }
    }
}

fn clock(ts: i64) -> String {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from(ts as f64 * 1000.0));
    vault::clock(
        &format!(
            "{:04}-{:02}-{:02}",
            date.get_full_year(),
            date.get_month() + 1,
            date.get_date()
        ),
        &format!("{:02}:{:02}", date.get_hours(), date.get_minutes()),
    )
}

#[component]
pub fn Pane(app: Signal<App>) -> Element {
    let state = app.read();
    let vault_state = state.vault.clone();
    let Some(panel) = vault_state.panel.clone() else {
        drop(state);
        return rsx! {
            div { class: "vault-inner", {note("Leyendo…")} }
        };
    };
    drop(state);

    let groups = vault::projects(
        &panel
            .environments
            .iter()
            .map(|environment| uicore::vault::Environment {
                project: environment.project.clone(),
                env: environment.env.clone(),
                keys: environment.keys,
            })
            .collect::<Vec<_>>(),
    );
    let open = vault_state.open.clone();
    let editing = vault_state.editing.clone();
    let revealed = vault_state.revealed.clone();
    let plain = vault_state.plain.clone();
    let secrets = vault_state.secrets.clone().unwrap_or_default();

    rsx! {
        div { class: "vault-inner",
                if !panel.enabled {
                    div { class: "vault-off",
                        "El vault no abrió: {panel.error.clone().unwrap_or_default()}"
                    }
                } else {
                    section { class: "vault-section",
                        div { class: "vault-head", h2 { "Entornos" } }
                        for group in groups {
                            div { key: "{group.project}", class: "vault-project",
                                h3 { class: "vault-project-name", "{group.project}" }
                                div { class: "vault-envs",
                                    for environment in group.environments.clone() {
                                        {
                                            let key = vault::key(&environment.project, &environment.env);
                                            let active = open.as_deref() == Some(key.as_str());
                                            let project = environment.project.clone();
                                            let env = environment.env.clone();
                                            rsx! {
                                                button {
                                                    key: "{key}",
                                                    class: if active { "vault-env open" } else { "vault-env" },
                                                    onclick: move |_| open_environment(app, project.clone(), env.clone()),
                                                    span { class: "vault-env-name", "{environment.env}" }
                                                    span { class: "vault-env-count", "{vault::count(environment.keys)}" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if panel.environments.is_empty() {
                            {note("Todavía no hay entornos.")}
                        }
                        form {
                            class: "vault-form",
                            onsubmit: move |event: FormEvent| {
                                event.prevent_default();
                                let (project, env) = {
                                    let state = app.read();
                                    (state.vault.env_project.clone(), state.vault.env_name.clone())
                                };
                                let body = serde_json::json!({
                                    "op": "create",
                                    "project": vault::value(&project),
                                    "env": vault::value(&env),
                                });
                                order(app, "/api/vault/environment", body);
                            },
                            input {
                                placeholder: "proyecto",
                                "aria-label": "proyecto",
                                spellcheck: "false",
                                value: "{vault_state.env_project}",
                                oninput: move |event: FormEvent| app.write().vault.env_project = event.value(),
                            }
                            input {
                                placeholder: "entorno",
                                "aria-label": "entorno",
                                spellcheck: "false",
                                value: "{vault_state.env_name}",
                                oninput: move |event: FormEvent| app.write().vault.env_name = event.value(),
                            }
                            button { "Crear entorno" }
                        }
                    }
                    section { class: "vault-section",
                        if let Some(open) = open.clone() {
                            div { class: "vault-head",
                                h2 { "{open}" }
                                {copy(
                                    &secrets
                                        .iter()
                                        .map(|(name, value)| format!("{name}={value}"))
                                        .collect::<Vec<_>>()
                                        .join("\n"),
                                    "Copiar como .env",
                                )}
                                button {
                                    class: "icon-btn danger",
                                    title: "Borrar el entorno",
                                    "aria-label": "Borrar el entorno",
                                    onclick: {
                                        let open = open.clone();
                                        move |_| {
                                            if let Some((project, env)) = vault::split(&open) {
                                                let body = serde_json::json!({ "op": "drop", "project": project, "env": env });
                                                order(app, "/api/vault/environment", body);
                                            }
                                        }
                                    },
                                    Icon { name: "close", size: 14 }
                                }
                            }
                            div { class: "vault-keys",
                                for (name, value) in secrets.clone() {
                                    if editing.as_deref() == Some(name.as_str()) {
                                        {
                                            let (project, env) =
                                                vault::split(&open).unwrap_or_default();
                                            rsx! {
                                                div { key: "{name}", class: "vault-row vault-editing",
                                                    span { class: "vault-key", "{name}" }
                                                    form {
                                                        class: "vault-form vault-edit",
                                                        onsubmit: {
                                                            let name = name.clone();
                                                            let project = project.clone();
                                                            let env = env.clone();
                                                            move |event: FormEvent| {
                                                                event.prevent_default();
                                                                let (value, secret) = {
                                                                    let state = app.read();
                                                                    (state.vault.edit_value.clone(), state.vault.edit_secret)
                                                                };
                                                                let body = serde_json::json!({
                                                                    "project": project,
                                                                    "env": env,
                                                                    "name": name,
                                                                    "value": vault::value(&value),
                                                                    "secret": secret,
                                                                });
                                                                order_secrets(app, "/api/vault/set", body);
                                                            }
                                                        },
                                                        input {
                                                            "aria-label": "Valor de {name}",
                                                            spellcheck: "false",
                                                            autocomplete: "off",
                                                            value: "{vault_state.edit_value}",
                                                            oninput: move |event: FormEvent| app.write().vault.edit_value = event.value(),
                                                        }
                                                        label { class: "vault-check",
                                                            input {
                                                                r#type: "checkbox",
                                                                "aria-label": "secreto",
                                                                checked: vault_state.edit_secret,
                                                                onchange: move |event| app.write().vault.edit_secret = event.checked(),
                                                            }
                                                            "secreto"
                                                        }
                                                        button { "Guardar" }
                                                        button {
                                                            r#type: "button",
                                                            onclick: move |_| app.write().vault.editing = None,
                                                            "Cancelar"
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    } else {
                                        {
                                            let shown = revealed.contains(&name);
                                            let is_plain = plain.contains(&name);
                                            let masked = vault::mask(&value);
                                            let click_name = name.clone();
                                            rsx! {
                                                div { key: "{name}", class: "vault-row",
                                                    span { class: "vault-key",
                                                        "{name}"
                                                        if is_plain {
                                                            span {
                                                                class: "vault-tag",
                                                                title: "Variable: no se redacta",
                                                                "var"
                                                            }
                                                        }
                                                    }
                                                    button {
                                                        class: if shown { "vault-value" } else { "vault-value masked" },
                                                        title: if shown { "Ocultar" } else { "Mostrar" },
                                                        onclick: move |_| {
                                                            let mut state = app.write();
                                                            let name = click_name.clone();
                                                            match state.vault.revealed.iter().position(|item| item == &name) {
                                                                Some(index) => {
                                                                    state.vault.revealed.remove(index);
                                                                }
                                                                None => state.vault.revealed.push(name),
                                                            }
                                                        },
                                                        if shown { "{value}" } else { "{masked}" }
                                                    }
                                                    div { class: "vault-actions",
                                                        {copy(&value, &format!("Copiar {name}"))}
                                                        button {
                                                            class: "icon-btn",
                                                            title: "Editar {name}",
                                                            "aria-label": "Editar {name}",
                                                            onclick: {
                                                                let name = name.clone();
                                                                let value = value.clone();
                                                                move |_| {
                                                                    let mut state = app.write();
                                                                    state.vault.editing = Some(name.clone());
                                                                    state.vault.edit_value = value.clone();
                                                                    state.vault.edit_secret = !is_plain;
                                                                }
                                                            },
                                                            Icon { name: "pencil", size: 14 }
                                                        }
                                                        button {
                                                            class: "icon-btn danger",
                                                            title: "Borrar {name}",
                                                            "aria-label": "Borrar {name}",
                                                            onclick: {
                                                                let name = name.clone();
                                                                let open = open.clone();
                                                                move |_| {
                                                                    if let Some((project, env)) = vault::split(&open) {
                                                                        let body = serde_json::json!({
                                                                            "project": project,
                                                                            "env": env,
                                                                            "name": name,
                                                                        });
                                                                        order_secrets(app, "/api/vault/unset", body);
                                                                    }
                                                                }
                                                            },
                                                            Icon { name: "close", size: 14 }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                if secrets.is_empty() {
                                    {note(if vault_state.secrets.is_some() { "No hay claves." } else { "Leyendo…" })}
                                }
                            }
                            form {
                                class: "vault-form",
                                onsubmit: {
                                    let open = open.clone();
                                    move |event: FormEvent| {
                                        event.prevent_default();
                                        let (name, value, secret) = {
                                            let state = app.read();
                                            (state.vault.key_name.clone(), state.vault.key_value.clone(), state.vault.key_secret)
                                        };
                                        if let Some((project, env)) = vault::split(&open) {
                                            let body = serde_json::json!({
                                                "project": project,
                                                "env": env,
                                                "name": vault::value(&name),
                                                "value": value,
                                                "secret": secret,
                                            });
                                            order_secrets(app, "/api/vault/set", body);
                                        }
                                    }
                                },
                                input {
                                    placeholder: "CLAVE",
                                    "aria-label": "CLAVE",
                                    spellcheck: "false",
                                    autocomplete: "off",
                                    value: "{vault_state.key_name}",
                                    oninput: move |event: FormEvent| app.write().vault.key_name = event.value(),
                                }
                                input {
                                    placeholder: "valor",
                                    "aria-label": "valor",
                                    spellcheck: "false",
                                    autocomplete: "off",
                                    value: "{vault_state.key_value}",
                                    oninput: move |event: FormEvent| app.write().vault.key_value = event.value(),
                                }
                                label { class: "vault-check",
                                    input {
                                        r#type: "checkbox",
                                        "aria-label": "secreto",
                                        checked: vault_state.key_secret,
                                        onchange: move |event| app.write().vault.key_secret = event.checked(),
                                    }
                                    "secreto"
                                }
                                button { "Guardar" }
                            }
                        } else {
                            {note("Elegí un entorno.")}
                        }
                    }
                    section { class: "vault-section",
                        div { class: "vault-head", h2 { "Tokens" } }
                        if let Some(minted) = vault_state.minted.clone() {
                            div { class: "vault-minted",
                                code { "{minted}" }
                                {copy(&minted, "Copiar el token")}
                                {note("Se muestra una sola vez: después no lo guarda nadie.")}
                            }
                        }
                        div { class: "vault-keys",
                            for token in panel.tokens.clone() {
                                div { key: "{token.id}", class: "vault-row",
                                    span { class: "vault-key", "{token.name}" }
                                    span { class: "vault-scope", "{vault::scope(token.admin, &token.project, &token.env)}" }
                                    button { class: "vault-value",
                                        {token
                                            .keys
                                            .clone()
                                            .filter(|keys| !keys.is_empty())
                                            .map(|keys| keys.join(", "))
                                            .unwrap_or_else(|| "todas las claves".to_string())}
                                    }
                                    span { class: "vault-when",
                                        {match token.last_used {
                                            Some(ts) => format!("usado {}", clock(ts)),
                                            None => "sin usar".to_string(),
                                        }}
                                    }
                                    div { class: "vault-actions",
                                        button {
                                            class: "icon-btn danger",
                                            title: "Revocar {token.name}",
                                            "aria-label": "Revocar {token.name}",
                                            onclick: {
                                                let id = token.id.clone();
                                                move |_| {
                                                    let body = serde_json::json!({ "op": "revoke", "id": id });
                                                    order(app, "/api/vault/token", body);
                                                }
                                            },
                                            Icon { name: "close", size: 14 }
                                        }
                                    }
                                }
                            }
                            if panel.tokens.is_empty() {
                                {note("No hay tokens.")}
                            }
                        }
                        form {
                            class: "vault-form",
                            onsubmit: move |event: FormEvent| {
                                event.prevent_default();
                                let state = app.read();
                                let keys: Vec<String> = state
                                    .vault
                                    .token_keys
                                    .split(',')
                                    .map(str::trim)
                                    .filter(|key| !key.is_empty())
                                    .map(str::to_string)
                                    .collect();
                                let body = serde_json::json!({
                                    "op": "create",
                                    "name": vault::value(&state.vault.token_name),
                                    "project": vault::value(&state.vault.token_project),
                                    "env": vault::value(&state.vault.token_env),
                                    "keys": keys,
                                });
                                drop(state);
                                create_token(app, body);
                            },
                            input {
                                placeholder: "nombre",
                                "aria-label": "nombre",
                                spellcheck: "false",
                                value: "{vault_state.token_name}",
                                oninput: move |event: FormEvent| app.write().vault.token_name = event.value(),
                            }
                            input {
                                placeholder: "proyecto",
                                "aria-label": "proyecto del token",
                                spellcheck: "false",
                                value: "{vault_state.token_project}",
                                oninput: move |event: FormEvent| app.write().vault.token_project = event.value(),
                            }
                            input {
                                placeholder: "entorno",
                                "aria-label": "entorno del token",
                                spellcheck: "false",
                                value: "{vault_state.token_env}",
                                oninput: move |event: FormEvent| app.write().vault.token_env = event.value(),
                            }
                            input {
                                placeholder: "claves (opcional)",
                                "aria-label": "claves (opcional)",
                                spellcheck: "false",
                                value: "{vault_state.token_keys}",
                                oninput: move |event: FormEvent| app.write().vault.token_keys = event.value(),
                            }
                            button { "Crear token" }
                        }
                    }
                    section { class: "vault-section",
                        div { class: "vault-head", h2 { "Audit" } }
                        div { class: "vault-keys",
                            for (index, entry) in panel.audit.iter().enumerate() {
                                div { key: "{index}", class: "vault-row",
                                    span { class: "vault-when", "{clock(entry.at)}" }
                                    span { class: "vault-scope", "{entry.actor}" }
                                    span { class: "vault-audit",
                                        "{vault::action(&entry.action)} {entry.project}/{entry.env}"
                                    }
                                }
                            }
                            if panel.audit.is_empty() {
                                {note("Todavía no hay movimientos.")}
                            }
                        }
                    }
                }
            }
    }
}
