const ICON = { read: "▤", write: "✎", edit: "±", bash: "$", search: "⌕", fetch: "⇩" };

let state = { projects: [] };
const tabs = new Map();
let activeKey = null;

const projectsEl = document.getElementById("projects");
const tabsEl = document.getElementById("tabs");
const panesEl = document.getElementById("panes");
const placeholderEl = document.getElementById("placeholder");
const sidebar = document.getElementById("sidebar");
const backdrop = document.getElementById("backdrop");
const composerEl = document.getElementById("composer");
const inputEl = document.getElementById("input");
const tabActionsEl = document.getElementById("tab-actions");
const viewersEl = document.getElementById("viewers");
const cancelEl = document.getElementById("cancel");
const pendingEl = document.getElementById("pending");
const fileEl = document.getElementById("file");
const searchEl = document.getElementById("search");
let pending = new Map();
let searchTimer = null;

async function api(path, body) {
  let response;
  try {
    response = await fetch(path, {
      method: body === undefined ? "GET" : "POST",
      headers: body === undefined ? {} : { "Content-Type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    return null;
  }
  if (response.status === 401) {
    location.href = "/login";
    return null;
  }
  return response.json().catch(() => null);
}

function conversationByKey(key) {
  for (const project of state.projects) {
    for (const conversation of project.conversations) {
      if (conversation.key === key) return conversation;
    }
  }
  return null;
}

function titleOf(key) {
  const conversation = conversationByKey(key);
  if (!conversation) return key;
  return conversation.title || conversation.key;
}

async function refresh() {
  const data = await api("/api/state");
  if (!data) return;
  state = data;
  const live = new Set();
  for (const project of state.projects) {
    for (const conversation of project.conversations) live.add(conversation.key);
  }
  for (const key of [...tabs.keys()]) {
    if (!live.has(key)) closeTab(key);
  }
  if (searchEl.value.trim().length < 2) renderSidebar();
  renderTabs();
  renderActions();
  updateTitle();
}

function renderSidebar() {
  projectsEl.replaceChildren();
  for (const project of state.projects) {
    const header = document.createElement("div");
    header.className = "project";
    const name = document.createElement("span");
    name.className = "name";
    name.textContent = project.name;
    name.title = project.path;
    header.append(name);
    if (project.name !== "general") {
      const remove = document.createElement("button");
      remove.className = "icon-btn";
      remove.textContent = "×";
      remove.title = "borrar proyecto";
      remove.onclick = (event) => {
        event.stopPropagation();
        deleteProject(project);
      };
      header.append(remove);
    }
    projectsEl.append(header);

    for (const conversation of project.conversations) {
      const item = document.createElement("div");
      item.className = "conversation" + (conversation.key === activeKey ? " active" : "");
      item.dataset.conversation = conversation.key;
      const dot = document.createElement("span");
      dot.className = "dot" + (conversation.running ? " running" : "");
      const title = document.createElement("span");
      title.className = "title";
      title.textContent = conversation.title || conversation.key;
      item.append(dot, title);
      if (!conversation.read_only) {
        const renameBtn = document.createElement("button");
        renameBtn.className = "icon-btn";
        renameBtn.dataset.action = "rename";
        renameBtn.textContent = "✎";
        renameBtn.title = "renombrar";
        renameBtn.onclick = (event) => {
          event.stopPropagation();
          rename(title, conversation);
        };
        const remove = document.createElement("button");
        remove.className = "icon-btn";
        remove.dataset.action = "delete";
        remove.textContent = "×";
        remove.title = "borrar conversación";
        remove.onclick = (event) => {
          event.stopPropagation();
          deleteConversation(conversation);
        };
        item.append(renameBtn, remove);
      }
      item.onclick = () => {
        openTab(conversation.key);
        closeSidebar();
      };
      projectsEl.append(item);
    }

    if (project.name !== "general") {
      const create = document.createElement("div");
      create.className = "conversation create";
      create.textContent = "+ conversación";
      create.onclick = async () => {
        const made = await api("/api/conversations", { project: project.name });
        await refresh();
        if (made && made.key) openTab(made.key);
        closeSidebar();
      };
      projectsEl.append(create);
    }
  }

  const createProject = document.createElement("div");
  createProject.className = "conversation create";
  createProject.id = "new-project";
  createProject.textContent = "+ proyecto";
  createProject.onclick = async () => {
    const name = prompt("Nombre del proyecto (una carpeta nueva en el workspace):");
    if (!name) return;
    const made = await api("/api/projects", { name });
    if (!made || made.error) alert(made ? made.error : "no pude crear el proyecto");
    await refresh();
  };
  projectsEl.append(createProject);
}

async function deleteConversation(conversation) {
  const name = conversation.title || conversation.key;
  if (!confirm(`¿Borrar la conversación "${name}"? Se pierde el historial.`)) return;
  const done = await api("/api/delete-conversation", { conversation: conversation.key });
  if (!done || done.error) {
    alert(done && done.error ? done.error : "no pude borrarla");
    return;
  }
  closeTab(conversation.key);
  await refresh();
}

async function deleteProject(project) {
  if (!confirm(`¿Borrar el proyecto "${project.name}"?`)) return;
  const done = await api("/api/delete-project", { project: project.name });
  if (!done || done.error) {
    alert(done && done.error ? done.error : "no pude borrarlo");
    return;
  }
  await refresh();
}

/* Buscar en el historial */

searchEl.addEventListener("input", () => {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(runSearch, 250);
});

async function runSearch() {
  const query = searchEl.value.trim();
  if (query.length < 2) {
    renderSidebar();
    return;
  }
  const data = await api(`/api/search?q=${encodeURIComponent(query)}`);
  renderResults((data && data.results) || []);
}

function renderResults(results) {
  projectsEl.replaceChildren();
  if (!results.length) {
    const empty = document.createElement("div");
    empty.className = "result-empty";
    empty.textContent = "nada encontrado";
    projectsEl.append(empty);
    return;
  }
  for (const hit of results) {
    const item = document.createElement("div");
    item.className = "result";
    const title = document.createElement("div");
    title.className = "result-title";
    title.textContent = `${hit.title} · ${hit.project}`;
    const snippet = document.createElement("div");
    snippet.className = "result-snippet";
    snippet.textContent = `${hit.role === "user" ? "vos" : "jimmy"}: ${hit.snippet}`;
    item.append(title, snippet);
    item.onclick = () => {
      openTab(hit.conversation);
      closeSidebar();
    };
    projectsEl.append(item);
  }
}

function rename(title, conversation) {
  const input = document.createElement("input");
  input.className = "rename";
  input.value = conversation.title;
  title.replaceWith(input);
  input.focus();
  input.select();

  const commit = async () => {
    const value = input.value.trim();
    input.replaceWith(title);
    if (!value || value === conversation.title) return;
    await api("/api/rename", { conversation: conversation.key, title: value });
    await refresh();
  };

  input.onblur = commit;
  input.onkeydown = (event) => {
    if (event.key === "Enter") input.blur();
    if (event.key === "Escape") {
      input.value = conversation.title;
      input.blur();
    }
  };
}

function openTab(key) {
  if (!tabs.has(key)) createTab(key);
  activate(key);
}

function createTab(key) {
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
    key,
    viewers: [],
    pane,
    transcript,
    jump,
    stream: null,
    live: null,
    tools: {},
    follow: true,
    synced: false,
    attention: null,
  };
  tabs.set(key, tab);

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
  tab.stream = new EventSource(`/api/stream?conversation=${encodeURIComponent(tab.key)}`);
  tab.stream.onopen = () => {
    tab.transcript.replaceChildren();
    tab.live = null;
    tab.tools = {};
    tab.synced = false;
  };
  tab.stream.onmessage = (message) => render(tab, JSON.parse(message.data));
}

function activate(key) {
  activeKey = key;
  for (const tab of tabs.values()) tab.pane.hidden = tab.key !== key;
  placeholderEl.hidden = tabs.size > 0;
  const tab = tabs.get(key);
  if (tab) tab.attention = null;
  renderSidebar();
  renderTabs();
  renderActions();
  renderPending();
  updateTitle();
  inputEl.focus();
}

function closeTab(key) {
  const tab = tabs.get(key);
  if (!tab) return;
  if (tab.stream) tab.stream.close();
  tab.pane.remove();
  tabs.delete(key);
  if (activeKey !== key) {
    renderTabs();
    return;
  }
  const next = tabs.keys().next().value;
  if (next) {
    activate(next);
  } else {
    activeKey = null;
    placeholderEl.hidden = false;
    renderTabs();
    renderActions();
  }
}

function renderTabs() {
  tabsEl.replaceChildren();
  for (const tab of tabs.values()) {
    const item = document.createElement("div");
    item.className = "tab" + (tab.key === activeKey ? " active" : "");
    const dot = document.createElement("span");
    const conversation = conversationByKey(tab.key);
    const status = conversation && conversation.running ? "running" : tab.attention || "";
    dot.className = "dot" + (status ? " " + status : "");
    const title = document.createElement("span");
    title.className = "title";
    title.textContent = titleOf(tab.key);
    const close = document.createElement("button");
    close.className = "close";
    close.textContent = "×";
    close.title = "cerrar pestaña";
    close.onclick = (event) => {
      event.stopPropagation();
      closeTab(tab.key);
    };
    item.append(dot, title, close);
    item.onclick = () => activate(tab.key);
    tabsEl.append(item);
  }
}

function renderActions() {
  const tab = activeKey ? tabs.get(activeKey) : null;
  const conversation = activeKey ? conversationByKey(activeKey) : null;
  tabActionsEl.hidden = !tab;
  composerEl.hidden = !conversation || conversation.read_only;
  if (!tab || !conversation) return;
  viewersEl.textContent =
    tab.viewers.length > 1 ? tab.viewers.join(", ") + " mirando" : "";
  cancelEl.hidden = !conversation.running;
}

function updateTitle() {
  const count = [...tabs.values()].filter((tab) => tab.attention).length;
  document.title = count ? `(${count}) Jimmy` : "Jimmy";
}

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
      if (tab.key !== activeKey) {
        tab.attention = "error";
        renderTabs();
        updateTitle();
      }
      break;
    case "done":
      if (tab.key !== activeKey) {
        tab.attention = "done";
        renderTabs();
        updateTitle();
      }
      break;
    case "presence":
      tab.viewers = event.users || [];
      if (tab.key === activeKey) renderActions();
      break;
    case "synced":
      tab.synced = true;
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
  const who = document.createElement("span");
  who.className = "who";
  who.textContent = "vos";
  element.append(who, document.createTextNode(event.text));
  append(tab, element);
  tab.live = null;
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
  scroll(tab);
}

function renderError(tab, event) {
  const element = document.createElement("div");
  element.className = "event error";
  element.textContent = event.message;
  append(tab, element);
  tab.live = null;
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
  const entry = { details, body, meta, output: null, stat: "" };
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
    details.open = true;
  }
  details.append(summary, body);
  append(tab, details);
  tab.tools[event.id] = entry;
  tab.live = null;
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
  scroll(tab);
}

