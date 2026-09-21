use std::process::ExitCode;

const ENDPOINT: &str = "https://html.duckduckgo.com/html/";
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64)";
const DEFAULT_COUNT: usize = 8;

struct Item {
    url: String,
    title: String,
    snippet: String,
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut count = DEFAULT_COUNT;

    if args.first().map(String::as_str) == Some("-n") {
        if args.len() < 2 {
            eprintln!("uso: search [-n N] consulta");
            return ExitCode::FAILURE;
        }
        match args[1].parse() {
            Ok(value) => count = value,
            Err(_) => {
                eprintln!("search: -n requiere un número");
                return ExitCode::FAILURE;
            }
        }
        args.drain(0..2);
    }

    if args.is_empty() {
        eprintln!("uso: search [-n N] consulta");
        return ExitCode::FAILURE;
    }

    let html = match fetch(&args.join(" ")) {
        Ok(html) => html,
        Err(error) => {
            eprintln!("search: {error}");
            return ExitCode::FAILURE;
        }
    };

    for (index, item) in parse(&html).into_iter().take(count).enumerate() {
        println!("{}. {}", index + 1, item.title);
        println!("   {}", item.url);
        let snippet = item
            .snippet
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if !snippet.is_empty() {
            println!("   {snippet}");
        }
    }

    ExitCode::SUCCESS
}

fn fetch(query: &str) -> Result<String, String> {
    let url = format!("{ENDPOINT}?q={}", encode(query));
    ureq::get(&url)
        .set("User-Agent", USER_AGENT)
        .call()
        .map_err(|error| error.to_string())?
        .into_string()
        .map_err(|error| error.to_string())
}

fn encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

enum Mode {
    Idle,
    Title,
    Snippet,
}

fn parse(html: &str) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    let mut mode = Mode::Idle;
    let mut title = String::new();
    let mut href = String::new();
    let mut rest = html;

    while let Some(open) = rest.find('<') {
        let text = decode(&rest[..open]);
        match mode {
            Mode::Title => title.push_str(&text),
            Mode::Snippet => {
                if let Some(item) = items.last_mut() {
                    item.snippet.push_str(&text);
                }
            }
            Mode::Idle => {}
        }

        rest = &rest[open..];
        let Some(close) = rest.find('>') else {
            break;
        };
        classify(
            &rest[1..close],
            &mut items,
            &mut mode,
            &mut title,
            &mut href,
        );
        rest = &rest[close + 1..];
    }

    items
}

fn classify(
    tag: &str,
    items: &mut Vec<Item>,
    mode: &mut Mode,
    title: &mut String,
    href: &mut String,
) {
    if !tag_name(tag).eq_ignore_ascii_case("a") {
        return;
    }

    if tag.starts_with('/') {
        match mode {
            Mode::Title => {
                if !href.is_empty() {
                    items.push(Item {
                        url: real_url(href),
                        title: title.trim().to_string(),
                        snippet: String::new(),
                    });
                }
                *mode = Mode::Idle;
            }
            Mode::Snippet => *mode = Mode::Idle,
            Mode::Idle => {}
        }
        return;
    }

    let class = attr(tag, "class").unwrap_or_default();
    if class.split_whitespace().any(|token| token == "result__a") {
        *href = attr(tag, "href").unwrap_or_default().to_string();
        title.clear();
        *mode = Mode::Title;
    } else if class
        .split_whitespace()
        .any(|token| token == "result__snippet")
    {
        *mode = Mode::Snippet;
    }
}

fn real_url(href: &str) -> String {
    let href = match href.strip_prefix("//") {
        Some(rest) => format!("https://{rest}"),
        None => href.to_string(),
    };
    match href.find("uddg=") {
        Some(at) => percent_decode(href[at + 5..].split('&').next().unwrap_or("")),
        None => href,
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn tag_name(tag: &str) -> &str {
    let tag = tag.strip_prefix('/').unwrap_or(tag);
    match tag.find(|c: char| c.is_whitespace() || c == '/') {
        Some(end) => &tag[..end],
        None => tag,
    }
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = tag;
    loop {
        let at = find_attr(rest, name)?;
        rest = &rest[at + name.len()..];
        let value = match rest.trim_start().strip_prefix('=') {
            Some(value) => value.trim_start(),
            None => continue,
        };
        let quote = value.chars().next()?;
        if quote == '"' || quote == '\'' {
            let end = value[1..].find(quote)?;
            return Some(&value[1..1 + end]);
        }
        return Some(value.split_whitespace().next().unwrap_or(""));
    }
}

fn find_attr(rest: &str, name: &str) -> Option<usize> {
    let bytes = rest.as_bytes();
    let mut start = 0;
    while let Some(offset) = rest[start..].find(name) {
        let at = start + offset;
        let before = at == 0 || bytes[at - 1].is_ascii_whitespace();
        let after = bytes
            .get(at + name.len())
            .is_some_and(|byte| byte.is_ascii_whitespace() || *byte == b'=');
        if before && after {
            return Some(at);
        }
        start = at + name.len();
    }
    None
}

fn decode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        match tail.find(';').filter(|end| *end <= 12) {
            Some(end) => match entity_char(&tail[1..end]) {
                Some(c) => {
                    out.push(c);
                    rest = &tail[end + 1..];
                }
                None => {
                    out.push('&');
                    rest = &tail[1..];
                }
            },
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }

    out.push_str(rest);
    out
}

fn entity_char(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        _ => {
            if let Some(hex) = entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
            {
                return u32::from_str_radix(hex, 16).ok().and_then(char::from_u32);
            }
            let decimal = entity.strip_prefix('#')?;
            decimal.parse::<u32>().ok().and_then(char::from_u32)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HTML: &str = r#"
    <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Frust-lang.org%2F&amp;rut=abc">The <b>Rust</b> Programming Language</a>
    <a class="result__snippet" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Frust-lang.org%2F&amp;rut=abc"><b>Rust</b> is fast &amp; reliable.</a>
    <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fa%3Fb%3D1&amp;rut=def">Example</a>
    <a class="result__snippet" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2F">Snippet   with
    newlines.</a>
    "#;

    #[test]
    fn parses_results() {
        let items = parse(HTML);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].url, "https://rust-lang.org/");
        assert_eq!(items[0].title, "The Rust Programming Language");
        assert_eq!(items[0].snippet.trim(), "Rust is fast & reliable.");
        assert_eq!(items[1].url, "https://example.com/a?b=1");
        assert_eq!(items[1].title, "Example");
    }

    #[test]
    fn decodes_entities() {
        assert_eq!(decode("a &amp; b &#39;c&#x27; &lt;d&gt;"), "a & b 'c' <d>");
    }
}
