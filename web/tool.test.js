const test = require("node:test");
const assert = require("node:assert");
const { describeTool } = require("./tool.js");

const work = "/data/workspace";

test("el cd de arranque sale del comando y queda como directorio", () => {
  assert.deepEqual(
    describeTool("bash", `{"command":"cd ${work}/projects/jimmy && git status"}`, work),
    { dir: "projects/jimmy", text: "git status" },
  );
});

test("el directorio se lee aunque el cd silencie el error", () => {
  assert.deepEqual(
    describeTool("bash", `{"command":"cd ${work}/projects/jimmy 2>/dev/null && make test"}`, work),
    { dir: "projects/jimmy", text: "make test" },
  );
});

test("el cd a la raíz del workspace es una tilde", () => {
  assert.deepEqual(describeTool("bash", `{"command":"cd ${work} && ls"}`, work), {
    dir: "~",
    text: "ls",
  });
});

test("el cd fuera del workspace queda entero", () => {
  assert.deepEqual(describeTool("bash", `{"command":"cd /tmp && ls"}`, work), {
    dir: "/tmp",
    text: "ls",
  });
});

test("varios cd seguidos dejan el último, que es donde corre el resto", () => {
  assert.deepEqual(
    describeTool("bash", `{"command":"cd /tmp && cd ${work}/projects/axe && git log"}`, work),
    { dir: "projects/axe", text: "git log" },
  );
});

test("un comando sin cd va entero y sin directorio", () => {
  assert.deepEqual(describeTool("bash", '{"command":"node --test web/"}', work), {
    dir: "",
    text: "node --test web/",
  });
});

test("un cd suelto, sin nada encadenado, no se toca", () => {
  assert.deepEqual(describeTool("bash", '{"command":"cd /tmp"}', work), {
    dir: "",
    text: "cd /tmp",
  });
});

test("sin workspace el directorio queda como vino", () => {
  assert.deepEqual(
    describeTool("bash", `{"command":"cd ${work}/projects/jimmy && git status"}`, ""),
    { dir: `${work}/projects/jimmy`, text: "git status" },
  );
});

test("el path de read, write y edit también se acorta", () => {
  const args = `{"path":"${work}/projects/jimmy/src/web.rs"}`;
  for (const name of ["read", "write", "edit"]) {
    assert.deepEqual(describeTool(name, args, work), {
      dir: "",
      text: "projects/jimmy/src/web.rs",
    });
  }
});

test("los args ilegibles se muestran crudos", () => {
  assert.deepEqual(describeTool("bash", "no es json", work), { dir: "", text: "no es json" });
});

test("una tool sin campos propios se muestra como json", () => {
  assert.deepEqual(describeTool("rara", '{"a":1}', work), { dir: "", text: '{"a":1}' });
});
