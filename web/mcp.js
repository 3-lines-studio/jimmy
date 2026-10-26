const MCP = "mcp";

let mcpServers = null;
let mcpBusy = false;

function isMcp(id) {
  return id === MCP;
}

function createMcpTab(id) {
  const pane = document.createElement("div");
  pane.className = "pane mcp";
  const inner = document.createElement("div");
  inner.className = "mcp-inner";
  pane.append(inner);
  panesEl.append(pane);

  const tab = { id, pane, inner, mcp: true, viewers: [] };
  tab.item = tabEl(tab);
  tabs.set(id, tab);
  loadMcp();
  return tab;
}

async function loadMcp() {
  const data = await api("/api/mcp");
  if (!data) return;
  mcpServers = data.servers || [];
  renderMcp();
}

function renderMcp() {
  const tab = tabs.get(MCP);
  if (!tab) return;
  tab.inner.replaceChildren();
  tab.inner.append(mcpIntroEl(), mcpListEl(), mcpFormEl());
}

function mcpSection(title) {
  const section = document.createElement("section");
  section.className = "mcp-section";
  const head = document.createElement("div");
  head.className = "mcp-head";
  const name = document.createElement("h2");
  name.textContent = title;
  head.append(name);
  section.append(head);
  return section;
}

function mcpIntroEl() {
  const section = mcpSection("Servidores MCP");
  const note = document.createElement("p");
  note.className = "mcp-note";
  note.textContent =
    "Conectá un servidor MCP por su URL. Jimmy descubre sus endpoints, se registra solo si el " +
    "servidor lo permite y guarda el token en el vault, una cuenta por entorno.";
  section.append(note);
  return section;
}

function mcpListEl() {
  const section = mcpSection("Conectados");
  if (!mcpServers || !mcpServers.length) {
    const empty = document.createElement("p");
    empty.className = "mcp-note";
    empty.textContent = "Todavía no hay servidores conectados.";
    section.append(empty);
    return section;
  }
  const list = document.createElement("div");
  list.className = "mcp-list";
  for (const server of mcpServers) {
    const row = document.createElement("div");
    row.className = "mcp-server";

    const info = document.createElement("div");
    info.className = "mcp-server-info";
    const name = document.createElement("span");
    name.className = "mcp-server-name";
    name.textContent = server.name;
    const meta = document.createElement("span");
    meta.className = "mcp-server-meta";
    meta.textContent = `${server.account} · ${server.url}`;
    info.append(name, meta);

    const actions = document.createElement("div");
    actions.className = "mcp-server-actions";
    actions.append(
      iconButton("key", "Ver tools", () => openMcpTools(server.name)),
      iconButton("close", "Desconectar", () => disconnectMcp(server.name), "danger"),
    );

    row.append(info, actions);
    list.append(row);
  }
  section.append(list);
  return section;
}

function mcpFormEl() {
  const section = mcpSection("Conectar uno nuevo");
  const form = document.createElement("div");
  form.className = "mcp-form";

  const name = mcpInput("nombre", "notion");
  const url = mcpInput("url", "https://mcp.notion.com/mcp");
  const account = mcpInput("cuenta", "manu");

  const button = document.createElement("button");
  button.className = "mcp-connect";
  button.textContent = "Conectar";
  button.disabled = mcpBusy;
  button.onclick = () => startMcp(name.value, url.value, account.value);

  form.append(name, url, account, button);
  section.append(form);
  return section;
}

function mcpInput(placeholder, value) {
  const input = document.createElement("input");
  input.className = "mcp-input";
  input.placeholder = placeholder;
  input.value = value;
  return input;
}

function startMcp(name, url, account) {
  if (!name.trim() || !url.trim() || !account.trim()) {
    notify("Completá nombre, URL y cuenta");
    return;
  }
  const query = new URLSearchParams({ name: name.trim(), url: url.trim(), account: account.trim() });
  window.location.href = `/mcp/start?${query}`;
}

async function disconnectMcp(name) {
  mcpBusy = true;
  const data = await api("/api/mcp/disconnect", { name });
  mcpBusy = false;
  if (!data) return;
  await loadMcp();
  notify(`${name} desconectado`);
}

async function openMcpTools(name) {
  const query = new URLSearchParams({ name });
  const data = await api(`/mcp/tools?${query}`);
  if (!data) return;
  const tools = data.tools || [];
  const tab = tabs.get(MCP);
  if (!tab) return;
  tab.inner.replaceChildren();
  tab.inner.append(mcpToolsSection(name, tools), mcpBackEl());
}

function mcpToolsSection(name, tools) {
  const section = mcpSection(`Tools de ${name}`);
  if (!tools.length) {
    const empty = document.createElement("p");
    empty.className = "mcp-note";
    empty.textContent = "El servidor no expone tools.";
    section.append(empty);
    return section;
  }
  const list = document.createElement("div");
  list.className = "mcp-tools";
  for (const tool of tools) {
    const item = document.createElement("div");
    item.className = "mcp-tool";
    const title = document.createElement("span");
    title.className = "mcp-tool-name";
    title.textContent = tool.name;
    const description = document.createElement("span");
    description.className = "mcp-tool-description";
    description.textContent = tool.description || "";
    item.append(title, description);
    list.append(item);
  }
  section.append(list);
  return section;
}

function mcpBackEl() {
  const back = document.createElement("button");
  back.className = "mcp-back";
  back.textContent = "Volver";
  back.onclick = () => loadMcp();
  return back;
}
