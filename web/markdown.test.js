const test = require("node:test");
const assert = require("node:assert");
const { markdown, inline } = require("./markdown.js");

test("un párrafo partido en varias líneas es un solo párrafo", () => {
  assert.equal(markdown("una linea\nque sigue\n\ny otro"), "<p>una linea que sigue</p><p>y otro</p>");
});

test("los bloques pegados sin línea en blanco arrancan otro bloque", () => {
  assert.equal(markdown("texto\n# título"), "<p>texto</p><h2>título</h2>");
  assert.equal(markdown("texto\n- uno"), "<p>texto</p><ul><li>uno</li></ul>");
});

test("las listas se anidan por indentación", () => {
  assert.equal(
    markdown("- uno\n  - sub\n  - sub dos\n- dos"),
    "<ul><li>uno<ul><li>sub</li><li>sub dos</li></ul></li><li>dos</li></ul>",
  );
});

test("una lista se corta cuando el texto vuelve al margen", () => {
  assert.equal(markdown("- uno\n- dos\n\ntexto"), "<ul><li>uno</li><li>dos</li></ul><p>texto</p>");
});

test("las listas ordenadas se numeran y pueden mezclarse", () => {
  assert.equal(markdown("1. uno\n2. dos"), "<ol><li>uno</li><li>dos</li></ol>");
  assert.equal(
    markdown("- uno\n1. dos"),
    "<ul><li>uno</li></ul><ol><li>dos</li></ol>",
  );
});

test("la continuación de un ítem va dentro del ítem", () => {
  assert.equal(
    markdown("- uno\n  y sigue\n- dos"),
    "<ul><li>uno y sigue</li><li>dos</li></ul>",
  );
});

test("un ítem puede llevar un bloque de código", () => {
  assert.equal(
    markdown("- corré esto:\n\n  ```bash\n  ls -la\n  ```"),
    '<ul><li><p>corré esto:</p><pre><code>ls -la</code></pre></li></ul>',
  );
});

test("las tareas salen con su casilla", () => {
  assert.equal(
    markdown("- [ ] pendiente\n- [x] hecho"),
    '<ul><li><input type="checkbox" disabled> pendiente</li><li><input type="checkbox" disabled checked> hecho</li></ul>',
  );
});

test("los headings bajan a h2 y sueltan los # de cierre", () => {
  assert.equal(markdown("# uno"), "<h2>uno</h2>");
  assert.equal(markdown("### tres ##"), "<h4>tres</h4>");
  assert.equal(markdown("#hashtag"), "<p>#hashtag</p>");
});

test("la regla horizontal es un hr", () => {
  assert.equal(markdown("texto\n\n---\n\ntexto"), "<p>texto</p><hr><p>texto</p>");
  assert.equal(markdown("***"), "<hr>");
});

test("la cita agrupa sus líneas y bloquea adentro", () => {
  assert.equal(
    markdown("> una cita\n> con dos líneas"),
    "<blockquote><p>una cita con dos líneas</p></blockquote>",
  );
  assert.equal(markdown("> - uno\n> - dos"), "<blockquote><ul><li>uno</li><li>dos</li></ul></blockquote>");
});

test("el código conserva sus líneas y su lenguaje", () => {
  assert.equal(
    markdown("```rust\nlet x = 1;\n```"),
    '<pre><code><span class="tok-keyword">let</span> x = <span class="tok-number">1</span>;</code></pre>',
  );
  assert.equal(markdown("~~~\nplano\n~~~"), "<pre><code>plano</code></pre>");
  assert.equal(
    markdown("```rust title=x\nlet x = 1;\n```"),
    '<pre><code><span class="tok-keyword">let</span> x = <span class="tok-number">1</span>;</code></pre>',
  );
});

test("un bloque de código abierto a mitad del stream no rompe", () => {
  assert.equal(markdown("```bash\nls"), '<pre><code>ls</code></pre>');
});

test("las tablas llevan encabezado, alineación y celdas", () => {
  assert.equal(
    markdown("| a | b |\n| :-- | --: |\n| 1 | 2 |"),
    '<div class="table-wrap"><table><thead><tr><th>a</th><th class="right">b</th></tr></thead><tbody><tr><td>1</td><td class="right">2</td></tr></tbody></table></div>',
  );
});

test("una fila con pipes y sin regla no es tabla", () => {
  assert.equal(markdown("| a | b |"), "<p>| a | b |</p>");
});

test("el énfasis cruza asteriscos y se anida", () => {
  assert.equal(inline("**hola *mundo* chau**"), "<strong>hola <em>mundo</em> chau</strong>");
  assert.equal(inline("***los dos***"), "<strong><em>los dos</em></strong>");
  assert.equal(inline("__fuerte__ y _cursiva_"), "<strong>fuerte</strong> y <em>cursiva</em>");
});

test("los asteriscos sueltos quedan", () => {
  assert.equal(inline("2 * 3 * 4"), "2 * 3 * 4");
  assert.equal(inline("snake_case_name"), "snake_case_name");
  assert.equal(inline("a * b"), "a * b");
});

test("el código en línea admite cualquier cantidad de backticks", () => {
  assert.equal(inline("usa `ls`"), "usa <code>ls</code>");
  assert.equal(inline("usa `` `ls` ``"), "usa <code>`ls`</code>");
  assert.equal(inline("con < y & adentro"), "con &lt; y &amp; adentro");
});

test("el tachado", () => {
  assert.equal(inline("~~viejo~~"), "<del>viejo</del>");
});

test("los links y las URLs peladas", () => {
  assert.equal(
    inline("[texto](https://ejemplo.com/a)"),
    '<a href="https://ejemplo.com/a" target="_blank" rel="noreferrer">texto</a>',
  );
  assert.equal(
    inline("mirá https://ejemplo.com/x."),
    'mirá <a href="https://ejemplo.com/x" target="_blank" rel="noreferrer">https://ejemplo.com/x</a>.',
  );
  assert.equal(
    inline("(https://ejemplo.com/x)"),
    '(<a href="https://ejemplo.com/x" target="_blank" rel="noreferrer">https://ejemplo.com/x</a>)',
  );
});

test("el escape de puntuación", () => {
  assert.equal(inline("un \\* literal"), "un * literal");
  assert.equal(inline("\\`no es código\\`"), "`no es código`");
});

test("el HTML del texto se escapa", () => {
  assert.equal(markdown("<script>alert(1)</script>"), "<p>&lt;script&gt;alert(1)&lt;/script&gt;</p>");
});

test("un link con esquema peligroso queda literal", () => {
  assert.equal(inline("[a](javascript:alert(1))"), "[a](javascript:alert(1))");
});
