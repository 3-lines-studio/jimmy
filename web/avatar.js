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
  const face = projectFace(name);
  el.className = `avatar bot body-${face.body} eyes-${face.eyes}`;
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
let boxes = [];
let measured = 0;

const GAZE_REACH = 240;
const MEASURE_EVERY = 400;

function measure() {
  boxes = [];
  for (const el of avatars) {
    if (!el.isConnected) {
      avatars.delete(el);
      continue;
    }
    const box = el.getBoundingClientRect();
    if (!box.width) continue;
    boxes.push({ el, x: box.left + box.width / 2, y: box.top + box.height / 2, w: box.width });
  }
  measured = performance.now();
}

function aim(el, width, dx, dy, distance) {
  const reach = Math.min(1, distance / 320);
  const shift = width * (el.classList.contains("focused") ? 0.3 : 0.17) * reach;
  return [
    ((dx / (distance || 1)) * shift).toFixed(1) + "px",
    ((dy / (distance || 1)) * shift * 0.7).toFixed(1) + "px",
  ];
}

function follow() {
  queued = 0;
  if (boxes.length !== avatars.size || performance.now() - measured > MEASURE_EVERY) measure();
  for (const box of boxes) {
    const dx = pointerX - box.x;
    const dy = pointerY - box.y;
    const distance = Math.hypot(dx, dy);
    const [x, y] = distance > GAZE_REACH ? ["0px", "0px"] : aim(box.el, box.w, dx, dy, distance);
    if (box.el.style.getPropertyValue("--dx") === x) continue;
    box.el.style.setProperty("--dx", x);
    box.el.style.setProperty("--dy", y);
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
  addEventListener("scroll", measure, { passive: true, capture: true });
  addEventListener("resize", measure, { passive: true });
}

if (typeof module !== "undefined")
  module.exports = { avatarState, avatarBot, paintAvatar, watchPointer, WAITING_MS };
