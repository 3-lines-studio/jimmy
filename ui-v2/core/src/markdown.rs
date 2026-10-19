use pulldown_cmark::{html, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

const COPY: &str = r#"<rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/>"#;
const CHECK: &str = r#"<path d="M20 6 9 17l-5-5"/>"#;

fn svg(class: &str, size: u32, paths: &str) -> String {
    format!(
        r#"<svg viewBox="0 0 24 24" width="{size}" height="{size}" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" class="{class}">{paths}</svg>"#
    )
}

fn escape_attr(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for char in text.chars() {
        match char {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '\n' => out.push_str("&#10;"),
            char => out.push(char),
        }
    }
    out
}

fn copy_button(text: Option<&str>, label: &str) -> String {
    let idle = svg("idle", 14, COPY);
    let done = svg("done", 14, CHECK);
    let data = match text {
        Some(text) => format!(r#" data-copy="{}""#, escape_attr(text)),
        None => String::new(),
    };
    format!(
        r#"<button class="copy"{data} title="{label}" aria-label="{label}">{idle}{done}</button>"#
    )
}

fn next(level: HeadingLevel) -> HeadingLevel {
    match level {
        HeadingLevel::H1 => HeadingLevel::H2,
        HeadingLevel::H2 => HeadingLevel::H3,
        _ => HeadingLevel::H4,
    }
}

struct Spec {
    line: &'static [&'static str],
    block: &'static [(&'static str, &'static str)],
    strings: &'static [char],
    chars: bool,
    keywords: &'static [&'static str],
}

