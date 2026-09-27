const test = require("node:test");
const assert = require("node:assert");
const { projectColor, projectInitials, projectLetter, projectTitle } = require("./project.js");

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

test("el círculo lleva la inicial, en mayúscula", () => {
  assert.equal(projectLetter("jimmy"), "J");
  assert.equal(projectLetter("bifrost"), "B");
  assert.equal(projectLetter("ñandú"), "Ñ");
});

test("el nombre del proyecto se muestra con la primera en mayúscula", () => {
  assert.equal(projectTitle("jimmy"), "Jimmy");
  assert.equal(projectTitle("ñandú"), "Ñandú");
  assert.equal(projectTitle("ken-viejo"), "Ken-viejo");
});
