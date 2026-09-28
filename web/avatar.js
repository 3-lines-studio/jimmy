const WAITING_MS = 1500;

const AVATAR_STATES = ["focused", "working", "waiting", "done", "error", "sleeping"];

const avatars = new Set();

function avatarState(project) {
  if (project.error) return "error";
  if (project.done) return "done";
  if (project.working) return "working";
  if (project.waiting) return "waiting";
  if (project.active) return "focused";
  if (!project.open) return "sleeping";
  return "idle";
}

function avatarBot(name) {
  const el = document.createElement("span");
  el.className = "avatar bot";
  el.style.setProperty("--h", projectHue(name));
  el.style.setProperty("--blink", (2.8 + Math.random() * 3.4).toFixed(2) + "s");
  el.style.setProperty("--delay", (-Math.random() * 5).toFixed(2) + "s");
  el.style.setProperty("--hop", (-Math.random() * 1.1).toFixed(2) + "s");
  el.style.setProperty("--gaze", (-Math.random() * 11).toFixed(2) + "s");
  el.innerHTML = '<span class="eye"><i></i></span><span class="eye"><i></i></span>';
  avatars.add(el);
  return el;
}

function paintAvatar(el, state) {
  for (const name of AVATAR_STATES) el.classList.toggle(name, name === state);
}

let pointerX = 0;
let pointerY = 0;
let queued = 0;

function follow() {
  queued = 0;
  for (const el of avatars) {
    if (!el.isConnected) {
      avatars.delete(el);
      continue;
    }
    const box = el.getBoundingClientRect();
    if (!box.width) continue;
    const dx = pointerX - (box.left + box.width / 2);
    const dy = pointerY - (box.top + box.height / 2);
    const distance = Math.hypot(dx, dy) || 1;
    const reach = Math.min(1, distance / 320);
    const shift = box.width * (el.classList.contains("focused") ? 0.3 : 0.17) * reach;
    el.style.setProperty("--dx", ((dx / distance) * shift).toFixed(2) + "px");
    el.style.setProperty("--dy", ((dy / distance) * shift * 0.7).toFixed(2) + "px");
  }
}

function watchPointer() {
  if (matchMedia("(prefers-reduced-motion: reduce)").matches) return;
  window.addEventListener(
    "pointermove",
    (event) => {
      pointerX = event.clientX;
      pointerY = event.clientY;
      if (!queued) queued = requestAnimationFrame(follow);
    },
    { passive: true },
  );
}

if (typeof module !== "undefined")
  module.exports = { avatarState, avatarBot, paintAvatar, watchPointer, WAITING_MS };