const KEYWORDS_JS: &[&str] = &["await", "async", "break", "case", "catch", "class", "const", "continue", "default", "delete", "do", "else", "export", "extends", "finally", "for", "from", "function", "if", "import", "in", "instanceof", "let", "new", "of", "return", "static", "super", "switch", "this", "throw", "try", "typeof", "var", "void", "while", "yield", "true", "false", "null", "undefined"];
const KEYWORDS_RUST: &[&str] = &["as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while"];
const KEYWORDS_GO: &[&str] = &["break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough", "for", "func", "go", "goto", "if", "import", "interface", "map", "package", "range", "return", "select", "struct", "switch", "type", "var", "true", "false", "nil"];
const KEYWORDS_PYTHON: &[&str] = &["and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except", "False", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return", "True", "try", "while", "with", "yield"];
const KEYWORDS_BASH: &[&str] = &["if", "then", "else", "elif", "fi", "for", "while", "do", "done", "case", "esac", "function", "return", "export", "local", "readonly", "source", "echo", "cd", "set", "unset", "exit"];
const KEYWORDS_SQL: &[&str] = &["select", "from", "where", "insert", "into", "values", "update", "set", "delete", "create", "table", "drop", "alter", "join", "left", "right", "inner", "outer", "group", "by", "order", "having", "limit", "offset", "and", "or", "not", "null", "as", "distinct"];
const KEYWORDS_JSON: &[&str] = &["true", "false", "null"];
const KEYWORDS_TOML: &[&str] = &["true", "false"];
const KEYWORDS_YAML: &[&str] = &["true", "false", "null", "yes", "no", "on", "off"];

fn spec(name: &str) -> Option<Spec> {
    let spec = match name {
        "js" => Spec { line: &["//"], block: &[("/*", "*/")], strings: &['\'', '"', '`'], chars: false, keywords: KEYWORDS_JS },
        "rust" => Spec { line: &["//"], block: &[("/*", "*/")], strings: &['"'], chars: true, keywords: KEYWORDS_RUST },
        "go" => Spec { line: &["//"], block: &[("/*", "*/")], strings: &['"', '\'', '`'], chars: false, keywords: KEYWORDS_GO },
        "python" => Spec { line: &["#"], block: &[], strings: &['"', '\''], chars: false, keywords: KEYWORDS_PYTHON },
        "bash" => Spec { line: &["#"], block: &[], strings: &['"', '\''], chars: false, keywords: KEYWORDS_BASH },
        "json" => Spec { line: &[], block: &[], strings: &['"'], chars: false, keywords: KEYWORDS_JSON },
        "css" => Spec { line: &[], block: &[("/*", "*/")], strings: &['"', '\''], chars: false, keywords: &[] },
        "html" => Spec { line: &[], block: &[("<!--", "--")], strings: &['"', '\''], chars: false, keywords: &[] },
        "sql" => Spec { line: &["--"], block: &[("/*", "*/")], strings: &['"', '\''], chars: false, keywords: KEYWORDS_SQL },
        "toml" => Spec { line: &["#"], block: &[], strings: &['"', '\''], chars: false, keywords: KEYWORDS_TOML },
        "yaml" => Spec { line: &["#"], block: &[], strings: &['"', '\''], chars: false, keywords: KEYWORDS_YAML },
        _ => return None,
    };
    Some(spec)
}

fn alias(language: &str) -> &str {
    match language {
        "javascript" | "jsx" | "ts" | "typescript" | "tsx" | "mjs" => "js",
        "json5" | "jsonl" => "json",
        "rs" => "rust",
        "py" => "python",
        "sh" | "shell" | "zsh" | "console" => "bash",
        "xml" | "svg" => "html",
        "yml" => "yaml",
        other => other,
    }
}

fn token(kind: &str, text: &str) -> String {
    format!("<span class=\"tok-{kind}\">{}</span>", escape_attr(text))
}

pub fn highlight_diff(code: &str) -> String {
    code.split('\n')
        .map(|line| {
            if line.starts_with('+') {
                token("add", line)
            } else if line.starts_with('-') {
                token("del", line)
            } else if line.starts_with("@@") {
                token("meta", line)
            } else {
                escape_attr(line)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn highlight(code: &str, language: &str) -> String {
    let name = alias(language);
    if name == "diff" || name == "patch" {
        return highlight_diff(code);
    }
    let Some(spec) = spec(name) else {
        return escape_attr(code);
    };
    let chars: Vec<char> = code.chars().collect();
    let n = chars.len();
    let mut out = String::new();
    let mut i = 0;
    while i < n {
        let rest: String = chars[i..].iter().collect();
        if let Some(mark) = spec.line.iter().find(|mark| rest.starts_with(**mark)) {
            let _ = mark;
            let stop = code_char_index(&chars, i, '\n').unwrap_or(n);
            out.push_str(&token("comment", &chars[i..stop].iter().collect::<String>()));
            i = stop;
            continue;
        }
        if let Some((open, close)) = spec.block.iter().find(|(open, _)| rest.starts_with(*open)) {
            let from = i + open.chars().count();
            let stop = match find_str(&chars, from, close) {
                Some(index) => index + close.chars().count(),
                None => n,
            };
            out.push_str(&token("comment", &chars[i..stop].iter().collect::<String>()));
            i = stop;
            continue;
        }
        let char = chars[i];
        if spec.strings.contains(&char)
            && (char != '\'' || !spec.chars || is_char(&chars, i))
        {
            let mut j = i + 1;
            while j < n {
                if chars[j] == '\\' {
                    j += 2;
                    continue;
                }
                if chars[j] == char {
                    j += 1;
                    break;
                }
                if char != '`' && chars[j] == '\n' {
                    break;
                }
                j += 1;
            }
            let stop = j.min(n);
            out.push_str(&token("string", &chars[i..stop].iter().collect::<String>()));
            i = stop;
            continue;
        }
        if char.is_ascii_digit()
            && !(i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_'))
        {
            let mut j = i;
            while j < n
                && (chars[j].is_ascii_hexdigit()
                    || matches!(chars[j], 'x' | 'X' | '.' | '_'))
            {
                j += 1;
            }
            out.push_str(&token("number", &chars[i..j].iter().collect::<String>()));
            i = j;
            continue;
        }
        if char.is_alphanumeric() || char == '_' || char == '$' {
            let mut j = i;
            while j < n
                && (chars[j].is_alphanumeric() || chars[j] == '_' || chars[j] == '$')
            {
                j += 1;
            }
            let word: String = chars[i..j].iter().collect();
            match spec.keywords.contains(&word.as_str()) {
                true => out.push_str(&token("keyword", &word)),
                false => out.push_str(&escape_attr(&word)),
            }
            i = j;
            continue;
        }
        out.push_str(&escape_attr(&char.to_string()));
        i += 1;
    }
    out
}

fn is_char(chars: &[char], i: usize) -> bool {
    if chars.get(i + 1) == Some(&'\\') {
        chars.get(i + 3) == Some(&'\'')
    } else {
        chars.get(i + 2) == Some(&'\'')
    }
}

fn wrap_tables(html: &str) -> String {
    let replaced = html
        .replace(r#"<th style="text-align: right">"#, r#"<th class="right">"#)
        .replace(r#"<td style="text-align: right">"#, r#"<td class="right">"#)
        .replace(r#"<th style="text-align: center">"#, r#"<th class="center">"#)
        .replace(r#"<td style="text-align: center">"#, r#"<td class="center">"#)
        .replace(r#"<th style="text-align: left">"#, "<th>")
        .replace(r#"<td style="text-align: left">"#, "<td>");
    let mut out = String::with_capacity(replaced.len());
    let mut rest = replaced.as_str();
    while let Some(start) = rest.find("<table>") {
        out.push_str(&rest[..start]);
        out.push_str(r#"<div class="table-wrap">"#);
        rest = &rest[start..];
        match rest.find("</table>") {
            Some(end) => {
                out.push_str(&rest[..end + "</table>".len()].replace('\n', ""));
                out.push_str("</div>");
                rest = rest[end + "</table>".len()..].trim_start_matches('\n');
            }
            None => break,
        }
    }
    out.push_str(rest);
    out
}

fn code_char_index(chars: &[char], from: usize, target: char) -> Option<usize> {
    chars
        .iter()
        .enumerate()
        .skip(from)
        .find(|(_, char)| **char == target)
        .map(|(index, _)| index)
}

fn find_str(chars: &[char], from: usize, needle: &str) -> Option<usize> {
    let needle: Vec<char> = needle.chars().collect();
    if needle.is_empty() || chars.len() < needle.len() {
        return None;
    }
    (from..=chars.len() - needle.len()).find(|&index| {
        chars[index..index + needle.len()] == needle[..]
    })
}

fn wrap_code(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find("<pre>") {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        let Some(end) = after.find("</pre>") else {
            out.push_str(after);
            return out;
        };
        let block = &after[..end + "</pre>".len()];
        out.push_str(r#"<div class="code">"#);
        out.push_str(block);
        out.push_str(&copy_button(None, "Copiar el código"));
        out.push_str("</div>");
        rest = &after[end + "</pre>".len()..];
    }
    out.push_str(rest);
    out
}

pub fn render(source: &str) -> String {
    if source.trim().is_empty() {
        return String::new();
    }
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let events: Vec<Event> = Parser::new_ext(source, options).collect();
    let mut highlighted: Vec<Event> = Vec::new();
    let mut index = 0;
    while index < events.len() {
        let language = match &events[index] {
            Event::Start(Tag::CodeBlock(kind)) => match kind {
                pulldown_cmark::CodeBlockKind::Fenced(name) => Some(name.to_string()),
                pulldown_cmark::CodeBlockKind::Indented => Some(String::new()),
            },
            _ => None,
        };
        let Some(language) = language else {
            highlighted.push(events[index].clone());
            index += 1;
            continue;
        };
        let mut code = String::new();
        index += 1;
        while index < events.len() && !matches!(events[index], Event::End(TagEnd::CodeBlock)) {
            if let Event::Text(text) = &events[index] {
                code.push_str(text);
            }
            index += 1;
        }
        index += 1;
        let code = code.strip_suffix('\n').unwrap_or(&code).to_string();
        highlighted.push(Event::Html(
            format!("\u{1}<pre><code>{}</code></pre>", highlight(&code, &language)).into(),
        ));
    }
    let parser = highlighted.into_iter().map(|event| match event {
        Event::Html(text) if text.starts_with('\u{1}') => {
            Event::Html(text[1..].to_string().into())
        }
        Event::Html(text) | Event::InlineHtml(text) => Event::Text(text),
        Event::Start(Tag::Heading {
            level,
            id,
            classes,
            attrs,
        }) => Event::Start(Tag::Heading {
            level: next(level),
            id,
            classes,
            attrs,
        }),
        Event::End(TagEnd::Heading(level)) => Event::End(TagEnd::Heading(next(level))),
        event => event,
    });
    let mut html = String::new();
    html::push_html(&mut html, parser);
    let mut out = wrap_tables(&html);
    out = wrap_code(&out);
    out.push_str(r#"<div class="actions">"#);
    out.push_str(&copy_button(Some(source), "Copiar el mensaje"));
    out.push_str("</div>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_gets_a_copy_button() {
        let html = render("```rust\nlet x = 1 < 2;\n```");
        assert!(html.contains(r#"<div class="code">"#), "{html}");
        assert!(html.contains("Copiar el código"), "{html}");
        assert!(
            html.contains(r#"class="code"><pre><code>"#),
            "{html}"
        );
    }

    #[test]
    fn the_message_gets_its_own_copy_button() {
        let html = render("hola");
        assert!(html.contains(r#"<div class="actions">"#), "{html}");
        assert!(html.contains("Copiar el mensaje"), "{html}");
        assert!(html.contains("<p>hola</p>"), "{html}");
    }

    #[test]
    fn headings_start_at_two_like_the_v1() {
        let html = render("# uno\n\n## dos\n\n### tres\n\n#### cuatro");
        assert!(html.contains("<h2>uno</h2>"), "{html}");
        assert!(html.contains("<h3>dos</h3>"), "{html}");
        assert!(html.contains("<h4>tres</h4>"), "{html}");
        assert!(html.contains("<h4>cuatro</h4>"), "{html}");
    }

    #[test]
    fn code_is_highlighted_like_the_v1() {
        let html = render("```rust\nfn main() { let x = 1; }\n```");
        assert!(html.contains(r#"<span class="tok-keyword">fn</span>"#), "{html}");
        assert!(html.contains(r#"<span class="tok-number">1</span>"#), "{html}");
        assert!(html.contains(r#"<div class="code"><pre><code>"#), "{html}");
        let plain = render("```diff\n+uno\n-dos\n@@ x\n```");
        assert!(plain.contains(r#"<span class="tok-add">+uno</span>"#), "{plain}");
        assert!(plain.contains(r#"<span class="tok-del">-dos</span>"#), "{plain}");
        assert!(plain.contains(r#"<span class="tok-meta">@@ x</span>"#), "{plain}");
        let unknown = render("```\nsin lenguaje\n```");
        assert!(unknown.contains("sin lenguaje"), "{unknown}");
        assert!(!unknown.contains("tok-"), "{unknown}");
        let escaped = highlight("a < b", "bash");
        assert!(escaped.contains("&lt;"), "{escaped}");
    }

    #[test]
    fn tables_are_wrapped_and_aligned_like_the_v1() {
        let html = render("| a | b | c |\n|---|--:|:-:|\n| 1 | 2 | 3 |");
        assert!(
            html.starts_with(r#"<div class="table-wrap"><table><thead><tr><th>a</th><th class="right">b</th><th class="center">c</th></tr></thead><tbody><tr><td>1</td><td class="right">2</td><td class="center">3</td></tr></tbody></table></div>"#),
            "{html}"
        );
    }

    #[test]
    fn raw_html_is_escaped() {
        let html = render("<script>alert(1)</script>");
        assert!(!html.contains("<script>"), "{html}");
        assert!(html.contains("&lt;script&gt;"), "{html}");
    }
}
