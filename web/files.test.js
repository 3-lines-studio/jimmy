const test = require("node:test");
const assert = require("node:assert");
const { extension, size } = require("./files.js");

test("la terminación es lo que el resaltador lee como lenguaje", () => {
  assert.equal(extension("web.rs"), "rs");
  assert.equal(extension("README.MD"), "md");
  assert.equal(extension("app.test.js"), "js");
  assert.equal(extension("logo.png"), "png");
});

test("un archivo sin punto no tiene terminación", () => {
  assert.equal(extension("Makefile"), "");
  assert.equal(extension("LICENSE"), "");
});

test("el punto adelante es el nombre, no la terminación", () => {
  assert.equal(extension(".env"), "");
});

test("el tamaño se redondea a la unidad que se lee de un vistazo", () => {
  assert.equal(size(0), "0 B");
  assert.equal(size(999), "999 B");
  assert.equal(size(1024), "1 KB");
  assert.equal(size(600 * 1024), "600 KB");
  assert.equal(size(1024 * 1024), "1,0 MB");
  assert.equal(size(1536 * 1024), "1,5 MB");
});
