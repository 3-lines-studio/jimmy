const ICON = {
  read: "file",
  write: "file-plus",
  edit: "pencil",
  bash: "terminal",
  search: "search",
  fetch: "download",
  browse: "globe",
};
const SVG_NS = "http://www.w3.org/2000/svg";
const PATHS = {
  menu: '<path d="M4 6h16"/><path d="M4 12h16"/><path d="M4 18h16"/>',
  sun: '<circle cx="12" cy="12" r="4"/><path d="M12 2v2"/><path d="M12 20v2"/><path d="m4.93 4.93 1.41 1.41"/><path d="m17.66 17.66 1.41 1.41"/><path d="M2 12h2"/><path d="M20 12h2"/><path d="m6.34 17.66-1.41 1.41"/><path d="m19.07 4.93-1.41 1.41"/>',
  moon: '<path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/>',
  search: '<circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/>',
  down: '<path d="m6 9 6 6 6-6"/>',
  left: '<path d="m15 18-6-6 6-6"/>',
  pencil:
    '<path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"/><path d="m15 5 4 4"/>',
  close: '<path d="M18 6 6 18"/><path d="m6 6 12 12"/>',
  logout:
    '<path d="m16 17 5-5-5-5"/><path d="M21 12H9"/><path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"/>',
  copy: '<rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/>',
  check: '<path d="M20 6 9 17l-5-5"/>',
  plus: '<path d="M5 12h14"/><path d="M12 5v14"/>',
  clip: '<path d="m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48"/>',
  up: '<path d="m5 12 7-7 7 7"/><path d="M12 19V5"/>',
  stop: '<rect width="13" height="13" x="5.5" y="5.5" rx="2"/>',
  terminal: '<path d="m4 17 6-6-6-6"/><path d="M12 19h8"/>',
  file: '<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/>',
  "file-plus":
    '<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="M9 15h6"/><path d="M12 12v6"/>',
  download:
    '<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="M7 10l5 5 5-5"/><path d="M12 15V3"/>',
  globe: '<circle cx="12" cy="12" r="10"/><path d="M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20"/><path d="M2 12h20"/>',
  dot: '<circle cx="12" cy="12" r="3"/>',
  folder:
    '<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>',
};

