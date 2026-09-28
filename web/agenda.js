/* La agenda: un tab con las tareas programadas y lo que contestó cada corrida.
   El estado y el horario salen del servidor, que lee `state/schedule/`. */

const AGENDA = "agenda";

let agendaTasks = [];
const agendaOpen = new Set();
const agendaPending = new Map();

function isAgenda(id) {
  return id === AGENDA;
}

function agendaEvery(value) {
  const text = String(value).trim();
  const unit = text.slice(-1);
  const amount = text.slice(0, -1).trim();
  const name = { s: "s", m: "min", h: "h", d: "d" }[unit] || unit;
  return `cada ${amount} ${name}`;
}

function agendaWhen(task) {
  if (task.when) {
    const [date, time] = String(task.when).replace(" ", "T").split("T");
    const [, month, day] = date.split("-");
    return `una vez · ${day}/${month} a las ${time}`;
  }
  if (task.at) return `todos los días · ${task.at}`;
  if (task.every) return agendaEvery(task.every);
  return "sin horario";
}

function agendaDuration(ms) {
  if (ms < 1000) return `${ms} ms`;
  const seconds = Math.round(ms / 1000);
  if (seconds < 60) return `${seconds} s`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} min`;
  return `${Math.round(minutes / 60)} h`;
}

function agendaDate(date) {
  const pad = (value) => String(value).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

function agendaDay(day, now) {
  if (day === agendaDate(now)) return "hoy";
  if (day === agendaDate(new Date(now.getTime() - 86_400_000))) return "ayer";
  const [, month, date] = String(day).split("-");
  return `${date}/${month}`;
}

function agendaClock(ts) {
  const date = new Date(ts * 1000);
  const pad = (value) => String(value).padStart(2, "0");
  return `${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

function agendaMoment(run, now) {
  return `${agendaDay(run.date, now)} ${agendaClock(run.ts)}`;
}

function agendaAgo(ts, now) {
  const seconds = Math.max(0, Math.round(now.getTime() / 1000 - ts));
  if (seconds < 60) return `hace ${seconds} s`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `hace ${minutes} min`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `hace ${hours} h`;
  return `hace ${Math.round(hours / 24)} d`;
}

function agendaState(task) {
  if (task.paused) return "";
  if (agendaPending.has(task.name)) return "running";
  const last = task.runs[0];
  if (!last) return "";
  return last.ok ? "done" : "error";
}

function agendaUnread(tasks) {
  return tasks.reduce((total, task) => total + (task.unread || 0), 0);
}

function agendaFailed(tasks) {
  return tasks.filter((task) => task.unread > 0 && task.runs[0] && !task.runs[0].ok).length;
}

function agendaMark(tasks) {
  if (!agendaFailed(tasks)) return "unread";
  return "error";
}

function agendaLast(task, now) {
  if (agendaPending.has(task.name)) return "corriendo…";
  const last = task.runs[0];
  if (!last) return "nunca corrió";
  return `${agendaMoment(last, now)} · ${agendaDuration(last.ms)}`;
}

function createAgendaTab(id) {
  const pane = document.createElement("div");
  pane.className = "pane agenda";
  const inner = document.createElement("div");
  inner.className = "agenda-inner";
  pane.append(inner);
  panesEl.append(pane);

  const tab = { id, pane, inner, agenda: true, viewers: [] };
  tab.item = tabEl(tab);
  tabs.set(id, tab);
  renderAgendaPane();
  return tab;
}

async function loadAgenda() {
  const data = await api("/api/agenda");
  if (!data) return;
  agendaTasks = data.tasks || [];
  for (const [name, previous] of [...agendaPending]) {
    const task = agendaTasks.find((candidate) => candidate.name === name);
    const last = task && task.runs[0] ? task.runs[0].ts : 0;
    if (!task || last !== previous) agendaPending.delete(name);
  }
  renderAgenda();
}

function renderAgenda() {
  const unread = agendaUnread(agendaTasks);
  const badge = document.getElementById("agenda-badge");
  badge.hidden = unread === 0;
  badge.textContent = unread;
  badge.classList.toggle("error", agendaFailed(agendaTasks) > 0);
  const tab = tabs.get(AGENDA);
  if (tab) tab.attention = unread ? agendaMark(agendaTasks) : null;
  renderAgendaPane();
  renderTabs();
}

function renderAgendaPane() {
  const tab = tabs.get(AGENDA);
  if (!tab) return;
  const now = new Date();
  tab.inner.replaceChildren();
  if (!agendaTasks.length) {
    const empty = document.createElement("div");
    empty.className = "agenda-empty";
    empty.textContent = "No hay tareas agendadas.";
    tab.inner.append(empty);
    return;
  }
  for (const task of agendaTasks) tab.inner.append(agendaTaskEl(task, now));
  if (agendaUnread(agendaTasks) > 0) tab.inner.append(agendaFootEl());
}

function agendaFootEl() {
  const foot = document.createElement("div");
  foot.className = "agenda-foot";
  const button = document.createElement("button");
  button.textContent = "Marcar todo leído";
  button.onclick = async () => {
    if (await api("/api/agenda/read", {})) await loadAgenda();
  };
  foot.append(button);
  return foot;
}

function agendaTaskEl(task, now) {
  const el = document.createElement("article");
  el.className = "task" + (agendaOpen.has(task.name) ? " open" : "");
  const head = document.createElement("div");
  head.className = "task-head";

  const title = document.createElement("div");
  title.className = "task-title";
  const dot = document.createElement("span");
  const state = agendaState(task);
  dot.className = "dot" + (state ? " " + state : "");
  const name = document.createElement("span");
  name.className = "task-name";
  name.textContent = task.name;
  const when = document.createElement("span");
  when.className = "task-when";
  when.textContent = agendaWhen(task);
  title.append(dot, name, when);
  if (task.unread > 0) {
    const pill = document.createElement("span");
    pill.className = "pill unread";
    pill.textContent = task.unread;
    title.append(pill);
  }
  if (task.paused) {
    const pill = document.createElement("span");
    pill.className = "pill";
    pill.textContent = "pausada";
    title.append(pill);
  }

  const meta = document.createElement("div");
  meta.className = "task-meta";
  const last = document.createElement("span");
  last.className = "task-last";
  last.textContent = agendaLast(task, now);
  meta.append(last);
  if (!task.paused) {
    meta.append(iconButton("play", "Correr ahora", () => runAgendaTask(task)));
  }
  meta.append(
    iconButton(task.paused ? "play" : "pause", task.paused ? "Reanudar" : "Pausar", () =>
      pauseAgendaTask(task),
    ),
  );

  head.append(title, meta);
  head.onclick = () => {
    if (agendaOpen.has(task.name)) {
      agendaOpen.delete(task.name);
    } else {
      agendaOpen.add(task.name);
      readAgendaTask(task);
    }
    renderAgendaPane();
  };
  el.append(head);
  if (agendaOpen.has(task.name)) el.append(agendaRunsEl(task, now));
  return el;
}

function agendaRunsEl(task, now) {
  const runs = document.createElement("div");
  runs.className = "task-runs";
  if (!task.runs.length) {
    const empty = document.createElement("div");
    empty.className = "run-empty";
    empty.textContent = "Todavía no corrió.";
    runs.append(empty);
    return runs;
  }
  for (const run of task.runs) {
    const row = document.createElement("div");
    row.className = "run" + (run.ok ? "" : " failed");
    const time = document.createElement("span");
    time.className = "run-time";
    time.textContent = agendaMoment(run, now);
    time.title = `${agendaAgo(run.ts, now)} · ${agendaDuration(run.ms)}`;
    const text = document.createElement("div");
    text.className = "run-text";
    if (run.text.trim()) {
      text.innerHTML = markdown(run.text);
    } else {
      text.textContent = "sin novedades";
    }
    row.append(time, text);
    runs.append(row);
  }
  return runs;
}

async function runAgendaTask(task) {  agendaPending.set(task.name, task.runs[0] ? task.runs[0].ts : 0);
  renderAgendaPane();
  if (!(await api("/api/agenda/run", { name: task.name }))) {
    agendaPending.delete(task.name);
    return renderAgendaPane();
  }
  setTimeout(loadAgenda, 5000);
}

async function pauseAgendaTask(task) {
  if (!(await api("/api/agenda/pause", { name: task.name, paused: !task.paused }))) return;
  await loadAgenda();
}

async function readAgendaTask(task) {
  if (!task.unread) return;
  task.unread = 0;
  renderAgenda();
  await api("/api/agenda/read", { name: task.name });
}

if (typeof module !== "undefined") {
  module.exports = {
    agendaWhen,
    agendaDuration,
    agendaMoment,
    agendaAgo,
    agendaEvery,
    agendaUnread,
    agendaFailed,
    agendaMark,
  };
}
