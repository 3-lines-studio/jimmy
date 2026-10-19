use pulldown_cmark::{html, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

const COPY: &str = r#"<path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/><rect width="14" height="14" x="8" y="8" rx="2" ry="2"/>"#;
const CHECK: &str = r#"<path d="M20 6 9 17l-5-5"/>"#;

fn svg(class: &str, size: u32, paths: &str) -> String {
    format!(
        r#"<svg class="{class}" viewBox="0 0 24 24" width="{size}" height="{size}" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{paths}</svg>"#
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

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

fn copy_button(text: &str, label: &str) -> String {
    let idle = svg("idle", 14, COPY);
    let done = svg("done", 14, CHECK);
    let escaped = escape_attr(text);
    format!(
        r#"<button class="copy" data-copy="{escaped}" title="{label}" aria-label="{label}">{idle}{done}</button>"#
    )
}

fn next(level: HeadingLevel) -> HeadingLevel {
    match level {
        HeadingLevel::H1 => HeadingLevel::H2,
        HeadingLevel::H2 => HeadingLevel::H3,
        _ => HeadingLevel::H4,
    }
}

fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut inside = false;
    for char in html.chars() {
        match char {
            '<' => inside = true,
            '>' => inside = false,
            char if !inside => out.push(char),
            _ => {}
        }
    }
    out
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
        let inner = block.trim_start_matches("<pre>").trim_end_matches("</pre>");
        let text = unescape(&strip_tags(inner));
        out.push_str(r#"<div class="code">"#);
        out.push_str(block);
        out.push_str(&copy_button(&text, "Copiar el código"));
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
    let parser = Parser::new_ext(source, options).map(|event| match event {
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
    let mut out = wrap_code(&html);
    out.push_str(r#"<div class="actions">"#);
    out.push_str(&copy_button(source, "Copiar el mensaje"));
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
            html.contains(r#"data-copy="let x = 1 &lt; 2;&#10;""#)
                || html.contains(r#"data-copy="let x = 1 &lt; 2;\n""#),
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
    fn raw_html_is_escaped() {
        let html = render("<script>alert(1)</script>");
        assert!(!html.contains("<script>"), "{html}");
        assert!(html.contains("&lt;script&gt;"), "{html}");
    }
}