function icon(name, size = 16) {
  const svg = document.createElementNS(SVG_NS, "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("width", size);
  svg.setAttribute("height", size);
  svg.setAttribute("fill", "none");
  svg.setAttribute("stroke", "currentColor");
  svg.setAttribute("stroke-width", "2");
  svg.setAttribute("stroke-linecap", "round");
  svg.setAttribute("stroke-linejoin", "round");
  svg.setAttribute("aria-hidden", "true");
  svg.innerHTML = PATHS[name];
  return svg;
}
const NOUN = {
  read: ["archivo", "archivos"],
  write: ["archivo", "archivos"],
  edit: ["edición", "ediciones"],
  bash: ["comando", "comandos"],
  search: ["búsqueda", "búsquedas"],
  fetch: ["página", "páginas"],
  browse: ["página", "páginas"],
};

let state = { projects: [] };
const tabs = new Map();
let activeId = null;
let pending = new Map();
const collapsed = new Set();
const expanded = new Set();
const VISIBLE = 10;

const projectsEl = document.getElementById("projects");
const previewsEl = document.getElementById("previews");
const machineEl = document.getElementById("machine");
const tabsEl = document.getElementById("tabs");
const panesEl = document.getElementById("panes");
const placeholderEl = document.getElementById("placeholder");
const sidebar = document.getElementById("sidebar");
const backdrop = document.getElementById("backdrop");
const tabActionsEl = document.getElementById("tab-actions");
const viewersEl = document.getElementById("viewers");
const composerEl = document.getElementById("composer");
const typingEl = document.getElementById("typing");
const inputEl = document.getElementById("input");
const searchEl = document.getElementById("search");
const pendingEl = document.getElementById("pending");
const fileEl = document.getElementById("file");
const cancelEl = document.getElementById("cancel");
const onlineEl = document.getElementById("online");
const toastEl = document.getElementById("toast");
const finePointer = matchMedia("(hover: hover) and (pointer: fine)");

document.getElementById("menu").append(icon("menu", 18));
document.querySelector("#sidebar-search .field").prepend(icon("search", 15));
const themeEl = document.getElementById("theme");
const themeColor = document.querySelector('meta[name="theme-color"]');
const sunEl = icon("sun", 18);
const moonEl = icon("moon", 18);
sunEl.classList.add("sun");
moonEl.classList.add("moon");
themeEl.append(sunEl, moonEl);
document.getElementById("attach").append(icon("clip", 17));
document.querySelector("#logout button").append(icon("logout", 18));
document.querySelector("#new-project button").append(icon("plus", 17));
const sendEl = document.querySelector("#composer .send");
const sendIcon = icon("up", 17);
const stopIcon = icon("stop", 15);
sendIcon.classList.add("send-icon");
stopIcon.classList.add("stop-icon");
sendEl.append(sendIcon, stopIcon);

async function request(path, options) {
  let response;
  try {
    response = await fetch(path, options);
  } catch {
    notify("no hay conexión con jimmy");
    return null;
  }
  if (response.status === 401) {
    location.href = "/login";
    return null;
  }
  const data = await response.json().catch(() => null);
  if (!response.ok) {
    notify((data && data.error) || `jimmy contestó ${response.status}`);
    return null;
  }
  return data;
}

function api(path, body) {
  return request(path, {
    method: body === undefined ? "GET" : "POST",
    headers: body === undefined ? {} : { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
}

let toastTimer = null;

function notify(text) {
  toastEl.textContent = text;
  toastEl.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(hideToast, 6000);
}

function hideToast() {
  clearTimeout(toastTimer);
  toastEl.hidden = true;
}

toastEl.onclick = hideToast;

function conversationById(id) {
  for (const project of state.projects) {
    for (const conversation of project.conversations) {
      if (conversation.key === id) return conversation;
    }
  }
  return null;
}

function projectOf(id) {
  if (isFiles(id)) return filesProject(id);
  const conversation = conversationById(id);
  return conversation ? conversation.project : "";
}

function projectPill(name) {
  const pill = document.createElement("span");
  pill.className = "pill";
  pill.textContent = projectInitials(name);
  pill.style.background = projectColor(name);
  return pill;
}

function titleOf(id) {
  if (isFiles(id)) return "archivos: " + filesProject(id);
  const conversation = conversationById(id);
  return conversation ? conversation.title || conversation.key : id;
}

function knownTab(id) {
  return isFiles(id) || Boolean(conversationById(id));
}

function isRunning(id) {
  const conversation = conversationById(id);
  return conversation ? conversation.running : false;
}

function isReadOnly(id) {
  const conversation = conversationById(id);
  return conversation ? conversation.read_only : false;
}

function runningKeys(state) {
  const keys = new Set();
  for (const project of state.projects) {
    for (const conversation of project.conversations) {
      if (conversation.running) keys.add(conversation.key);
    }
  }
  return keys;
}

async function refresh() {
  const data = await api("/api/state");
  if (!data) return;
  const before = JSON.stringify(state.projects);
  const wasRunning = runningKeys(state);
  state = data;
  for (const project of data.projects) {
    for (const conversation of project.conversations) {
      if (conversation.running || !wasRunning.has(conversation.key)) continue;
      const tab = tabs.get(conversation.key);
      if (!tab || tab.id === activeId) continue;
      tab.attention = "done";
      searchDirty = true;
    }
  }
  const live = new Set();
  for (const project of state.projects) {
    for (const conversation of project.conversations) live.add(conversation.key);
  }
  for (const id of [...tabs.keys()]) {
    if (!live.has(id) && !isFiles(id)) closeTab(id);
  }
  if (searchEl.value.trim().length < 2) {
    renderSidebar();
  } else if (searchDirty || JSON.stringify(state.projects) !== before) {
    searchDirty = false;
    runSearch();
  }
  renderPreviews();
  renderMachine();
  renderTabs();
  renderActions();
  updateTitle();
}

/* Sidebar */

const groupEls = new Map();

async function stopPreview(name) {
  await api("/api/preview/stop", { name });
  await refresh();
}

function previewEl(preview) {
  const row = document.createElement("div");
  row.className = "preview";
  const link = document.createElement("a");
  link.className = "link";
  link.href = preview.path;
  link.target = "_blank";
  link.rel = "noopener";
  link.title = "abrir " + preview.path;
  const name = document.createElement("span");
  name.className = "name";
  name.textContent = preview.name;
  const port = document.createElement("span");
  port.className = "port";
  port.textContent = ":" + preview.port;
  link.append(name, port, icon("globe", 14));
  const stop = iconButton("stop", "parar " + preview.name, () => stopPreview(preview.name));
  row.append(link, stop);
  return row;
}

function renderPreviews() {
  const previews = state.previews || [];
  previewsEl.hidden = previews.length === 0;
  previewsEl.replaceChildren(...previews.map(previewEl));
}

function machineRow(label, value, title) {
  const row = document.createElement("div");
  row.className = "row";
  row.title = title;
  const name = document.createElement("span");
  name.className = "label";
  name.textContent = label;
  const amount = document.createElement("span");
  amount.className = "value";
  amount.textContent = value;
  row.append(name, amount);
  return row;
}

function renderMachine() {
  const machine = state.machine;
  machineEl.hidden = !machine;
  if (!machine) return;
  const detail = document.createElement("div");
  detail.className = "detail";
  detail.textContent = machineDetail(machine);
  machineEl.replaceChildren(
    machineRow(
      "RAM",
      machineRatio(machine.memory.used, machine.memory.total),
      "la memoria del contenedor, cache y kernel incluidos",
    ),
    machineRow(
      "disco",
      machineRatio(machine.disk.used, machine.disk.total),
      "el volumen " + state.workspace,
    ),
    detail,
  );
}

function projectEl(project) {
  const group = {
    el: document.createElement("div"),
    project,
    name: document.createElement("span"),
    list: document.createElement("div"),
    conversations: new Map(),
    more: document.createElement("button"),
  };
  group.el.className = "group";
  group.list.className = "threads";
  const header = document.createElement("div");
  header.className = "project toggle";
  header.onclick = () => toggleProject(group.project.name);
  const caret = document.createElement("span");
  caret.className = "caret";
  caret.append(icon("down", 14));
  group.name.className = "name";
  const add = iconButton("plus", "nueva conversación", async () => {
    const made = await api("/api/conversations", { project: group.project.name });
    await refresh();
    if (made && made.key) openTab(made.key);
    closeSidebar();
  });
  const files = iconButton("folder", "ver archivos", () => {
    openTab(FILES + group.project.name);
    closeSidebar();
  });
  const remove = iconButton("close", "quitar proyecto", () => deleteProject(group.project), "danger");
  group.more.className = "more";
  group.more.hidden = true;
  group.more.onclick = () => toggleMore(group.project.name);
  header.append(caret, projectPill(group.project.name), group.name, add, files, remove);
  group.el.append(header, group.list, group.more);
  return group;
}

function iconButton(name, title, action, kind = "") {
  const button = document.createElement("button");
  button.className = "icon-btn" + (kind ? " " + kind : "");
  button.append(icon(name, 15));
  button.title = title;
  button.setAttribute("aria-label", title);
  button.onclick = (event) => {
    event.stopPropagation();
    action();
  };
  return button;
}

function conversationEl(conversation) {
  const item = {
    el: document.createElement("div"),
    conversation,
    renaming: false,
    dot: document.createElement("span"),
    title: document.createElement("span"),
  };
  item.el.className = "conversation";
  item.el.dataset.conversation = conversation.key;
  item.dot.className = "dot";
  item.title.className = "title";
  item.el.append(item.dot, item.title);
  item.el.onclick = () => {
    openTab(item.conversation.key);
    closeSidebar();
  };
  if (!conversation.read_only) {
    item.title.ondblclick = (event) => {
      event.stopPropagation();
      rename(item);
    };
    item.el.append(
      iconButton("pencil", "renombrar", () => rename(item)),
      iconButton("close", "borrar conversación", () => deleteConversation(item.conversation), "danger"),
    );
  }
  return item;
}

function refreshConversation(item, conversation) {
  item.conversation = conversation;
  item.el.classList.toggle("active", conversation.key === activeId);
  const dot = "dot" + (conversation.running ? " running" : "");
  if (item.dot.className !== dot) item.dot.className = dot;
  if (!item.renaming) setText(item.title, conversation.title || conversation.key);
}

function setText(node, value) {
  if (node.textContent !== value) node.textContent = value;
}

function place(parent, child, after) {
  const before = after ? after.nextSibling : parent.firstChild;
  if (before === child) return;
  if (!before && parent.lastChild === child) return;
  parent.insertBefore(child, before);
}

function renderSidebar() {
  const alive = new Set();
  let lastGroup = null;
  for (const project of state.projects) {
    alive.add(project.name);
    let group = groupEls.get(project.name);
    if (!group) {
      group = projectEl(project);
      groupEls.set(project.name, group);
    }
    group.project = project;
    setText(group.name, project.name);
    group.name.title = project.path;
    group.el.classList.toggle("collapsed", collapsed.has(project.name));
    const open = expanded.has(project.name);
    setText(group.more, open ? "ver menos" : "ver más");
    group.more.hidden = project.conversations.length <= VISIBLE;
    const seen = new Set();
    let lastItem = null;
    const shown = open ? project.conversations : project.conversations.slice(0, VISIBLE);
    for (const conversation of shown) {
      seen.add(conversation.key);
      let item = group.conversations.get(conversation.key);
      if (!item) {
        item = conversationEl(conversation);
        group.conversations.set(conversation.key, item);
      }
      refreshConversation(item, conversation);
      place(group.list, item.el, lastItem);
      lastItem = item.el;
    }
    for (const [key, item] of group.conversations) {
      if (seen.has(key)) continue;
      item.el.remove();
      group.conversations.delete(key);
    }
    place(projectsEl, group.el, lastGroup);
    lastGroup = group.el;
  }
  for (const [name, group] of groupEls) {
    if (alive.has(name)) continue;
    group.el.remove();
    groupEls.delete(name);
  }
}

function toggleMore(name) {
  if (expanded.has(name)) expanded.delete(name);
  else expanded.add(name);
  renderSidebar();
}

function toggleProject(name) {
  if (collapsed.has(name)) collapsed.delete(name);
  else collapsed.add(name);
  const group = groupEls.get(name);
  if (group) group.el.classList.toggle("collapsed", collapsed.has(name));
}

function rename(item) {
  if (item.renaming) return;
  item.renaming = true;
  const input = document.createElement("input");
  input.className = "rename";
  input.value = item.conversation.title || "";
  item.title.replaceWith(input);
  input.focus();
  input.select();

  const commit = async () => {
    const value = input.value.trim();
    if (input.isConnected) input.replaceWith(item.title);
    item.renaming = false;
    if (!value || value === item.conversation.title) return;
    await api("/api/rename", { conversation: item.conversation.key, title: value });
    await refresh();
  };

  input.onblur = commit;
  input.onkeydown = (event) => {
    if (event.key === "Enter") input.blur();
    if (event.key === "Escape") {
      input.value = item.conversation.title || "";
      input.blur();
    }
  };
}

async function deleteConversation(conversation) {
  const name = conversation.title || conversation.key;
  if (!confirm(`¿Borrar la conversación "${name}"? Se pierde el historial.`)) return;
  await api("/api/delete-conversation", { conversation: conversation.key });
  closeTab(conversation.key);
  await refresh();
}

async function deleteProject(project) {
  if (!confirm(`¿Quitar "${project.name}" y todas sus conversaciones?`)) return;
  const body = { project: project.name };
  if (project.unversioned) {
    const warn = `"${project.name}" no está en git: lo que tenga adentro no existe en ningún otro lado.`;
    if (!confirm(`${warn} ¿Lo borro igual?`)) return;
    body.force = true;
  }
  for (const conversation of project.conversations) closeTab(conversation.key);
  closeTab(FILES + project.name);
  await api("/api/delete-project", body);
  await refresh();
}

/* Search */

let searchTimer = null;
let searchDirty = false;

searchEl.addEventListener("input", () => {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(runSearch, 250);
});

searchEl.addEventListener("keydown", (event) => {
  if (event.key !== "Escape") return;
  searchEl.value = "";
  clearFind();
  renderSidebar();
});

async function runSearch() {
  const query = searchEl.value.trim();
  if (query.length < 2) {
    clearFind();
    renderSidebar();
    return;
  }
  const data = await api(`/api/search?q=${encodeURIComponent(query)}`);
  renderResults(query, data && data.results ? data.results : []);
}

function clearFind() {
  for (const tab of tabs.values()) {
    tab.find = null;
    clearMarks(tab.transcript);
  }
}

function openResult(conversation, query) {
  const tab = tabs.get(conversation) || createTab(conversation);
  tab.find = query;
  activate(conversation);
  if (tab.synced) findIn(tab, query);
}

function renderResults(query, results) {
  groupEls.clear();
  projectsEl.replaceChildren();
  const header = document.createElement("div");
  header.className = "project";
  header.textContent = results.length ? `${results.length} resultados` : "sin resultados";
  projectsEl.append(header);
  let current = null;
  let group = null;
  for (const result of results) {
    if (result.conversation !== current) {
      current = result.conversation;
      group = document.createElement("div");
      group.className = "result-group";
      const title = document.createElement("div");
      title.className = "result-title";
      title.textContent = result.title || result.conversation;
      title.onclick = () => openResult(result.conversation, query);
      group.append(title);
      projectsEl.append(group);
    }
    const snippet = document.createElement("div");
    snippet.className = "result-snippet";
    snippet.textContent = result.snippet;
    snippet.onclick = () => openResult(result.conversation, query);
    group.append(snippet);
  }
}

function findIn(tab, query) {
  clearMarks(tab.transcript);
  const needle = query.toLowerCase();
  if (needle.length < 2) return;
  const walker = document.createTreeWalker(tab.transcript, NodeFilter.SHOW_TEXT);
  const nodes = [];
  while (walker.nextNode()) nodes.push(walker.currentNode);
  let first = null;
  for (const node of nodes) {
    const text = node.nodeValue;
    const lower = text.toLowerCase();
    if (!lower.includes(needle)) continue;
    const fragment = document.createDocumentFragment();
    let index = 0;
    let at = lower.indexOf(needle);
    while (at !== -1) {
      fragment.append(document.createTextNode(text.slice(index, at)));
      const mark = document.createElement("mark");
      mark.textContent = text.slice(at, at + needle.length);
      fragment.append(mark);
      if (!first) first = mark;
      index = at + needle.length;
      at = lower.indexOf(needle, index);
    }
    fragment.append(document.createTextNode(text.slice(index)));
    node.replaceWith(fragment);
  }
  if (first) first.scrollIntoView({ block: "center" });
}

function clearMarks(root) {
  for (const mark of root.querySelectorAll("mark")) {
    mark.replaceWith(document.createTextNode(mark.textContent));
  }
  root.normalize();
}

/* Tabs */

const TABS_KEY = "jimmy-tabs";

function readTabs() {
  try {
    const value = JSON.parse(localStorage.getItem(TABS_KEY) || "{}");
    return { open: value.open || [], active: value.active || null };
  } catch {
    return { open: [], active: null };
  }
}

function remember() {
  localStorage.setItem(TABS_KEY, JSON.stringify({ open: [...tabs.keys()], active: activeId }));
  history.replaceState(null, "", activeId ? "#" + encodeURIComponent(activeId) : location.pathname);
}

function restore() {
  const saved = readTabs();
  const hash = decodeURIComponent(location.hash.slice(1));
  const wanted = saved.open.includes(hash) || !hash ? saved.open : [...saved.open, hash];
  const seen = new Set();
  for (const id of wanted) {
    if (seen.has(id) || !knownTab(id)) continue;
    seen.add(id);
    if (!tabs.has(id)) createTab(id);
  }
  const active = [hash, saved.active].find((id) => tabs.has(id)) || tabs.keys().next().value;
  if (active) activate(active);
  else if (hash) remember();
}

function openTab(id) {
  if (!tabs.has(id)) createTab(id);
  activate(id);
}

function createTab(id) {
  if (isFiles(id)) return createFilesTab(id);
  const pane = document.createElement("div");
  pane.className = "pane";
  const transcript = document.createElement("div");
  transcript.className = "transcript";
  const jump = document.createElement("button");
  jump.className = "jump";
  jump.textContent = "ir al final ↓";
  jump.hidden = true;
  const earlier = document.createElement("button");
  earlier.className = "earlier";
  earlier.textContent = "ver anteriores ↑";
  earlier.hidden = true;
  pane.append(transcript, earlier, jump);
  panesEl.append(pane);

  const tab = {
    id,
    pane,
    transcript,
    jump,
    earlier,
    first: 0,
    loading: false,
    prepending: false,
    stream: null,
    live: null,
    steps: null,
    tools: {},
    follow: true,
    viewers: [],
    find: null,
    synced: false,
    attention: null,
    count: 0,
  };
  tab.item = tabEl(tab);
  tabs.set(id, tab);
  transcript.addEventListener("scroll", () => {
    tab.follow = transcript.scrollHeight - transcript.scrollTop - transcript.clientHeight < 120;
    jump.hidden = tab.follow;
  });
  jump.onclick = () => {
    tab.follow = true;
    jump.hidden = true;
    transcript.scrollTop = transcript.scrollHeight;
  };
  earlier.onclick = () => loadEarlier(tab);

  return tab;
}

function subscribe(tab) {
  if (tab.stream) tab.stream.close();
  const stream = new EventSource(
    `/api/stream?conversation=${encodeURIComponent(tab.id)}&since=${tab.count}`,
  );
  tab.stream = stream;
  stream.onopen = () => {
    if (stream.reconnected) {
      subscribe(tab);
      return;
    }
    stream.reconnected = true;
    tab.synced = false;
  };
  stream.onmessage = (message) => render(tab, JSON.parse(message.data));
}

function renderEarlier(tab) {
  tab.earlier.hidden = tab.first <= 0;
  tab.earlier.textContent = tab.loading ? "trayendo…" : "ver anteriores ↑";
}

async function loadEarlier(tab) {
  if (tab.loading || tab.first <= 0) return;
  tab.loading = true;
  renderEarlier(tab);
  const data = await api(
    `/api/history?conversation=${encodeURIComponent(tab.id)}&before=${tab.first}`,
  );
  tab.loading = false;
  if (!data || !data.events.length) {
    tab.first = 0;
    renderEarlier(tab);
    return;
  }
  prepend(tab, data.events);
  tab.first = data.first;
  renderEarlier(tab);
}

/// Los eventos viejos van arriba, en orden, y la vista se queda donde estaba:
/// el alto que creció arriba es el que se suma al scroll.
function prepend(tab, events) {
  const state = { live: tab.live, steps: tab.steps, tools: tab.tools };
  const height = tab.transcript.scrollHeight;
  tab.prepending = true;
  for (const event of [...events].reverse()) render(tab, event);
  tab.prepending = false;
  tab.live = state.live;
  tab.steps = state.steps;
  tab.tools = state.tools;
  tab.transcript.scrollTop += tab.transcript.scrollHeight - height;
}

function unsubscribe(tab) {
  if (!tab.stream) return;
  tab.stream.close();
  tab.stream = null;
}

function clearTab(tab) {
  tab.transcript.replaceChildren();
  tab.live = null;
  tab.steps = null;
  tab.tools = {};
  tab.count = 0;
  tab.first = 0;
  tab.synced = false;
  renderEarlier(tab);
}

function activate(id) {
  const previous = activeId ? tabs.get(activeId) : null;
  if (previous && previous.id !== id) unsubscribe(previous);
  activeId = id;
  for (const tab of tabs.values()) tab.pane.hidden = tab.id !== id;
  placeholderEl.hidden = tabs.size > 0;
  const tab = tabs.get(id);
  if (tab && !tab.stream && !tab.files) subscribe(tab);
  if (tab) tab.attention = null;
  if (searchEl.value.trim().length < 2) renderSidebar();
  renderTabs();
  renderActions();
  renderPending();
  updateTitle();
  remember();
  if (tab) tab.item.el.scrollIntoView({ block: "nearest", inline: "nearest" });
  if (finePointer.matches) inputEl.focus();
}

function closeTab(id) {
  const tab = tabs.get(id);
  if (!tab) return;
  unsubscribe(tab);
  tab.pane.remove();
  tab.item.el.remove();
  pending.delete(id);
  tabs.delete(id);
  if (activeId !== id) {
    remember();
    renderTabs();
    return;
  }
  const next = tabs.keys().next().value;
  if (next) {
    activate(next);
  } else {
    activeId = null;
    placeholderEl.hidden = false;
    remember();
    renderTabs();
    renderActions();
    renderPending();
  }
}

function tabEl(tab) {
  const item = {
    el: document.createElement("div"),
    dot: document.createElement("span"),
    title: document.createElement("span"),
  };
  item.el.className = "tab";
  item.dot.className = "dot";
  item.title.className = "title";
  const close = document.createElement("button");
  close.className = "close";
  close.append(icon("close", 15));
  close.title = "cerrar pestaña";
  close.setAttribute("aria-label", "cerrar pestaña");
  close.onclick = (event) => {
    event.stopPropagation();
    closeTab(tab.id);
  };
  item.el.append(projectPill(projectOf(tab.id)), item.dot, item.title, close);
  item.el.onclick = () => activate(tab.id);
  return item;
}

function renderTabs() {
  let last = null;
  for (const tab of tabs.values()) {
    tab.item.el.classList.toggle("active", tab.id === activeId);
    const status = isRunning(tab.id) ? "running" : tab.attention || "";
    const dot = "dot" + (status ? " " + status : "");
    if (tab.item.dot.className !== dot) tab.item.dot.className = dot;
    setText(tab.item.title, titleOf(tab.id));
    place(tabsEl, tab.item.el, last);
    last = tab.item.el;
  }
}

function renderActions() {
  const tab = activeId ? tabs.get(activeId) : null;
  const running = Boolean(tab) && isRunning(activeId);
  tabActionsEl.hidden = !tab || Boolean(tab.files);
  composerEl.hidden = !tab || Boolean(tab.files) || isReadOnly(activeId);
  cancelEl.hidden = !running || !composerEl.hidden;
  sendEl.classList.toggle("stop", running && !composerEl.hidden);
  sendEl.setAttribute("aria-label", running ? "frenar el turno" : "enviar");
  typingEl.hidden = !tab || !tab.typingUser;
  if (tab && tab.typingUser) typingEl.textContent = `${tab.typingUser} está escribiendo…`;
  if (tab)
    viewersEl.textContent = tab.viewers.length > 1 ? tab.viewers.join(", ") + " mirando" : "";
}

function updateTitle() {
  const count = [...tabs.values()].filter((tab) => tab.attention).length;
  document.title = count ? `(${count}) Jimmy` : "Jimmy";
}

/* Rendering */

const NOT_LOGGED = new Set(["delta", "tool_delta", "typing", "presence", "online", "synced"]);

function render(tab, event) {
  if (!NOT_LOGGED.has(event.event) && !tab.prepending) tab.count++;
  switch (event.event) {
    case "user":
      renderUser(tab, event);
      break;
    case "image":
      renderImage(tab, event);
      break;
    case "delta":
      renderDelta(tab, event);
      break;
    case "assistant":
      renderAssistant(tab, event);
      break;
    case "tool_start":
      startTool(tab, event);
      break;
    case "tool_delta":
      streamTool(tab, event);
      break;
    case "tool_result":
      finishTool(tab, event);
      break;
    case "error":
      renderError(tab, event);
      searchDirty = true;
      if (tab.id !== activeId) {
        tab.attention = "error";
        renderTabs();
        updateTitle();
      }
      break;
    case "stopped":
      renderStopped(tab, event);
      break;
    case "done":
      tab.steps = null;
      searchDirty = true;
      if (tab.id !== activeId) {
        tab.attention = "done";
        renderTabs();
        updateTitle();
      }
      break;
    case "presence":
      tab.viewers = event.users;
      if (tab.id === activeId) renderActions();
      break;
    case "typing":
      if (event.user === state.user) break;
      tab.typingUser = event.user;
      clearTimeout(tab.typingTimer);
      tab.typingTimer = setTimeout(() => {
        tab.typingUser = "";
        renderActions();
      }, 4000);
      if (tab.id === activeId) renderActions();
      break;
    case "synced":
      if (event.count < tab.count) {
        clearTab(tab);
        subscribe(tab);
        break;
      }
      tab.count = event.count || 0;
      tab.first = event.first || 0;
      tab.synced = true;
      renderEarlier(tab);
      if (tab.find) findIn(tab, tab.find);
      break;
    default:
      break;
  }
}

function scroll(tab) {
  if (tab.follow) tab.transcript.scrollTop = tab.transcript.scrollHeight;
  else tab.jump.hidden = false;
}

function append(tab, element) {
  if (tab.prepending) {
    tab.transcript.prepend(element);
    return;
  }
  tab.transcript.append(element);
  scroll(tab);
}

function renderUser(tab, event) {
  const element = document.createElement("div");
  element.className = "event user";
  if (event.text) element.textContent = event.text;
  if (event.author) {
    const author = document.createElement("div");
    author.className = "author";
    author.textContent = event.author;
    element.prepend(author);
  }
  if (event.images && event.images.length) {
    element.append(renderThumbs(tab, event.images));
  }
  append(tab, element);
  tab.live = null;
  tab.steps = null;
}

function renderThumbs(tab, images) {
  const thumbs = document.createElement("div");
  thumbs.className = "thumbs";
  for (const name of images || []) {
    const url = fileUrl(tab.id, name);
    const link = document.createElement("a");
    link.href = url;
    link.target = "_blank";
    link.rel = "noreferrer";
    const img = document.createElement("img");
    img.src = url;
    img.alt = name;
    link.append(img);
    thumbs.append(link);
  }
  return thumbs;
}

function renderImage(tab, event) {
  const element = document.createElement("div");
  element.className = "event image";
  element.append(renderThumbs(tab, [event.name]));
  if (event.caption) {
    const caption = document.createElement("div");
    caption.className = "caption";
    caption.textContent = event.caption;
    element.append(caption);
  }
  append(tab, element);
}

function renderDelta(tab, event) {
  if (!tab.live) {
    tab.live = document.createElement("div");
    tab.live.className = "event assistant streaming";
    tab.live.dataset.raw = "";
    append(tab, tab.live);
  }
  tab.live.dataset.raw += event.text;
  tab.live.textContent = tab.live.dataset.raw;
  scroll(tab);
}

function renderAssistant(tab, event) {
  if (!tab.live) {
    tab.live = document.createElement("div");
    tab.live.className = "event assistant";
    append(tab, tab.live);
  }
  tab.live.classList.remove("streaming");
  tab.live.innerHTML = markdown(event.text);
  decorate(tab.live, event.text);
  tab.live = null;
  tab.steps = null;
  scroll(tab);
}

function decorate(element, text) {
  if (!text.trim()) return;
  for (const pre of element.querySelectorAll("pre")) {
    const code = pre.textContent;
    const box = document.createElement("div");
    box.className = "code";
    pre.replaceWith(box);
    box.append(pre, copyButton(code, "copiar el código"));
  }
  const actions = document.createElement("div");
  actions.className = "actions";
  actions.append(copyButton(text, "copiar el mensaje"));
  element.append(actions);
}

function copyButton(text, label) {
  const button = document.createElement("button");
  button.className = "copy";
  button.title = label;
  button.setAttribute("aria-label", label);
  const idle = icon("copy", 14);
  idle.classList.add("idle");
  const done = icon("check", 14);
  done.classList.add("done");
  button.append(idle, done);
  button.onclick = () => copyText(button, text);
  return button;
}

async function copyText(button, text) {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    notify("no pude copiar");
    return;
  }
  button.classList.add("copied");
  clearTimeout(button.timer);
  button.timer = setTimeout(() => button.classList.remove("copied"), 1500);
}

function renderError(tab, event) {
  const element = document.createElement("div");
  element.className = "event error";
  element.textContent = event.message;
  append(tab, element);
  tab.live = null;
  tab.steps = null;
}

function renderStopped(tab, event) {
  const element = document.createElement("div");
  element.className = "event stopped";
  element.textContent = `${event.author} frenó el turno`;
  append(tab, element);
  tab.live = null;
  tab.steps = null;
}

function startTool(tab, event) {
  const details = document.createElement("details");
  details.className = "tool";
  details.dataset.tool = event.name;
  const summary = document.createElement("summary");

  const mark = document.createElement("span");
  mark.className = "icon";
  mark.append(icon(ICON[event.name] || "dot", 13));
  const name = document.createElement("span");
  name.className = "name";
  name.textContent = event.name;
  const detail = document.createElement("span");
  detail.className = "detail";
  const described = describeTool(event.name, event.args, state.workspace);
  if (described.dir) {
    const dir = document.createElement("span");
    dir.className = "dir";
    dir.append(icon("folder", 12), document.createTextNode(described.dir));
    detail.append(dir);
  }
  detail.append(described.text);
  const meta = document.createElement("span");
  meta.className = "meta";
  meta.textContent = "…";
  summary.append(mark, name, detail, meta);

  const body = document.createElement("div");
  body.className = "body";
  const entry = { details, body, meta, output: null, stat: "", group: null };
  if (event.name === "edit") {
    const diff = diffNodes(event.args);
    body.append(...diff.nodes);
    entry.stat = diff.stat;
    meta.textContent = diff.stat;
  } else if (event.name === "write") {
    const diff = contentNodes(event.args);
    body.append(...diff.nodes);
    entry.stat = diff.stat;
    meta.textContent = diff.stat;
  } else {
    const output = document.createElement("pre");
    body.append(output);
    entry.output = output;
  }
  details.append(summary, body);
  const group = steps(tab, event.name);
  entry.group = group;
  group.list.append(details);
  tab.tools[event.id] = entry;
  tab.live = null;
  scroll(tab);
}

/** Los pasos seguidos van en la misma línea, hasta que el agente dice algo. */
function steps(tab, name) {
  if (tab.steps) {
    tab.steps.names.push(name);
    tab.steps.label.textContent = stepsLabel(tab.steps.names);
    return tab.steps;
  }
  const details = document.createElement("details");
  details.className = "steps";
  const summary = document.createElement("summary");
  const caret = document.createElement("span");
  caret.className = "caret";
  caret.textContent = "▶";
  const label = document.createElement("span");
  label.className = "label";
  const elapsed = document.createElement("span");
  elapsed.className = "elapsed";
  summary.append(caret, label, elapsed);
  const list = document.createElement("div");
  list.className = "list";
  details.append(summary, list);
  append(tab, details);
  tab.steps = {
    details,
    list,
    label,
    elapsed,
    names: [name],
    ms: 0,
  };
  tab.steps.label.textContent = stepsLabel(tab.steps.names);
  return tab.steps;
}

function stepsLabel(names) {
  const counts = new Map();
  for (const name of names) {
    const [one, many] = NOUN[name] || [name, name];
    const noun = counts.get(one) || { many, count: 0 };
    noun.count += 1;
    counts.set(one, noun);
  }
  const parts = [...counts].map(
    ([one, { many, count }]) => `${count} ${count === 1 ? one : many}`,
  );
  if (parts.length === 1) return parts[0];
  return parts.slice(0, -1).join(", ") + " y " + parts[parts.length - 1];
}

function elapsed(ms) {
  if (ms < 1000) return `${ms} ms`;
  const seconds = ms / 1000;
  if (seconds < 60) return `${seconds.toFixed(1).replace(".", ",")} s`;
  return `${Math.floor(seconds / 60)}m ${Math.round(seconds % 60)}s`;
}

function streamTool(tab, event) {
  const tool = tab.tools[event.id];
  if (!tool || !tool.output) return;
  tool.output.textContent += event.text;
  scroll(tab);
}

function finishTool(tab, event) {
  const tool = tab.tools[event.id];
  if (!tool) {
    renderError(tab, { message: event.text });
    return;
  }
  tool.details.classList.add(event.failed ? "failed" : "ok");
  const stat = tool.stat ? tool.stat + " · " : "";
  tool.meta.textContent = `${stat}${event.failed ? "error" : "ok"} · ${event.ms} ms`;
  if (tool.output) {
    tool.output.textContent = event.text;
  } else {
    const result = document.createElement("div");
    result.className = "diff-result";
    result.textContent = event.text;
    tool.body.append(result);
  }
  if (tool.group) {
    tool.group.ms += event.ms;
    tool.group.elapsed.textContent = tool.group.ms ? `· ${elapsed(tool.group.ms)}` : "";
    if (event.failed) tool.group.details.classList.add("failed");
  }
  scroll(tab);
}

function diffNodes(raw) {
  const nodes = [];
  const args = parseArgs(raw);
  if (!args) return { nodes: [plainLine(raw)], stat: "" };
  if (args.path) nodes.push(pathLine(args.path));
  let added = 0;
  let removed = 0;
  for (const edit of args.edits || []) {
    for (const line of lines(edit.oldText)) {
      nodes.push(diffLine("- " + line, "del"));
      removed += 1;
    }
    for (const line of lines(edit.newText)) {
      nodes.push(diffLine("+ " + line, "add"));
      added += 1;
    }
  }
  return { nodes, stat: `+${added} -${removed}` };
}

function contentNodes(raw) {
  const nodes = [];
  const args = parseArgs(raw);
  if (!args) return { nodes: [plainLine(raw)], stat: "" };
  if (args.path) nodes.push(pathLine(args.path));
  const content = lines(args.content);
  for (const line of content) nodes.push(diffLine("+ " + line, "add"));
  return { nodes, stat: `+${content.length}` };
}

function pathLine(path) {
  const line = document.createElement("div");
  line.className = "diff-path";
  line.textContent = path;
  return line;
}

function lines(text) {
  const value = (text || "").replace(/\n$/, "");
  return value === "" ? [] : value.split("\n");
}

function diffLine(text, kind) {
  const line = document.createElement("div");
  line.className = "diff-line " + kind;
  line.textContent = text;
  return line;
}

function plainLine(text) {
  const pre = document.createElement("pre");
  pre.textContent = text;
  return pre;
}

/* Attachments */

function pendingFor(id) {
  return pending.get(id) || [];
}

function fileUrl(conversation, name) {
  return `/api/file?conversation=${encodeURIComponent(conversation)}&name=${encodeURIComponent(name)}`;
}

async function uploadImage(conversation, file) {
  const query = `conversation=${encodeURIComponent(conversation)}&name=${encodeURIComponent(file.name || "imagen")}`;
  const uploaded = await request(`/api/upload?${query}`, { method: "POST", body: file });
  return uploaded ? uploaded.name : "";
}

const SHRINK_OVER = 1024 * 1024;
const MAX_SIDE = 1600;

function loadImage(file) {
  return new Promise((resolve, reject) => {
    const url = URL.createObjectURL(file);
    const image = new Image();
    image.onload = () => {
      URL.revokeObjectURL(url);
      resolve(image);
    };
    image.onerror = () => {
      URL.revokeObjectURL(url);
      reject(new Error("no pude leer la imagen"));
    };
    image.src = url;
  });
}

function toBlob(canvas, type) {
  return new Promise((resolve) => canvas.toBlob(resolve, type, 0.85));
}

async function shrink(file) {
  if (file.size < SHRINK_OVER) return file;
  try {
    const image = await loadImage(file);
    const scale = Math.min(1, MAX_SIDE / Math.max(image.width, image.height));
    const canvas = document.createElement("canvas");
    canvas.width = Math.round(image.width * scale);
    canvas.height = Math.round(image.height * scale);
    canvas.getContext("2d").drawImage(image, 0, 0, canvas.width, canvas.height);
    const png = file.type === "image/png";
    const blob = await toBlob(canvas, png ? "image/png" : "image/jpeg");
    if (!blob || blob.size >= file.size) return file;
    const name = png ? file.name : file.name.replace(/\.[^.]+$/, "") + ".jpg";
    return new File([blob], name, { type: blob.type });
  } catch {
    return file;
  }
}

async function attachFiles(files) {
  const conversation = activeId;
  if (!conversation) return;
  if (files.some((file) => !file.type.startsWith("image/"))) notify("por ahora sólo imágenes");
  for (const file of files) {
    if (!file.type.startsWith("image/")) continue;
    const name = await uploadImage(conversation, await shrink(file));
    if (!name) continue;
    const list = pendingFor(conversation);
    list.push({ name, url: fileUrl(conversation, name) });
    pending.set(conversation, list);
  }
  renderPending();
}

function clearPending() {
  pending.delete(activeId);
  renderPending();
}

function renderPending() {
  const list = pendingFor(activeId);
  pendingEl.hidden = list.length === 0;
  pendingEl.replaceChildren();
  list.forEach((item, index) => {
    const thumb = document.createElement("div");
    thumb.className = "thumb";
    const img = document.createElement("img");
    img.src = item.url;
    const remove = document.createElement("button");
    remove.type = "button";
    remove.append(icon("close", 12));
    remove.setAttribute("aria-label", "quitar adjunto");
    remove.onclick = () => {
      list.splice(index, 1);
      renderPending();
    };
    thumb.append(img, remove);
    pendingEl.append(thumb);
  });
}

fileEl.addEventListener("change", async () => {
  await attachFiles([...fileEl.files]);
  fileEl.value = "";
});

inputEl.addEventListener("paste", async (event) => {
  const files = [...(event.clipboardData ? event.clipboardData.files : [])];
  if (!files.length) return;
  event.preventDefault();
  await attachFiles(files);
});

/* Composer */

function grow() {
  inputEl.style.height = "auto";
  inputEl.style.height = Math.min(inputEl.scrollHeight, 240) + "px";
}

inputEl.addEventListener("input", () => {
  grow();
  sayTyping();
});

function sayTyping() {
  const tab = activeId ? tabs.get(activeId) : null;
  if (!tab || isReadOnly(activeId)) return;
  const now = Date.now();
  if (now - (tab.typingSent || 0) < 2000) return;
  tab.typingSent = now;
  api("/api/typing", { conversation: activeId });
}

composerEl.onsubmit = async (formEvent) => {
  formEvent.preventDefault();
  if (!activeId) return;
  const list = pendingFor(activeId);
  const text = inputEl.value.trim();
  if (!text && !list.length) return;
  const images = list.map((item) => item.name);
  const sent = await api("/api/send", { conversation: activeId, text, images });
  if (!sent || !sent.started) return;
  inputEl.value = "";
  grow();
  clearPending();
  const tab = tabs.get(activeId);
  if (tab) tab.follow = true;
  refresh();
};

inputEl.addEventListener("keydown", (keyEvent) => {
  if (!finePointer.matches) return;
  if (keyEvent.key === "Enter" && !keyEvent.shiftKey) {
    keyEvent.preventDefault();
    composerEl.requestSubmit();
  }
});

function cancelTurn() {
  if (!activeId) return;
  api("/api/cancel", { conversation: activeId }).then(refresh);
}

sendEl.onclick = (event) => {
  if (!sendEl.classList.contains("stop")) return;
  event.preventDefault();
  cancelTurn();
};

document.getElementById("cancel").onclick = cancelTurn;

/* Proyectos */

document.getElementById("new-project").onsubmit = async (event) => {
  event.preventDefault();
  const field = document.getElementById("project-name");
  const name = field.value.trim();
  if (!name) return;
  const made = await api("/api/projects", { name });
  if (!made) return;
  field.value = "";
  await refresh();
};

/* Tema */

const lightQuery = matchMedia("(prefers-color-scheme: light)");

function showTheme() {
  const dark = document.documentElement.dataset.theme !== "light";
  const title = dark ? "pasar al tema claro" : "pasar al tema oscuro";
  themeEl.title = title;
  themeEl.setAttribute("aria-label", title);
  themeColor.content = getComputedStyle(document.documentElement).getPropertyValue("--bg").trim();
}

themeEl.onclick = () => {
  const theme = document.documentElement.dataset.theme === "light" ? "dark" : "light";
  document.documentElement.dataset.theme = theme;
  localStorage.setItem("jimmy-theme", theme);
  showTheme();
};

lightQuery.addEventListener("change", () => {
  if (localStorage.getItem("jimmy-theme")) return;
  document.documentElement.dataset.theme = lightQuery.matches ? "light" : "dark";
  showTheme();
});

showTheme();

/* Sidebar drawer */

let drawerInHistory = false;

function hideSidebar() {
  sidebar.classList.remove("open");
  backdrop.hidden = true;
}

function openSidebar() {
  sidebar.classList.add("open");
  backdrop.hidden = false;
  if (drawerInHistory) return;
  history.pushState({ drawer: true }, "", location.href);
  drawerInHistory = true;
}

function closeSidebar() {
  hideSidebar();
  if (!drawerInHistory) return;
  drawerInHistory = false;
  history.back();
}

document.getElementById("menu").onclick = openSidebar;
backdrop.onclick = closeSidebar;

addEventListener("popstate", () => {
  if (!drawerInHistory) return;
  drawerInHistory = false;
  hideSidebar();
});

addEventListener("hashchange", () => {
  const id = decodeURIComponent(location.hash.slice(1));
  if (id === activeId) return;
  if (id && knownTab(id)) openTab(id);
});

function watchOnline() {
  const source = new EventSource("/api/online");
  source.onmessage = (message) => {
    const event = JSON.parse(message.data);
    if (event.event === "online")
      onlineEl.textContent = event.users.length > 1 ? event.users.join(", ") : "";
  };
}

function receiveShared() {
  if (!location.search) return;
  const query = new URLSearchParams(location.search);
  const text = (query.get("text") || "").trim();
  const url = (query.get("url") || "").trim();
  history.replaceState(null, "", location.pathname + location.hash);
  const parts = text.includes(url) ? [text] : [text, url];
  const shared = parts.filter((part) => part).join("\n");
  if (!shared) return;
  if (!activeId || isFiles(activeId) || isReadOnly(activeId)) {
    notify("compartiste algo: abrí una conversación y pegalo");
    return;
  }
  inputEl.value = shared;
  grow();
}

async function main() {
  watchOnline();
  await refresh();
  restore();
  receiveShared();
  setInterval(refresh, 15000);
  addEventListener("visibilitychange", () => {
    if (!document.hidden) refresh();
  });
}

main();
