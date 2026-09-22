const ICON = { read: "▤", write: "✎", edit: "±", bash: "$", search: "⌕", fetch: "↓", browse: "◇" };
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

const projectsEl = document.getElementById("projects");
const tabsEl = document.getElementById("tabs");
const panesEl = document.getElementById("panes");
const placeholderEl = document.getElementById("placeholder");
const sidebar = document.getElementById("sidebar");
const backdrop = document.getElementById("backdrop");
const tabActionsEl = document.getElementById("tab-actions");
const viewersEl = document.getElementById("viewers");
const composerEl = document.getElementById("composer");
const inputEl = document.getElementById("input");
const searchEl = document.getElementById("search");
const pendingEl = document.getElementById("pending");
const fileEl = document.getElementById("file");
const cancelEl = document.getElementById("cancel");
const toastEl = document.getElementById("toast");
const finePointer = matchMedia("(hover: hover) and (pointer: fine)");

async function api(path, body) {
  let response;
  try {
    response = await fetch(path, {
      method: body === undefined ? "GET" : "POST",
      headers: body === undefined ? {} : { "Content-Type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
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

let toastTimer = null;

function notify(text) {
  toastEl.textContent = text;
  toastEl.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => {
    toastEl.hidden = true;
  }, 6000);
}

function conversationById(id) {
  for (const project of state.projects) {
    for (const conversation of project.conversations) {
      if (conversation.key === id) return conversation;
    }
  }
  return null;
}

function titleOf(id) {
  const conversation = conversationById(id);
  return conversation ? conversation.title || conversation.key : id;
}

function isRunning(id) {
  const conversation = conversationById(id);
  return conversation ? conversation.running : false;
}

function isReadOnly(id) {
  const conversation = conversationById(id);
  return conversation ? conversation.read_only : false;
}

async function refresh() {
  const data = await api("/api/state");
  if (!data) return;
  const before = JSON.stringify(state.projects);
  state = data;
  const live = new Set();
  for (const project of state.projects) {
    for (const conversation of project.conversations) live.add(conversation.key);
  }
  for (const id of [...tabs.keys()]) {
    if (!live.has(id)) closeTab(id);
  }
  if (searchEl.value.trim().length < 2) {
    renderSidebar();
  } else if (searchDirty || JSON.stringify(state.projects) !== before) {
    searchDirty = false;
    runSearch();
  }
  renderTabs();
  renderActions();
  updateTitle();
}

/* Sidebar */

const groupEls = new Map();

function projectEl(project) {
  const group = {
    el: document.createElement("div"),
    project,
    name: document.createElement("span"),
    list: document.createElement("div"),
    conversations: new Map(),
  };
  group.el.className = "group";
  group.list.className = "threads";
  const header = document.createElement("div");
  header.className = "project toggle";
  header.onclick = () => toggleProject(group.project.name);
  const caret = document.createElement("span");
  caret.className = "caret";
  caret.textContent = "▾";
  group.name.className = "name";
  const remove = iconButton("×", "quitar proyecto", () => deleteProject(group.project), "danger");
  header.append(caret, group.name, remove);
  group.create = document.createElement("div");
  group.create.className = "conversation create";
  group.create.textContent = "+ conversación";
  group.create.onclick = async () => {
    const made = await api("/api/conversations", { project: group.project.name });
    await refresh();
    if (made && made.key) openTab(made.key);
    closeSidebar();
  };
  group.el.append(header, group.list);
  return group;
}

function iconButton(label, title, action, kind = "") {
  const button = document.createElement("button");
  button.className = "icon-btn" + (kind ? " " + kind : "");
  button.textContent = label;
  button.title = title;
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
      iconButton("✎", "renombrar", () => rename(item)),
      iconButton("×", "borrar conversación", () => deleteConversation(item.conversation), "danger"),
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
    const seen = new Set();
    let lastItem = null;
    for (const conversation of project.conversations) {
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
    place(group.list, group.create, lastItem);
    place(projectsEl, group.el, lastGroup);
    lastGroup = group.el;
  }
  for (const [name, group] of groupEls) {
    if (alive.has(name)) continue;
    group.el.remove();
    groupEls.delete(name);
  }
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
  for (const conversation of project.conversations) closeTab(conversation.key);
  await api("/api/delete-project", { project: project.name });
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

function openTab(id) {
  if (!tabs.has(id)) createTab(id);
  activate(id);
}

function createTab(id) {
  const pane = document.createElement("div");
  pane.className = "pane";
  const transcript = document.createElement("div");
  transcript.className = "transcript";
  const jump = document.createElement("button");
  jump.className = "jump";
  jump.textContent = "ir al final ↓";
  jump.hidden = true;
  pane.append(transcript, jump);
  panesEl.append(pane);

  const tab = {
    id,
    pane,
    transcript,
    jump,
    stream: null,
    live: null,
    steps: null,
    tools: {},
    follow: true,
    viewers: [],
    find: null,
    synced: false,
    attention: null,
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

  subscribe(tab);
  return tab;
}

function subscribe(tab) {
  if (tab.stream) tab.stream.close();
  tab.stream = new EventSource(`/api/stream?conversation=${encodeURIComponent(tab.id)}`);
  tab.stream.onopen = () => {
    tab.transcript.replaceChildren();
    tab.live = null;
    tab.steps = null;
    tab.tools = {};
    tab.synced = false;
  };
  tab.stream.onmessage = (message) => render(tab, JSON.parse(message.data));
}

function activate(id) {
  activeId = id;
  for (const tab of tabs.values()) tab.pane.hidden = tab.id !== id;
  placeholderEl.hidden = tabs.size > 0;
  const tab = tabs.get(id);
  if (tab) tab.attention = null;
  if (searchEl.value.trim().length < 2) renderSidebar();
  renderTabs();
  renderActions();
  renderPending();
  updateTitle();
  if (finePointer.matches) inputEl.focus();
}

function closeTab(id) {
  const tab = tabs.get(id);
  if (!tab) return;
  if (tab.stream) tab.stream.close();
  tab.pane.remove();
  tab.item.el.remove();
  pending.delete(id);
  tabs.delete(id);
  if (activeId !== id) {
    renderTabs();
    return;
  }
  const next = tabs.keys().next().value;
  if (next) {
    activate(next);
  } else {
    activeId = null;
    placeholderEl.hidden = false;
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
  close.textContent = "×";
  close.title = "cerrar pestaña";
  close.onclick = (event) => {
    event.stopPropagation();
    closeTab(tab.id);
  };
  item.el.append(item.dot, item.title, close);
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
  tabActionsEl.hidden = !tab;
  composerEl.hidden = !tab || isReadOnly(activeId);
  cancelEl.hidden = !tab || !isRunning(activeId);
  if (tab)
    viewersEl.textContent = tab.viewers.length > 1 ? tab.viewers.join(", ") + " mirando" : "";
}

function updateTitle() {
  const count = [...tabs.values()].filter((tab) => tab.attention).length;
  document.title = count ? `(${count}) Jimmy` : "Jimmy";
}

/* Rendering */

function render(tab, event) {
  switch (event.event) {
    case "user":
      renderUser(tab, event);
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
    case "synced":
      tab.synced = true;
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
  tab.transcript.append(element);
  scroll(tab);
}

function renderUser(tab, event) {
  const element = document.createElement("div");
  element.className = "event user";
  element.textContent = event.text;
  append(tab, element);
  tab.live = null;
  tab.steps = null;
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
  tab.live = null;
  tab.steps = null;
  scroll(tab);
}

function renderError(tab, event) {
  const element = document.createElement("div");
  element.className = "event error";
  element.textContent = event.message;
  append(tab, element);
  tab.live = null;
  tab.steps = null;
}

function startTool(tab, event) {
  const details = document.createElement("details");
  details.className = "tool";
  details.dataset.tool = event.name;
  const summary = document.createElement("summary");

  const icon = document.createElement("span");
  icon.className = "icon";
  icon.textContent = ICON[event.name] || "•";
  const name = document.createElement("span");
  name.className = "name";
  name.textContent = event.name;
  const detail = document.createElement("span");
  detail.className = "detail";
  detail.textContent = toolDetail(event.name, event.args);
  const meta = document.createElement("span");
  meta.className = "meta";
  meta.textContent = "…";
  summary.append(icon, name, detail, meta);

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
  if (event.failed) tool.details.open = true;
  if (tool.group) {
    tool.group.ms += event.ms;
    tool.group.elapsed.textContent = tool.group.ms ? `· ${elapsed(tool.group.ms)}` : "";
    if (event.failed) {
      tool.group.details.classList.add("failed");
      tool.group.details.open = true;
    }
  }
  scroll(tab);
}

function toolDetail(name, raw) {
  const args = parseArgs(raw);
  if (!args) return raw;
  if (name === "bash") return args.command || "";
  if (name === "read" || name === "write" || name === "edit") return args.path || "";
  if (name === "search") return args.query || "";
  if (name === "fetch") return args.url || "";
  if (name === "browse") return args.url || `${(args.steps || []).length} pasos`;
  return JSON.stringify(args);
}

function parseArgs(raw) {
  try {
    return JSON.parse(raw || "{}");
  } catch {
    return null;
  }
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

/* Syntax highlighting */

const KEYWORDS = {
  js: "await async break case catch class const continue default delete do else export extends finally for from function if import in instanceof let new of return static super switch this throw try typeof var void while yield true false null undefined",
  rust: "as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait true type unsafe use where while",
  go: "break case chan const continue default defer else fallthrough for func go goto if import interface map package range return select struct switch type var true false nil",
  python: "and as assert async await break class continue def del elif else except False finally for from global if import in is lambda None nonlocal not or pass raise return True try while with yield",
  bash: "if then else elif fi for while do done case esac function return export local readonly source echo cd set unset exit",
  sql: "select from where insert into values update set delete create table drop alter join left right inner outer group by order having limit offset and or not null as distinct",
  json: "true false null",
  toml: "true false",
  yaml: "true false null yes no on off",
};

const SPECS = new Map(
  [
    ["js", { line: ["//"], block: [["/*", "*/"]], strings: ["'", '"', "`"], keywords: KEYWORDS.js }],
    ["rust", { line: ["//"], block: [["/*", "*/"]], keywords: KEYWORDS.rust }],
    ["go", { line: ["//"], block: [["/*", "*/"]], strings: ['"', "'", "`"], keywords: KEYWORDS.go }],
    ["python", { line: ["#"], strings: ['"', "'"], keywords: KEYWORDS.python }],
    ["bash", { line: ["#"], keywords: KEYWORDS.bash }],
    ["json", { strings: ['"'], keywords: KEYWORDS.json }],
    ["css", { block: [["/*", "*/"]] }],
    ["html", { block: [["<!--", "-->"]] }],
    ["sql", { line: ["--"], block: [["/*", "*/"]], keywords: KEYWORDS.sql }],
    ["toml", { line: ["#"], keywords: KEYWORDS.toml }],
    ["yaml", { line: ["#"], keywords: KEYWORDS.yaml }],
  ].map(([name, spec]) => [
    name,
    {
      line: spec.line || [],
      block: spec.block || [],
      strings: spec.strings || ['"', "'"],
      keywords: new Set((spec.keywords || "").split(" ").filter(Boolean)),
    },
  ]),
);

const ALIASES = {
  javascript: "js",
  jsx: "js",
  ts: "js",
  typescript: "js",
  tsx: "js",
  mjs: "js",
  json5: "json",
  rs: "rust",
  py: "python",
  sh: "bash",
  shell: "bash",
  zsh: "bash",
  console: "bash",
  xml: "html",
  svg: "html",
  yml: "yaml",
};

function token(kind, text) {
  return `<span class="tok-${kind}">${escapeHtml(text)}</span>`;
}

function highlight(code, language) {
  const name = ALIASES[language] || language;
  if (name === "diff" || name === "patch") return highlightDiff(code);
  const spec = SPECS.get(name);
  if (!spec) return escapeHtml(code);

  let out = "";
  let i = 0;
  const n = code.length;
  while (i < n) {
    const rest = code.slice(i);

    const lineMark = spec.line.find((mark) => rest.startsWith(mark));
    if (lineMark) {
      const end = code.indexOf("\n", i);
      const stop = end === -1 ? n : end;
      out += token("comment", code.slice(i, stop));
      i = stop;
      continue;
    }

    const block = spec.block.find(([open]) => rest.startsWith(open));
    if (block) {
      const [open, close] = block;
      const end = code.indexOf(close, i + open.length);
      const stop = end === -1 ? n : end + close.length;
      out += token("comment", code.slice(i, stop));
      i = stop;
      continue;
    }

    const char = code[i];
    if (spec.strings.includes(char)) {
      let j = i + 1;
      while (j < n) {
        if (code[j] === "\\") {
          j += 2;
          continue;
        }
        if (code[j] === char) {
          j += 1;
          break;
        }
        if (char !== "`" && code[j] === "\n") break;
        j += 1;
      }
      out += token("string", code.slice(i, j));
      i = j;
      continue;
    }

    if (/[0-9]/.test(char) && !/[A-Za-z_]/.test(code[i - 1] || "")) {
      let j = i;
      while (j < n && /[0-9a-fA-FxX._]/.test(code[j])) j += 1;
      out += token("number", code.slice(i, j));
      i = j;
      continue;
    }

    if (/[A-Za-z_$]/.test(char)) {
      let j = i;
      while (j < n && /[A-Za-z0-9_$]/.test(code[j])) j += 1;
      const word = code.slice(i, j);
      out += spec.keywords.has(word) ? token("keyword", word) : escapeHtml(word);
      i = j;
      continue;
    }

    out += escapeHtml(char);
    i += 1;
  }
  return out;
}

function highlightDiff(code) {
  return code
    .split("\n")
    .map((line) => {
      if (line.startsWith("+")) return token("add", line);
      if (line.startsWith("-")) return token("del", line);
      if (line.startsWith("@@")) return token("meta", line);
      return escapeHtml(line);
    })
    .join("\n");
}

/* Markdown subset */

function escapeHtml(text) {
  return text.replace(
    /[&<>"']/g,
    (char) =>
      ({
        "&": "&amp;",
        "<": "&lt;",
        ">": "&gt;",
        '"': "&quot;",
        "'": "&#39;",
      })[char],
  );
}

function inline(text) {
  const codes = [];
  let out = escapeHtml(text).replace(/`([^`]+)`/g, (_, code) => {
    codes.push(code);
    return `\u0000${codes.length - 1}\u0000`;
  });
  out = out
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/(^|[^*])\*([^*\s][^*]*)\*/g, "$1<em>$2</em>")
    .replace(
      /\[([^\]]+)\]\((https?:[^)\s]+)\)/g,
      '<a href="$2" target="_blank" rel="noreferrer">$1</a>',
    );
  return out.replace(/\u0000(\d+)\u0000/g, (_, index) => `<code>${codes[index]}</code>`);
}

function splitRow(line) {
  const cells = [];
  let current = "";
  const body = line.trim().replace(/^\|/, "").replace(/\|$/, "");
  for (let index = 0; index < body.length; index += 1) {
    if (body[index] === "\\" && body[index + 1] === "|") {
      current += "|";
      index += 1;
      continue;
    }
    if (body[index] === "|") {
      cells.push(current.trim());
      current = "";
      continue;
    }
    current += body[index];
  }
  cells.push(current.trim());
  return cells;
}

function alignOf(mark) {
  const left = mark.startsWith(":");
  const right = mark.endsWith(":");
  if (left && right) return "center";
  return right ? "right" : "";
}

// A GitHub table is a header row, a `---` rule that also carries the
// alignment, and as many body rows as keep having a pipe.
function tableAt(lines, start) {
  const header = lines[start];
  const rule = lines[start + 1];
  if (!header || !rule || !header.includes("|")) return null;
  const marks = splitRow(rule);
  if (!marks.length || !marks.every((mark) => /^:?-+:?$/.test(mark))) return null;
  const heads = splitRow(header);
  if (heads.length !== marks.length) return null;

  const rows = [];
  let end = start + 2;
  while (end < lines.length && lines[end].includes("|") && lines[end].trim() !== "") {
    rows.push(splitRow(lines[end]));
    end += 1;
  }

  const aligns = marks.map(alignOf);
  const cell = (tag, text, column) => {
    const align = aligns[column];
    return `<${tag}${align ? ` class="${align}"` : ""}>${inline(text)}</${tag}>`;
  };

  let html = '<div class="table-wrap"><table><thead><tr>';
  heads.forEach((head, column) => {
    html += cell("th", head, column);
  });
  html += "</tr></thead><tbody>";
  for (const row of rows) {
    html += "<tr>";
    heads.forEach((_, column) => {
      html += cell("td", row[column] || "", column);
    });
    html += "</tr>";
  }
  html += "</tbody></table></div>";
  return { html, end };
}

function markdown(source) {
  const lines = source.split("\n");
  let html = "";
  let list = null;
  let code = null;
  let language = "";

  const closeList = () => {
    if (list) {
      html += `</${list}>`;
      list = null;
    }
  };

  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];
    if (line.trimStart().startsWith("```")) {
      if (code === null) {
        closeList();
        code = [];
        language = line.trim().slice(3).trim().toLowerCase();
      } else {
        html += `<pre><code>${highlight(code.join("\n"), language)}</code></pre>`;
        code = null;
        language = "";
      }
      continue;
    }
    if (code !== null) {
      code.push(line);
      continue;
    }

    const table = tableAt(lines, index);
    if (table) {
      closeList();
      html += table.html;
      index = table.end - 1;
      continue;
    }

    const heading = line.match(/^(#{1,6})\s+(.*)$/);
    if (heading) {
      closeList();
      const level = Math.min(heading[1].length + 1, 4);
      html += `<h${level}>${inline(heading[2])}</h${level}>`;
      continue;
    }

    const bullet = line.match(/^\s*[-*]\s+(.*)$/);
    if (bullet) {
      if (list !== "ul") {
        closeList();
        html += "<ul>";
        list = "ul";
      }
      html += `<li>${inline(bullet[1])}</li>`;
      continue;
    }

    const ordered = line.match(/^\s*\d+\.\s+(.*)$/);
    if (ordered) {
      if (list !== "ol") {
        closeList();
        html += "<ol>";
        list = "ol";
      }
      html += `<li>${inline(ordered[1])}</li>`;
      continue;
    }

    if (line.trim() === "") {
      closeList();
      continue;
    }

    closeList();
    html += `<p>${inline(line)}</p>`;
  }

  closeList();
  if (code !== null) html += `<pre><code>${highlight(code.join("\n"), language)}</code></pre>`;
  return html;
}

/* Attachments */

function pendingFor(id) {
  return pending.get(id) || [];
}

function readImage(file) {
  return new Promise((resolve) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result);
    reader.onerror = () => resolve(null);
    reader.readAsDataURL(file);
  });
}

async function attachFiles(files) {
  if (!activeId) return;
  for (const file of files) {
    if (!file.type.startsWith("image/")) continue;
    const url = await readImage(file);
    if (!url) continue;
    const list = pendingFor(activeId);
    list.push({ name: file.name || "imagen", url });
    pending.set(activeId, list);
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
    remove.textContent = "×";
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

inputEl.addEventListener("input", grow);

composerEl.onsubmit = async (formEvent) => {
  formEvent.preventDefault();
  if (!activeId) return;
  const list = pendingFor(activeId);
  const text = inputEl.value.trim();
  if (!text && !list.length) return;
  const images = list.map((item) => item.url);
  const sent = await api("/api/send", { conversation: activeId, text, images });
  if (!sent || !sent.started) return;
  inputEl.value = "";
  grow();
  clearPending();
  const tab = tabs.get(activeId);
  if (tab) tab.follow = true;
};

inputEl.addEventListener("keydown", (keyEvent) => {
  if (keyEvent.key === "Enter" && !keyEvent.shiftKey) {
    keyEvent.preventDefault();
    composerEl.requestSubmit();
  }
});

document.getElementById("cancel").onclick = () => {
  if (activeId) api("/api/cancel", { conversation: activeId });
};

/* Proyectos */

document.getElementById("new-project").onclick = async () => {
  const field = document.getElementById("project-name");
  const name = field.value.trim();
  if (!name) return;
  const made = await api("/api/projects", { name });
  if (!made) return;
  field.value = "";
  await refresh();
};

/* Tema */

const themeEl = document.getElementById("theme");
const lightQuery = matchMedia("(prefers-color-scheme: light)");

function showTheme() {
  const dark = document.documentElement.dataset.theme !== "light";
  themeEl.textContent = dark ? "☀" : "☾";
  themeEl.title = dark ? "pasar al tema claro" : "pasar al tema oscuro";
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

function openSidebar() {
  sidebar.classList.add("open");
  backdrop.hidden = false;
}

function closeSidebar() {
  sidebar.classList.remove("open");
  backdrop.hidden = true;
}

document.getElementById("menu").onclick = openSidebar;
backdrop.onclick = closeSidebar;

async function main() {
  await refresh();
  setInterval(refresh, 3000);
}

main();
