const test = require("node:test");
const assert = require("node:assert");
const {
  projectColor,
  projectHue,
  projectInitials,
  projectTitle,
  projectSize,
} = require("./project.js");

test("el color de un proyecto sale de su nombre, así que no cambia entre recargas", () => {
  assert.equal(projectColor("jimmy"), projectColor("jimmy"));
  assert.match(projectColor("jimmy"), /^hsl\(\d+ 55% 42%\)$/);
});

test("dos proyectos distintos caen en colores distintos", () => {
  assert.notEqual(projectColor("jimmy"), projectColor("bifrost"));
  assert.notEqual(projectColor("heimdall"), projectColor("ken"));
});

test("la pill lleva las tres primeras letras del proyecto", () => {
  assert.equal(projectInitials("jimmy"), "jim");
  assert.equal(projectInitials("go"), "go");
});

test("el tono del proyecto da la vuelta al círculo cromático", () => {
  assert.equal(projectHue("jimmy"), projectHue("jimmy"));
  assert.match(String(projectHue("bifrost")), /^\d{1,3}$/);
  assert.ok(projectHue("heimdall") < 360);
});

test("el nombre del proyecto se muestra con la primera en mayúscula", () => {
  assert.equal(projectTitle("jimmy"), "Jimmy");
  assert.equal(projectTitle("ñandú"), "Ñandú");
  assert.equal(projectTitle("ken-viejo"), "Ken-viejo");
});

test("el peso del proyecto baja a KB cuando no llega al mega", () => {
  assert.equal(projectSize(45000), "45 KB");
  assert.equal(projectSize(8192), "8 KB");
  assert.equal(projectSize(999), "1 KB");
  assert.equal(projectSize(0), "0 KB");
});

test("el peso del proyecto va en MB y en GB, sin decimales de más", () => {
  assert.equal(projectSize(1000000), "1 MB");
  assert.equal(projectSize(716800), "717 KB");
  assert.equal(projectSize(3149824), "3 MB");
  assert.equal(projectSize(1208803328), "1,2 GB");
  assert.equal(projectSize(2000000000), "2 GB");
});
