const test = require("node:test");
const assert = require("node:assert");
const { vaultKey, vaultScope, vaultCount, vaultMask, vaultAction, vaultClock } = require("./vault.js");

test("el entorno se nombra proyecto y entorno", () => {
  assert.equal(vaultKey({ project: "picsel", env: "dev" }), "picsel/dev");
});

test("el alcance de un token se lee sin abrirlo", () => {
  assert.equal(vaultScope({ project: "*", env: "dev" }), "*/dev");
  assert.equal(vaultScope({ admin: true, project: "_", env: "_" }), "admin");
});

test("las claves se cuentan en singular y en plural", () => {
  assert.equal(vaultCount(0), "0 claves");
  assert.equal(vaultCount(1), "1 clave");
  assert.equal(vaultCount(12), "12 claves");
});

test("un valor tapado no dice cuánto mide", () => {
  assert.equal(vaultMask("abc"), "•••");
  assert.equal(vaultMask("x".repeat(40)), "•".repeat(16));
  assert.equal(vaultMask(""), "");
});

test("lo que hizo jimmy se lee en castellano", () => {
  assert.equal(vaultAction("get-secrets"), "leyó");
  assert.equal(vaultAction("token-create"), "creó un token");
  assert.equal(vaultAction("otra-cosa"), "otra-cosa");
});

test("la fecha del audit se lee de un vistazo", () => {
  const ts = Math.round(new Date(2026, 8, 26, 5, 7, 0).getTime() / 1000);
  assert.equal(vaultClock(ts), "26/09 05:07");
});