function toolDetail(name, raw) {
  const args = parseArgs(raw);
  if (!args) return raw;
  if (name === "bash") return args.command || "";
  if (name === "read" || name === "write" || name === "edit") return args.path || "";
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

/* Composer */

function grow() {
  inputEl.style.height = "auto";
  inputEl.style.height = Math.min(inputEl.scrollHeight, 240) + "px";
}

inputEl.addEventListener("input", grow);

composerEl.onsubmit = async (event) => {
  event.preventDefault();
  if (!activeKey) return;
  const list = pendingFor(activeKey);
  const text = inputEl.value.trim();
  if (!text && !list.length) return;
  const images = list.map((item) => item.url);
  inputEl.value = "";
  grow();
  clearPending();
  const tab = tabs.get(activeKey);
  if (tab) tab.follow = true;
  await api("/api/send", { conversation: activeKey, text, images });
};

inputEl.addEventListener("keydown", (event) => {
  if (event.key === "Enter" && !event.shiftKey) {
    event.preventDefault();
    composerEl.requestSubmit();
  }
});

/* Adjuntos */

function pendingFor(key) {
  return pending.get(key) || [];
}

function readAsDataURL(file) {
  return new Promise((resolve) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result);
    reader.readAsDataURL(file);
  });
}

async function attachFiles(files) {
  if (!activeKey) return;
  for (const file of files) {
    if (!file.type.startsWith("image/")) continue;
    const url = await readAsDataURL(file);
    const list = pendingFor(activeKey);
    list.push({ url, preview: URL.createObjectURL(file) });
    pending.set(activeKey, list);
  }
  renderPending();
}

