const test = require("node:test");
const assert = require("node:assert");
const { agendaWhen, agendaDuration, agendaMoment, agendaAgo, agendaEvery, agendaFailed } =
  require("./agenda.js");

test("el horario se lee de un vistazo", () => {
  assert.equal(agendaWhen({ at: "05:00" }), "todos los días · 05:00");
  assert.equal(agendaWhen({ every: "30m" }), "cada 30 min");
  assert.equal(agendaWhen({ every: "6h" }), "cada 6 h");
  assert.equal(agendaWhen({ every: "2d" }), "cada 2 d");
  assert.equal(agendaWhen({ when: "2026-09-14T15:00" }), "una vez · 14/09 a las 15:00");
  assert.equal(agendaWhen({}), "sin horario");
});

test("una duración larga no se muestra en segundos", () => {
  assert.equal(agendaDuration(320), "320 ms");
  assert.equal(agendaDuration(38_000), "38 s");
  assert.equal(agendaDuration(125_000), "2 min");
  assert.equal(agendaDuration(7_300_000), "2 h");
});

test("la corrida dice qué día fue", () => {
  const now = new Date(2026, 8, 26, 18, 0, 0);
  const ts = Math.round(new Date(2026, 8, 26, 5, 0, 0).getTime() / 1000);
  const ayer = Math.round(new Date(2026, 8, 25, 22, 0, 0).getTime() / 1000);
  assert.equal(agendaMoment({ date: "2026-09-26", ts }, now), "hoy 05:00");
  assert.equal(agendaMoment({ date: "2026-09-25", ts: ayer }, now), "ayer 22:00");
  assert.equal(agendaMoment({ date: "2026-09-14", ts }, now), "14/09 05:00");
});

test("hace cuánto corrió", () => {
  const now = new Date(2026, 8, 26, 18, 0, 0);
  const seconds = Math.round(now.getTime() / 1000);
  assert.equal(agendaAgo(seconds - 30, now), "hace 30 s");
  assert.equal(agendaAgo(seconds - 720, now), "hace 12 min");
  assert.equal(agendaAgo(seconds - 61_200, now), "hace 17 h");
  assert.equal(agendaAgo(seconds - 259_200, now), "hace 3 d");
});

test("el badge cuenta sólo lo que falló y no está pausado", () => {
  const tasks = [
    { name: "ok", paused: false, runs: [{ ok: true }] },
    { name: "rota", paused: false, runs: [{ ok: false }] },
    { name: "callada", paused: true, runs: [{ ok: false }] },
    { name: "nueva", paused: false, runs: [] },
  ];
  assert.equal(agendaFailed(tasks), 1);
  assert.equal(agendaFailed([]), 0);
});
