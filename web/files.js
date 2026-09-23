/* El explorador de archivos de un proyecto: un tab más, de sólo lectura. El
   árbol se pide por carpeta, cuando se abre, y el archivo se muestra al
   tocarlo. */

const FILES = "files:";

function isFiles(id) {
  return typeof id === "string" && id.startsWith(FILES);
}

function filesProject(id) {
  return id.slice(FILES.length);
}

function createFilesTab(id) {
  const pane = document.createElement("div");
  pane.className = "pane files";
  const tree = document.createElement("div");
  tree.className = "files-tree";
  const view = document.createElement("div");
  view.className = "files-view";
  const head = document.createElement("div");
  head.className = "files-head";
  head.hidden = true;
  const back = document.createElement("button");
  back.className = "files-back";
  back.append(icon("left", 15), label("archivos"));
  const path = label("", "files-path");
  head.append(back, path);
  const body = document.createElement("div");
  body.className = "files-body";
  body.append(note("Elegí un archivo para leerlo."));
  view.append(head, body);
  pane.append(tree, view);
  panesEl.append(pane);

  const tab = {
    id,
    pane,
    tree,
    body,
    head,
    path,
    files: true,
    viewers: [],
    project: filesProject(id),
    file: "",
    row: null,
  };
  tab.item = tabEl(tab);
  tabs.set(id, tab);

  back.onclick = () => pane.classList.remove("showing");
  list(tab, tree, "");
  return tab;
}

/// Un nivel del árbol. Cada carpeta se pide una vez, cuando se abre.
async function list(tab, parent, path) {
  const query =
    "/api/tree?project=" + encodeURIComponent(tab.project) + "&path=" + encodeURIComponent(path);
  const data = await api(query);
  if (!data) {
    parent.append(note("no pude leer esa carpeta"));
    return;
  }
  if (!data.entries.length) {
    parent.append(note("vacía"));
    return;
  }
  for (const entry of data.entries) parent.append(row(tab, join(path, entry.name), entry));
}

function row(tab, path, entry) {
  if (entry.dir) return folder(tab, path, entry);
  const element = document.createElement("button");
  element.className = "files-row";
  element.append(icon("file", 15), label(entry.name), label(size(entry.size), "files-size"));
  element.onclick = () => openFile(tab, path, entry, element);
  return element;
}

function folder(tab, path, entry) {
  const box = document.createElement("details");
  box.className = "files-dir";
  const summary = document.createElement("summary");
  summary.append(icon("folder", 15), label(entry.name));
  const kids = document.createElement("div");
  kids.className = "files-kids";
  box.append(summary, kids);
  box.ontoggle = () => {
    if (!box.open || kids.childElementCount) return;
    list(tab, kids, path);
  };
  return box;
}

async function openFile(tab, path, entry, element) {
  if (tab.row) tab.row.classList.remove("active");
  tab.row = element;
  element.classList.add("active");
  tab.file = path;
  tab.path.textContent = path;
  tab.head.hidden = false;
  tab.pane.classList.add("showing");
  tab.body.replaceChildren(note("trayendo…"));

  if (entry.kind === "image") {
    const image = document.createElement("img");
    image.className = "files-image";
    image.alt = entry.name;
    image.src = rawUrl(tab.project, path);
    tab.body.replaceChildren(image);
    return;
  }

  const response = await fetch(rawUrl(tab.project, path));
  if (tab.file !== path) return;
  if (!response.ok) {
    tab.body.replaceChildren(note("no pude abrir ese archivo"));
    return;
  }
  const text = await response.text();
  if (tab.file !== path) return;
  if (!(response.headers.get("content-type") || "").startsWith("text/")) {
    tab.body.replaceChildren(note("es un archivo binario: no hay nada que mostrar"));
    return;
  }
  tab.body.replaceChildren(entry.kind === "markdown" ? markdownOf(text) : codeOf(text));
  if (response.headers.get("x-truncated"))
    tab.body.append(note("se ve sólo el principio: el archivo pasa los 512 KB"));
}

function codeOf(text) {
  const box = document.createElement("div");
  box.className = "files-code";
  const pre = document.createElement("pre");
  pre.textContent = text;
  box.append(pre, copyButton(text, "copiar el archivo"));
  return box;
}

function markdownOf(text) {
  const box = document.createElement("div");
  box.className = "event assistant files-markdown";
  box.innerHTML = markdown(text);
  decorate(box, text);
  return box;
}

function rawUrl(project, path) {
  return "/api/raw?project=" + encodeURIComponent(project) + "&path=" + encodeURIComponent(path);
}

function join(path, name) {
  return path ? path + "/" + name : name;
}

function note(text) {
  const element = document.createElement("div");
  element.className = "files-note";
  element.textContent = text;
  return element;
}

function label(value, className = "files-label") {
  const element = document.createElement("span");
  element.className = className;
  element.textContent = value;
  return element;
}

function size(bytes) {
  if (bytes < 1024) return bytes + " B";
  if (bytes < 1024 * 1024) return Math.round(bytes / 1024) + " KB";
  return (bytes / 1024 / 1024).toFixed(1).replace(".", ",") + " MB";
}