function clearPending() {
  for (const item of pendingFor(activeKey)) URL.revokeObjectURL(item.preview);
  pending.delete(activeKey);
  renderPending();
}

function renderPending() {
  const list = pendingFor(activeKey);
  pendingEl.hidden = list.length === 0;
  pendingEl.replaceChildren();
  list.forEach((item, index) => {
    const thumb = document.createElement("div");
    thumb.className = "thumb";
    const img = document.createElement("img");
    img.src = item.preview;
    const remove = document.createElement("button");
    remove.type = "button";
    remove.textContent = "×";
    remove.onclick = () => {
      URL.revokeObjectURL(item.preview);
      list.splice(index, 1);
      renderPending();
    };
    thumb.append(img, remove);
    pendingEl.append(thumb);
  });
}

fileEl.onchange = async () => {
  await attachFiles([...fileEl.files]);
  fileEl.value = "";
};

inputEl.addEventListener("paste", async (event) => {
  const files = [...(event.clipboardData ? event.clipboardData.files : [])];
  if (!files.length) return;
  event.preventDefault();
  await attachFiles(files);
});

cancelEl.onclick = async () => {
  if (!activeKey) return;
  await api("/api/cancel", { conversation: activeKey });
};

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

document.getElementById("logout").onsubmit = async (event) => {
  event.preventDefault();
  await fetch("/api/logout", { method: "POST" });
  location.href = "/login";
};

async function main() {
  await refresh();
  setInterval(refresh, 2000);
}

main();
