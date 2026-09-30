/* El vault: los secretos de jimmy, con la cara de casa. El store vive en este
   mismo proceso y el server escucha en loopback, así que acá no hay login
   propio: entra el que ya entró a jimmy, y esa visita queda en el audit. */

const VAULT = "vault";

const VAULT_ACTIONS = {
  "get-secrets": "leyó",
  set: "escribió",
  unset: "borró",
  "env-create": "creó el entorno",
  "env-drop": "borró el entorno",
  "token-create": "creó un token",
  "token-revoke": "revocó un token",
};

let vaultPanel = null;
let vaultSecrets = null;
let vaultOpen = null;
let vaultRevealed = new Set();
let vaultMinted = null;

function isVault(id) {
  return id === VAULT;
}

function vaultKey(environment) {
  return `${environment.project}/${environment.env}`;
}

function vaultScope(token) {
  if (token.admin) return "admin";
  return `${token.project}/${token.env}`;
}

function vaultCount(keys) {
  return keys === 1 ? "1 clave" : `${keys} claves`;
}

function vaultMask(value) {
  return "•".repeat(Math.min([...String(value)].length, 16));
}

function vaultAction(action) {
  return VAULT_ACTIONS[action] || action;
}

function vaultClock(ts) {
  const date = new Date(ts * 1000);
  const pad = (value) => String(value).padStart(2, "0");
  const day = `${pad(date.getDate())}/${pad(date.getMonth() + 1)}`;
  return `${day} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

function createVaultTab(id) {
  const pane = document.createElement("div");
  pane.className = "pane vault";
  const inner = document.createElement("div");
  inner.className = "vault-inner";
  pane.append(inner);
  panesEl.append(pane);

  const tab = { id, pane, inner, vault: true, viewers: [] };
  tab.item = tabEl(tab);
  tabs.set(id, tab);
  loadVault();
  return tab;
}

async function loadVault() {
  const data = await api("/api/vault");
  if (!data) return;
  vaultPanel = data;
  const environments = data.environments || [];
  if (vaultOpen && !environments.some((environment) => vaultKey(environment) === vaultOpen)) {
    vaultOpen = null;
    vaultSecrets = null;
  }
  renderVault();
}

async function loadSecrets(environment) {
  const query = new URLSearchParams({ project: environment.project, env: environment.env });
  const data = await api(`/api/vault/secrets?${query}`);
  const key = vaultKey(environment);
  if (!data || vaultOpen !== key) return;
  vaultSecrets = data.secrets || {};
  renderVault();
}

function renderVault() {
  const tab = tabs.get(VAULT);
  if (!tab || !vaultPanel) return;
  tab.inner.replaceChildren();
  if (!vaultPanel.enabled) {
    const off = document.createElement("div");
    off.className = "vault-off";
    off.textContent = `El vault no abrió: ${vaultPanel.error}`;
    tab.inner.append(off);
    return;
  }
  tab.inner.append(vaultEnvsEl(), vaultSecretsEl(), vaultTokensEl(), vaultAuditEl());
}

function vaultSection(title, actions = []) {
  const section = document.createElement("section");
  section.className = "vault-section";
  const head = document.createElement("div");
  head.className = "vault-head";
  const name = document.createElement("h2");
  name.textContent = title;
  head.append(name, ...actions);
  section.append(head);
  return section;
}

function vaultEnvsEl() {
  const section = vaultSection("Entornos");
  const list = document.createElement("div");
  list.className = "vault-envs";
  for (const environment of vaultPanel.environments || []) {
    const key = vaultKey(environment);
    const button = document.createElement("button");
    button.className = "vault-env" + (key === vaultOpen ? " open" : "");
    const name = document.createElement("span");
    name.className = "vault-env-name";
    name.textContent = key;
    const count = document.createElement("span");
    count.className = "vault-env-count";
    count.textContent = vaultCount(environment.keys);
    button.append(name, count);
    button.onclick = () => openVaultEnv(environment);
    list.append(button);
  }
  if (!list.children.length) list.append(vaultNote("Todavía no hay entornos."));
  section.append(list);
  section.append(
    vaultForm([{ placeholder: "proyecto" }, { placeholder: "entorno" }], "Crear entorno", (values) =>
      vaultOrder("/api/vault/environment", { op: "create", project: values[0], env: values[1] }),
    ),
  );
  return section;
}

async function openVaultEnv(environment) {
  const key = vaultKey(environment);
  if (vaultOpen === key) {
    vaultOpen = null;
    vaultSecrets = null;
    vaultRevealed.clear();
    return renderVault();
  }
  vaultOpen = key;
  vaultSecrets = null;
  vaultRevealed.clear();
  renderVault();
  await loadSecrets(environment);
}

function vaultSecretsEl() {
  if (!vaultOpen) return vaultNote("Elegí un entorno.");
  const [project, env] = vaultOpen.split("/");
  const secrets = vaultSecrets || {};
  const keys = Object.keys(secrets);
  const section = vaultSection(vaultOpen, [
    copyButton(keys.map((name) => `${name}=${secrets[name]}`).join("\n"), "Copiar como .env"),
    iconButton("close", "Borrar el entorno", () =>
      vaultOrder("/api/vault/environment", { op: "drop", project, env }),
    ),
  ]);

  const list = document.createElement("div");
  list.className = "vault-keys";
  for (const name of keys) {
    const row = document.createElement("div");
    row.className = "vault-row";
    const label = document.createElement("span");
    label.className = "vault-key";
    label.textContent = name;
    const shown = vaultRevealed.has(name);
    const value = document.createElement("button");
    value.className = "vault-value" + (shown ? "" : " masked");
    value.textContent = shown ? secrets[name] : vaultMask(secrets[name]);
    value.title = shown ? "Ocultar" : "Mostrar";
    value.onclick = () => {
      if (shown) vaultRevealed.delete(name);
      else vaultRevealed.add(name);
      renderVault();
    };
    const actions = document.createElement("div");
    actions.className = "vault-actions";
    actions.append(copyButton(secrets[name], `Copiar ${name}`));
    actions.append(
      iconButton("close", `Borrar ${name}`, () => vaultOrder("/api/vault/unset", { project, env, name })),
    );
    row.append(label, value, actions);
    list.append(row);
  }
  if (!keys.length) list.append(vaultNote(vaultSecrets ? "No hay claves." : "Leyendo…"));
  section.append(list);
  section.append(
    vaultForm(
      [{ placeholder: "CLAVE" }, { placeholder: "valor" }],
      "Guardar",
      (values) => vaultOrder("/api/vault/set", { project, env, name: values[0], value: values[1] }),
    ),
  );
  return section;
}

function vaultTokensEl() {
  const section = vaultSection("Tokens");
  if (vaultMinted) {
    const box = document.createElement("div");
    box.className = "vault-minted";
    const text = document.createElement("code");
    text.textContent = vaultMinted;
    box.append(text, copyButton(vaultMinted, "Copiar el token"));
    box.append(vaultNote("Se muestra una sola vez: después no lo guarda nadie."));
    section.append(box);
  }
  const list = document.createElement("div");
  list.className = "vault-keys";
  for (const token of vaultPanel.tokens || []) {
    const row = document.createElement("div");
    row.className = "vault-row";
    const name = document.createElement("span");
    name.className = "vault-key";
    name.textContent = token.name;
    const scope = document.createElement("span");
    scope.className = "vault-scope";
    scope.textContent = vaultScope(token);
    const used = document.createElement("span");
    used.className = "vault-when";
    used.textContent = token.last_used ? `usado ${vaultClock(token.last_used)}` : "sin usar";
    const rule = document.createElement("button");
    rule.className = "vault-value";
    rule.textContent = token.keys && token.keys.length ? token.keys.join(", ") : "todas las claves";
    const actions = document.createElement("div");
    actions.className = "vault-actions";
    actions.append(iconButton("close", `Revocar ${token.name}`, () => vaultRevoke(token)));
    row.append(name, scope, rule, used, actions);
    list.append(row);
  }
  if (!list.children.length) list.append(vaultNote("No hay tokens."));
  section.append(list);
  section.append(
    vaultForm(
      [{ placeholder: "nombre" }, { placeholder: "proyecto" }, { placeholder: "entorno" }, { placeholder: "claves (opcional)" }],
      "Crear token",
      async (values) => {
        const data = await api("/api/vault/token", {
          op: "create",
          name: values[0],
          project: values[1],
          env: values[2],
          keys: values[3]
            .split(",")
            .map((key) => key.trim())
            .filter(Boolean),
        });
        if (!data) return false;
        vaultMinted = data.plain;
        await loadVault();
        return true;
      },
    ),
  );
  return section;
}

function vaultAuditEl() {
  const section = vaultSection("Audit");
  const list = document.createElement("div");
  list.className = "vault-keys";
  for (const entry of vaultPanel.audit || []) {
    const row = document.createElement("div");
    row.className = "vault-row";
    const when = document.createElement("span");
    when.className = "vault-when";
    when.textContent = vaultClock(entry.at);
    const who = document.createElement("span");
    who.className = "vault-scope";
    who.textContent = entry.actor;
    const what = document.createElement("span");
    what.className = "vault-audit";
    what.textContent = `${vaultAction(entry.action)} ${entry.project}/${entry.env}${entry.name ? " · " + entry.name : ""}`;
    row.append(when, who, what);
    list.append(row);
  }
  if (!list.children.length) list.append(vaultNote("Todavía no hay movimientos."));
  section.append(list);
  return section;
}

function vaultNote(text) {
  const note = document.createElement("div");
  note.className = "vault-note";
  note.textContent = text;
  return note;
}

function vaultForm(placeholders, action, submit) {
  const form = document.createElement("form");
  form.className = "vault-form";
  const inputs = placeholders.map(({ placeholder, type }) => {
    const input = document.createElement("input");
    input.placeholder = placeholder;
    input.setAttribute("aria-label", placeholder);
    input.type = type || "text";
    input.spellcheck = false;
    input.autocomplete = "off";
    input.autocapitalize = "off";
    form.append(input);
    return input;
  });
  const button = document.createElement("button");
  button.textContent = action;
  form.append(button);
  form.onsubmit = async (event) => {
    event.preventDefault();
    const values = inputs.map((input) => input.value.trim());
    if (await submit(values)) for (const input of inputs) input.value = "";
  };
  return form;
}

async function vaultOrder(path, body) {
  if (!(await api(path, body))) return false;
  await loadVault();
  return true;
}

async function vaultRevoke(token) {
  await vaultOrder("/api/vault/token", { op: "revoke", id: token.id });
}

if (typeof module !== "undefined") {
  module.exports = { vaultKey, vaultScope, vaultCount, vaultMask, vaultAction, vaultClock };
}
