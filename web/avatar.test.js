const test = require("node:test");
const assert = require("node:assert");
const { avatarState } = require("./avatar.js");

const idle = { open: true };

test("un proyecto donde nada pasó queda en su lugar", () => {
  assert.equal(avatarState(idle), "idle");
});

test("un proyecto sin ninguna tab abierta duerme", () => {
  assert.equal(avatarState({ open: false }), "sleeping");
});

test("la tab activa mira al frente", () => {
  assert.equal(avatarState({ ...idle, active: true }), "focused");
});

test("mientras el agente trabaja, salta", () => {
  assert.equal(avatarState({ ...idle, working: true }), "working");
});

test("si el agente se quedó esperando, se hamaca", () => {
  assert.equal(avatarState({ ...idle, waiting: true }), "waiting");
});

test("el turno que terminó sin que lo mires festeja", () => {
  assert.equal(avatarState({ ...idle, done: true }), "done");
});

test("el error gana sobre todo lo demás", () => {
  assert.equal(
    avatarState({ ...idle, error: true, done: true, working: true, waiting: true, active: true }),
    "error",
  );
});

test("trabajar gana sobre esperar y sobre la tab activa", () => {
  assert.equal(avatarState({ ...idle, working: true, waiting: true, active: true }), "working");
});

test("el que terminó gana sobre el que sigue trabajando", () => {
  assert.equal(avatarState({ ...idle, done: true, working: true }), "done");
});

test("un proyecto sin tabs abiertas pero corriendo no duerme", () => {
  assert.equal(avatarState({ open: false, working: true }), "working");
});
