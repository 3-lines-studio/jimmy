pub fn to_telegram_html(markdown: &str) -> String {
    let mut out = String::new();
    let mut lines = markdown.lines().peekable();
    let mut in_code = false;
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            if in_code {
                out.push_str("</pre>\n");
                in_code = false;
            } else {
                out.push_str("<pre>");
                in_code = true;
            }
            continue;
        }
        if in_code {
            out.push_str(&escape(line));
            out.push('\n');
            continue;
        }
        if trimmed.starts_with('|') && trimmed.ends_with('|') {
            let mut rows = vec![line];
            while let Some(next) = lines.peek().copied() {
                let t = next.trim_start();
                if t.starts_with('|') && t.ends_with('|') {
                    rows.push(next);
                    lines.next();
                } else {
                    break;
                }
            }
            out.push_str("<pre>");
            let mut first = true;
            for row in rows {
                if row
                    .trim()
                    .chars()
                    .all(|c| matches!(c, '|' | '-' | ':' | ' '))
                {
                    continue;
                }
                if !first {
                    out.push('\n');
                }
                out.push_str(&escape(row));
                first = false;
            }
            out.push_str("</pre>\n");
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("> ") {
            let mut quote = rest.to_string();
            while let Some(next) = lines.peek().copied() {
                if let Some(r) = next.trim_start().strip_prefix("> ") {
                    quote.push('\n');
                    quote.push_str(r);
                    lines.next();
                } else {
                    break;
                }
            }
            out.push_str("<blockquote>");
            out.push_str(&inline(&quote));
            out.push_str("</blockquote>\n");
            continue;
        }
        if let Some(text) = heading(trimmed) {
            out.push_str("<b>");
            out.push_str(&inline(text));
            out.push_str("</b>\n");
            continue;
        }
        out.push_str(&inline(line));
        out.push('\n');
    }
    if in_code {
        out.push_str("</pre>");
    }
    out.trim_end().to_string()
}

