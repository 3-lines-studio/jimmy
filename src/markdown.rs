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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_message_stays_whole() {
        assert_eq!(split("hola\n", 20), ["hola\n"]);
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
