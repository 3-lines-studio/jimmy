/* Markdown */

const PUNCTUATION = "\\`*_{}[]()#+-.!<>~|";

function escapeHtml(text) {
  return text.replace(
    /[&<>"']/g,
    (char) =>
      ({
        "&": "&amp;",
        "<": "&lt;",
        ">": "&gt;",
        '"': "&quot;",
        "'": "&#39;",
      })[char],
  );
}

function isAlnum(char) {
  return char !== undefined && /[A-Za-z0-9]/.test(char);
}

function countRun(chars, from, char) {
  let count = 0;
  while (chars[from + count] === char) count += 1;
  return count;
}

function findRun(chars, from, char, length) {
  let index = from;
  while (index <= chars.length - length) {
    if (chars[index] !== char) {
      index += 1;
      continue;
    }
    const run = countRun(chars, index, char);
    if (run === length) return index;
    index += run;
  }
  return -1;
}

function findDouble(chars, from, char) {
  return findRun(chars, from, char, 2);
}

function findItalic(chars, from, char) {
  for (let index = from; index < chars.length; index += 1) {
    if (chars[index] !== char) continue;
    if (chars[index - 1] === char || /\s/.test(chars[index - 1])) continue;
    const after = chars[index + 1];
    if (after !== undefined && (isAlnum(after) || after === char)) continue;
    return index;
  }
  return -1;
}

function codeSpan(chars, index) {
  const run = countRun(chars, index, "`");
  const close = findRun(chars, index + run, "`", run);
  if (close === -1) return null;
  let body = chars.slice(index + run, close);
  if (body.length > 2 && body[0] === " " && body[body.length - 1] === " ") body = body.slice(1, -1);
  return { html: `<code>${escapeHtml(body.join(""))}</code>`, end: close + run };
}

function emphasis(chars, index, char) {
  const open = index === 0 || !isAlnum(chars[index - 1]);
  if (!open) return null;
  const run = countRun(chars, index, char);
  const next = chars[index + 1];
  if (run >= 2 && next !== undefined && !/\s/.test(next)) {
    const length = Math.min(run, 3);
    const close = findRun(chars, index + length, char, length);
    if (close === -1) return null;
    const body = inline(chars.slice(index + length, close).join(""));
    const html =
      length === 3 ? `<strong><em>${body}</em></strong>` : `<strong>${body}</strong>`;
    return { html, end: close + length };
  }
  if (run === 1 && next !== undefined && !/\s/.test(next)) {
    const close = findItalic(chars, index + 1, char);
    if (close === -1) return null;
    return { html: `<em>${inline(chars.slice(index + 1, close).join(""))}</em>`, end: close + 1 };
  }
  return null;
}

function strikethrough(chars, index) {
  const close = findDouble(chars, index + 2, "~");
  if (close === -1) return null;
  return { html: `<del>${inline(chars.slice(index + 2, close).join(""))}</del>`, end: close + 2 };
}

function safeUrl(url) {
  return /^(https?:\/\/|mailto:|\/|#)/.test(url);
}

function linkAt(chars, from) {
  let close = -1;
  for (let index = from + 1; index < chars.length; index += 1) {
    if (chars[index] === "[") return null;
    if (chars[index] === "]") {
      close = index;
      break;
    }
  }
  if (close === -1 || chars[close + 1] !== "(") return null;
  let end = -1;
  for (let index = close + 2; index < chars.length; index += 1) {
    if (chars[index] === ")") {
      end = index;
      break;
    }
  }
  if (end === -1) return null;
  const url = chars.slice(close + 2, end).join("").trim();
  if (!safeUrl(url)) return null;
  return { label: chars.slice(from + 1, close).join(""), url, end: end + 1 };
}

function count(text, char) {
  return text.split(char).length - 1;
}

function trimUrl(url) {
  let out = url;
  while (/[.,;:!?]$/.test(out)) out = out.slice(0, -1);
  while (out.endsWith(")") && count(out, "(") < count(out, ")")) out = out.slice(0, -1);
  return out;
}

function bareUrl(chars, index) {
  const match = /^https?:\/\/[^\s<>"'`]+/.exec(chars.slice(index).join(""));
  if (!match) return null;
  const url = trimUrl(match[0]);
  if (url === "") return null;
  return {
    html: `<a href="${escapeHtml(url)}" target="_blank" rel="noreferrer">${escapeHtml(url)}</a>`,
    end: index + Array.from(url).length,
  };
}

function inline(text) {
  const chars = Array.from(text);
  let out = "";
  let index = 0;
  while (index < chars.length) {
    const char = chars[index];
    if (char === "\\" && PUNCTUATION.includes(chars[index + 1])) {
      out += escapeHtml(chars[index + 1]);
      index += 2;
      continue;
    }
    if (char === "`") {
      const span = codeSpan(chars, index);
      if (span) {
        out += span.html;
        index = span.end;
        continue;
      }
    }
    if (char === "*" || char === "_") {
      const marked = emphasis(chars, index, char);
      if (marked) {
        out += marked.html;
        index = marked.end;
        continue;
      }
    }
    if (char === "~" && chars[index + 1] === "~") {
      const struck = strikethrough(chars, index);
      if (struck) {
        out += struck.html;
        index = struck.end;
        continue;
      }
    }
    if (char === "[") {
      const link = linkAt(chars, index);
      if (link) {
        out += `<a href="${escapeHtml(link.url)}" target="_blank" rel="noreferrer">${inline(link.label)}</a>`;
        index = link.end;
        continue;
      }
    }
    if (char === "h") {
      const url = bareUrl(chars, index);
      if (url) {
        out += url.html;
        index = url.end;
        continue;
      }
    }
    out += escapeHtml(char);
    index += 1;
  }
  return out;
}

function indentOf(line) {
  return line.match(/^\s*/)[0].length;
}

function itemAt(line) {
  const match = line.match(/^(\s*)([-*+]|\d{1,9}[.)])\s+(.*)$/);
  if (!match) return null;
  return {
    indent: match[1].length,
    ordered: /\d/.test(match[2]),
    contentIndent: match[1].length + match[2].length + 1,
    text: match[3],
  };
}

function isRule(line) {
  return /^ {0,3}([-*_])(\s*\1){2,}\s*$/.test(line);
}

function headingAt(line) {
  const match = line.match(/^ {0,3}(#{1,6})\s+(.*?)\s*#*\s*$/);
  if (!match) return null;
  const level = Math.min(match[1].length + 1, 4);
  return `<h${level}>${inline(match[2])}</h${level}>`;
}

function fenceAt(lines, index) {
  const open = lines[index].match(/^ {0,3}(`{3,}|~{3,})\s*(\S*)/);
  if (!open) return null;
  const body = [];
  let end = index + 1;
  while (end < lines.length && !/^ {0,3}(`{3,}|~{3,})\s*$/.test(lines[end])) {
    body.push(lines[end]);
    end += 1;
  }
  if (end < lines.length) end += 1;
  const language = open[2].toLowerCase();
  return { html: `<pre><code>${highlight(body.join("\n"), language)}</code></pre>`, end };
}

function quoteAt(lines, index) {
  if (!/^ {0,3}>/.test(lines[index])) return null;
  const body = [];
  let end = index;
  while (end < lines.length && /^ {0,3}>/.test(lines[end])) {
    body.push(lines[end].replace(/^ {0,3}> ?/, ""));
    end += 1;
  }
  return { html: `<blockquote>${blocks(body)}</blockquote>`, end };
}

function listAt(lines, start) {
  const first = itemAt(lines[start]);
  if (!first) return null;
  const belongs = (line) => {
    if (line.trim() === "") return true;
    const item = itemAt(line);
    if (item) return item.indent >= first.indent;
    return indentOf(line) > first.indent;
  };
  let end = start;
  while (end < lines.length && belongs(lines[end])) end += 1;

  const roots = [];
  const open = [];
  for (const line of lines.slice(start, end)) {
    if (line.trim() === "") continue;
    const item = itemAt(line);
    if (!item) {
      const node = open[open.length - 1];
      node.text.push(line.slice(Math.min(node.contentIndent, indentOf(line))));
      continue;
    }
    while (open.length && open[open.length - 1].indent >= item.indent) open.pop();
    const node = { ...item, children: [], text: [item.text] };
    if (open.length) open[open.length - 1].children.push(node);
    else roots.push(node);
    open.push(node);
  }
  return { html: renderList(roots), end };
}

function renderList(nodes) {
  let html = "";
  let index = 0;
  while (index < nodes.length) {
    const ordered = nodes[index].ordered;
    const group = [];
    while (index < nodes.length && nodes[index].ordered === ordered) group.push(nodes[index++]);
    const tag = ordered ? "ol" : "ul";
    html += `<${tag}>${group.map(renderItem).join("")}</${tag}>`;
  }
  return html;
}

function renderItem(node) {
  const task = node.text[0].match(/^\[([ xX])\]\s+(.*)$/);
  const box = task
    ? `<input type="checkbox" disabled${task[1] === " " ? "" : " checked"}> `
    : "";
  const text = task ? [task[2], ...node.text.slice(1)] : node.text;
  const html = blocks(text);
  const body = oneParagraph(html);
  return `<li>${box}${body === null ? html : body}${renderList(node.children)}</li>`;
}

function oneParagraph(html) {
  if (!html.startsWith("<p>") || !html.endsWith("</p>")) return null;
  const body = html.slice(3, -4);
  return body.includes("</p>") ? null : body;
}

function tableAt(lines, start) {
  const header = lines[start];
  const rule = lines[start + 1];
  if (!header || !rule || !header.includes("|")) return null;
  const marks = splitRow(rule);
  if (!marks.length || !marks.every((mark) => /^:?-+:?$/.test(mark))) return null;
  const heads = splitRow(header);
  if (heads.length !== marks.length) return null;

  const rows = [];
  let end = start + 2;
  while (end < lines.length && lines[end].includes("|") && lines[end].trim() !== "") {
    rows.push(splitRow(lines[end]));
    end += 1;
  }

  const aligns = marks.map(alignOf);
  const cell = (tag, text, column) => {
    const align = aligns[column];
    return `<${tag}${align ? ` class="${align}"` : ""}>${inline(text)}</${tag}>`;
  };

  let html = '<div class="table-wrap"><table><thead><tr>';
  heads.forEach((head, column) => {
    html += cell("th", head, column);
  });
  html += "</tr></thead><tbody>";
  for (const row of rows) {
    html += "<tr>";
    heads.forEach((_, column) => {
      html += cell("td", row[column] || "", column);
    });
    html += "</tr>";
  }
  html += "</tbody></table></div>";
  return { html, end };
}

function splitRow(line) {
  const cells = [];
  let current = "";
  const body = line.trim().replace(/^\|/, "").replace(/\|$/, "");
  for (let index = 0; index < body.length; index += 1) {
    if (body[index] === "\\" && body[index + 1] === "|") {
      current += "|";
      index += 1;
      continue;
    }
    if (body[index] === "|") {
      cells.push(current.trim());
      current = "";
      continue;
    }
    current += body[index];
  }
  cells.push(current.trim());
  return cells;
}

function alignOf(mark) {
  const left = mark.startsWith(":");
  const right = mark.endsWith(":");
  if (left && right) return "center";
  return right ? "right" : "";
}

function paragraphAt(lines, index) {
  const body = [lines[index].trim()];
  let end = index + 1;
  while (end < lines.length && !startsBlock(lines, end)) {
    body.push(lines[end].trim());
    end += 1;
  }
  return { html: `<p>${inline(body.join(" "))}</p>`, end };
}

function startsBlock(lines, index) {
  const line = lines[index];
  if (line.trim() === "") return true;
  if (itemAt(line)) return true;
  if (headingAt(line)) return true;
  if (isRule(line)) return true;
  if (/^ {0,3}>/.test(line)) return true;
  if (fenceAt(lines, index)) return true;
  if (tableAt(lines, index)) return true;
  return false;
}

function blocks(lines) {
  let html = "";
  let index = 0;
  while (index < lines.length) {
    const line = lines[index];
    if (line.trim() === "") {
      index += 1;
      continue;
    }
    const fence = fenceAt(lines, index);
    if (fence) {
      html += fence.html;
      index = fence.end;
      continue;
    }
    const heading = headingAt(line);
    if (heading) {
      html += heading;
      index += 1;
      continue;
    }
    if (isRule(line)) {
      html += "<hr>";
      index += 1;
      continue;
    }
    const quote = quoteAt(lines, index);
    if (quote) {
      html += quote.html;
      index = quote.end;
      continue;
    }
    const table = tableAt(lines, index);
    if (table) {
      html += table.html;
      index = table.end;
      continue;
    }
    const list = listAt(lines, index);
    if (list) {
      html += list.html;
      index = list.end;
      continue;
    }
    const paragraph = paragraphAt(lines, index);
    html += paragraph.html;
    index = paragraph.end;
  }
  return html;
}

function markdown(source) {
  return blocks(String(source).split("\n"));
}

/* Syntax highlighting */

const KEYWORDS = {
  js: "await async break case catch class const continue default delete do else export extends finally for from function if import in instanceof let new of return static super switch this throw try typeof var void while yield true false null undefined",
  rust: "as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait true type unsafe use where while",
  go: "break case chan const continue default defer else fallthrough for func go goto if import interface map package range return select struct switch type var true false nil",
  python: "and as assert async await break class continue def del elif else except False finally for from global if import in is lambda None nonlocal not or pass raise return True try while with yield",
  bash: "if then else elif fi for while do done case esac function return export local readonly source echo cd set unset exit",
  sql: "select from where insert into values update set delete create table drop alter join left right inner outer group by order having limit offset and or not null as distinct",
  json: "true false null",
  toml: "true false",
  yaml: "true false null yes no on off",
};

const SPECS = new Map(
  [
    ["js", { line: ["//"], block: [["/*", "*/"]], strings: ["'", '"', "`"], keywords: KEYWORDS.js }],
    ["rust", { line: ["//"], block: [["/*", "*/"]], keywords: KEYWORDS.rust, chars: true }],
    ["go", { line: ["//"], block: [["/*", "*/"]], strings: ['"', "'", "`"], keywords: KEYWORDS.go }],
    ["python", { line: ["#"], strings: ['"', "'"], keywords: KEYWORDS.python }],
    ["bash", { line: ["#"], keywords: KEYWORDS.bash }],
    ["json", { strings: ['"'], keywords: KEYWORDS.json }],
    ["css", { block: [["/*", "*/"]] }],
    ["html", { block: [["<!--", "--"]] }],
    ["sql", { line: ["--"], block: [["/*", "*/"]], keywords: KEYWORDS.sql }],
    ["toml", { line: ["#"], keywords: KEYWORDS.toml }],
    ["yaml", { line: ["#"], keywords: KEYWORDS.yaml }],
  ].map(([name, spec]) => [
    name,
    {
      line: spec.line || [],
      block: spec.block || [],
      strings: spec.strings || ['"', "'"],
      chars: spec.chars || false,
      keywords: new Set((spec.keywords || "").split(" ").filter(Boolean)),
    },
  ]),
);

const ALIASES = {
  javascript: "js",
  jsx: "js",
  ts: "js",
  typescript: "js",
  tsx: "js",
  mjs: "js",
  json5: "json",
  jsonl: "json",
  rs: "rust",
  py: "python",
  sh: "bash",
  shell: "bash",
  zsh: "bash",
  console: "bash",
  xml: "html",
  svg: "html",
  yml: "yaml",
};

function token(kind, text) {
  return `<span class="tok-${kind}">${escapeHtml(text)}</span>`;
}

/// Un `'` en Rust o Go abre un carácter —`'/'`, `'\n'`— y no un texto, así que el
/// que cierra lejos es una vida (`'static`) y va sin color.
function isChar(code, i) {
  return code[i + 1] === "\\" ? code[i + 3] === "'" : code[i + 2] === "'";
}

function highlight(code, language) {
  const name = ALIASES[language] || language;
  if (name === "diff" || name === "patch") return highlightDiff(code);
  const spec = SPECS.get(name);
  if (!spec) return escapeHtml(code);

  let out = "";
  let i = 0;
  const n = code.length;
  while (i < n) {
    const rest = code.slice(i);

    const lineMark = spec.line.find((mark) => rest.startsWith(mark));
    if (lineMark) {
      const end = code.indexOf("\n", i);
      const stop = end === -1 ? n : end;
      out += token("comment", code.slice(i, stop));
      i = stop;
      continue;
    }

    const block = spec.block.find(([open]) => rest.startsWith(open));
    if (block) {
      const [open, close] = block;
      const end = code.indexOf(close, i + open.length);
      const stop = end === -1 ? n : end + close.length;
      out += token("comment", code.slice(i, stop));
      i = stop;
      continue;
    }

    const char = code[i];
    if (spec.strings.includes(char) && (char !== "'" || !spec.chars || isChar(code, i))) {
      let j = i + 1;
      while (j < n) {
        if (code[j] === "\\") {
          j += 2;
          continue;
        }
        if (code[j] === char) {
          j += 1;
          break;
        }
        if (char !== "`" && code[j] === "\n") break;
        j += 1;
      }
      out += token("string", code.slice(i, j));
      i = j;
      continue;
    }

    if (/[0-9]/.test(char) && !/[A-Za-z_]/.test(code[i - 1] || "")) {
      let j = i;
      while (j < n && /[0-9a-fA-FxX._]/.test(code[j])) j += 1;
      out += token("number", code.slice(i, j));
      i = j;
      continue;
    }

    if (/[A-Za-z_$]/.test(char)) {
      let j = i;
      while (j < n && /[A-Za-z0-9_$]/.test(code[j])) j += 1;
      const word = code.slice(i, j);
      out += spec.keywords.has(word) ? token("keyword", word) : escapeHtml(word);
      i = j;
      continue;
    }

    out += escapeHtml(char);
    i += 1;
  }
  return out;
}

function highlightDiff(code) {
  return code
    .split("\n")
    .map((line) => {
      if (line.startsWith("+")) return token("add", line);
      if (line.startsWith("-")) return token("del", line);
      if (line.startsWith("@@")) return token("meta", line);
      return escapeHtml(line);
    })
    .join("\n");
}

if (typeof module !== "undefined") module.exports = { markdown, inline, escapeHtml };