pub fn split(markdown: &str, max: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut count = 0usize;
    let mut fence: Option<String> = None;
    for line in markdown.split_inclusive('\n') {
        let len = line.chars().count();
        if count > 0 && count + len > max {
            if fence.is_some() {
                current.push_str("```\n");
            }
            chunks.push(std::mem::take(&mut current));
            count = 0;
            if let Some(info) = &fence {
                current.push_str("```");
                current.push_str(info);
                current.push('\n');
                count = 4 + info.chars().count();
            }
        }
        current.push_str(line);
        count += len;
        let t = line.trim_end();
        if t.starts_with("```") {
            fence = match fence {
                Some(_) => None,
                None => Some(t.trim_start_matches('`').to_string()),
            };
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    if chunks.is_empty() {
        chunks.push(String::new());
    }
    chunks
}

fn heading(line: &str) -> Option<&str> {
    let n = line.bytes().take_while(|b| *b == b'#').count();
    if n == 0 || n > 6 {
        return None;
    }
    line[n..].strip_prefix(' ')
}

fn inline(s: &str) -> String {
    let cs: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < cs.len() {
        match cs[i] {
            '`' => {
                if let Some(j) = (i + 1..cs.len()).find(|&j| cs[j] == '`') {
                    out.push_str("<code>");
                    out.push_str(&escape(&text(&cs[i + 1..j])));
                    out.push_str("</code>");
                    i = j + 1;
                } else {
                    out.push('`');
                    i += 1;
                }
            }
            m @ ('*' | '_') => {
                let open = i == 0 || !cs[i - 1].is_alphanumeric();
                let double = i + 1 < cs.len() && cs[i + 1] == m;
                if double && open {
                    if let Some(j) = find_double(&cs, i + 2, m) {
                        out.push_str("<b>");
                        out.push_str(&inline(&text(&cs[i + 2..j])));
                        out.push_str("</b>");
                        i = j + 2;
                        continue;
                    }
                } else if open && i + 1 < cs.len() && !cs[i + 1].is_whitespace() {
                    if let Some(j) = find_italic(&cs, i + 1, m) {
                        out.push_str("<i>");
                        out.push_str(&inline(&text(&cs[i + 1..j])));
                        out.push_str("</i>");
                        i = j + 1;
                        continue;
                    }
                }
                out.push(m);
                i += 1;
            }
            '~' if i + 1 < cs.len() && cs[i + 1] == '~' => {
                if let Some(j) = find_double(&cs, i + 2, '~') {
                    out.push_str("<s>");
                    out.push_str(&inline(&text(&cs[i + 2..j])));
                    out.push_str("</s>");
                    i = j + 2;
                } else {
                    out.push('~');
                    i += 1;
                }
            }
            '[' => {
                if let Some((label, url, end)) = parse_link(&cs, i) {
                    out.push_str("<a href=\"");
                    out.push_str(&escape_attr(&url));
                    out.push_str("\">");
                    out.push_str(&inline(&label));
                    out.push_str("</a>");
                    i = end;
                } else {
                    out.push('[');
                    i += 1;
                }
            }
            c => {
                out.push_str(&escape_char(c));
                i += 1;
            }
        }
    }
    out
}

fn find_double(cs: &[char], from: usize, marker: char) -> Option<usize> {
    (from..cs.len().saturating_sub(1)).find(|&j| cs[j] == marker && cs[j + 1] == marker)
}

fn find_italic(cs: &[char], from: usize, marker: char) -> Option<usize> {
    (from..cs.len()).find(|&j| {
        cs[j] == marker
            && (j == 0 || cs[j - 1] != marker)
            && !cs[j - 1].is_whitespace()
            && (j + 1 >= cs.len() || (!cs[j + 1].is_alphanumeric() && cs[j + 1] != marker))
    })
}

fn parse_link(cs: &[char], i: usize) -> Option<(String, String, usize)> {
    let close = (i + 1..cs.len()).find(|&j| cs[j] == ']')?;
    if close + 1 >= cs.len() || cs[close + 1] != '(' {
        return None;
    }
    let end = (close + 2..cs.len()).find(|&j| cs[j] == ')')?;
    Some((text(&cs[i + 1..close]), text(&cs[close + 2..end]), end + 1))
}

fn text(cs: &[char]) -> String {
    cs.iter().collect()
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        out.push_str(&escape_char(c));
    }
    out
}

fn escape_char(c: char) -> String {
    match c {
        '&' => "&amp;".into(),
        '<' => "&lt;".into(),
        '>' => "&gt;".into(),
        _ => c.to_string(),
    }
}

fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_styles() {
        assert_eq!(to_telegram_html("**hi**"), "<b>hi</b>");
        assert_eq!(to_telegram_html("__hi__"), "<b>hi</b>");
        assert_eq!(to_telegram_html("*hi*"), "<i>hi</i>");
        assert_eq!(to_telegram_html("~~hi~~"), "<s>hi</s>");
        assert_eq!(to_telegram_html("`a_b`"), "<code>a_b</code>");
    }

    #[test]
    fn snake_case_stays_plain() {
        assert_eq!(
            to_telegram_html("user_id and file_path"),
            "user_id and file_path"
        );
        assert_eq!(to_telegram_html("a * b * c"), "a * b * c");
    }

    #[test]
    fn escapes_entities() {
        assert_eq!(to_telegram_html("<b> & x"), "&lt;b&gt; &amp; x");
    }

    #[test]
    fn heading_is_bold() {
        assert_eq!(to_telegram_html("## Title"), "<b>Title</b>");
    }

    #[test]
    fn links() {
        assert_eq!(
            to_telegram_html("[x](https://e.com/a?b=1&c=2)"),
            "<a href=\"https://e.com/a?b=1&amp;c=2\">x</a>"
        );
    }

    #[test]
    fn code_block_escapes() {
        let html = to_telegram_html("```\na < b\n```");
        assert_eq!(html, "<pre>a &lt; b\n</pre>");
    }

    #[test]
    fn blockquote_groups_lines() {
        assert_eq!(
            to_telegram_html("> one\n> two"),
            "<blockquote>one\ntwo</blockquote>"
        );
    }

    #[test]
    fn split_reopens_code_fences() {
        let md = "```\n1111111111\n1111111111\n2222222222\n2222222222\n```\n";
        let parts = split(md, 20);
        assert!(parts.len() >= 2);
        for part in &parts {
            assert_eq!(part.matches("```").count() % 2, 0);
        }
    }
}
