pub fn state_path() -> &'static str {
    "/api/state"
}

pub fn history_path(key: &str, before: usize) -> String {
    format!("/api/history?conversation={key}&before={before}")
}

pub fn stream_path(key: &str, since: usize) -> String {
    format!("/api/stream?conversation={key}&since={since}")
}

pub fn file_path(conversation: &str, name: &str) -> String {
    format!("/api/file?conversation={conversation}&name={name}")
}

pub fn tree_path(project: &str, path: &str) -> String {
    format!("/api/tree?project={project}&path={}", encode(path))
}

pub fn raw_path(project: &str, path: &str) -> String {
    format!("/api/raw?project={project}&path={}", encode(path))
}

pub fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        let plain = byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')');
        match plain {
            true => out.push(byte as char),
            false => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_paths_carry_the_query_like_the_v1() {
        assert_eq!(state_path(), "/api/state");
        assert_eq!(history_path("web-1", 40), "/api/history?conversation=web-1&before=40");
        assert_eq!(stream_path("web-1", 7), "/api/stream?conversation=web-1&since=7");
        assert_eq!(file_path("web-1", "uno.png"), "/api/file?conversation=web-1&name=uno.png");
        assert_eq!(tree_path("ken", "src/main.rs"), "/api/tree?project=ken&path=src%2Fmain.rs");
        assert_eq!(raw_path("ken", "notas uno.md"), "/api/raw?project=ken&path=notas%20uno.md");
    }

    #[test]
    fn the_encoder_leaves_the_unreserved_alone() {
        assert_eq!(encode("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(encode("a/b?c=d&e"), "a%2Fb%3Fc%3Dd%26e");
    }
}
