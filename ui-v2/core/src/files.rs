pub fn extension(name: &str) -> String {
    match name.rfind('.') {
        Some(dot) if dot > 0 => name[dot + 1..].to_lowercase(),
        _ => String::new(),
    }
}

pub fn size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    if bytes < 1024 * 1024 {
        return format!("{} KB", (bytes as f64 / 1024.0).round() as u64);
    }
    format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0).replace('.', ",")
}

pub fn join(path: &str, name: &str) -> String {
    match path.is_empty() {
        true => name.to_string(),
        false => format!("{path}/{name}"),
    }
}

pub fn title(id: &str) -> Option<String> {
    id.strip_prefix("files:")
        .map(|project| format!("Archivos: {}", crate::project::title(project)))
}

pub fn project_of(id: &str) -> Option<String> {
    id.strip_prefix("files:").map(str::to_string)
}

pub fn is_files(id: &str) -> bool {
    id.starts_with("files:")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_extension_is_what_the_highlighter_reads() {
        assert_eq!(extension("web.rs"), "rs");
        assert_eq!(extension("README.MD"), "md");
        assert_eq!(extension("app.test.js"), "js");
        assert_eq!(extension("logo.png"), "png");
    }

    #[test]
    fn a_file_without_a_dot_has_no_extension() {
        assert_eq!(extension("Makefile"), "");
        assert_eq!(extension("LICENSE"), "");
    }

    #[test]
    fn a_leading_dot_is_the_name_not_the_extension() {
        assert_eq!(extension(".env"), "");
    }

    #[test]
    fn the_size_rounds_to_the_unit_you_read_at_a_glance() {
        assert_eq!(size(0), "0 B");
        assert_eq!(size(999), "999 B");
        assert_eq!(size(1024), "1 KB");
        assert_eq!(size(600 * 1024), "600 KB");
        assert_eq!(size(1024 * 1024), "1,0 MB");
        assert_eq!(size(1536 * 1024), "1,5 MB");
    }

    #[test]
    fn the_files_tab_says_which_project_it_shows() {
        assert_eq!(title("files:ken").as_deref(), Some("Archivos: Ken"));
        assert_eq!(title("otra"), None);
        assert_eq!(project_of("files:ken").as_deref(), Some("ken"));
        assert!(is_files("files:ken"));
        assert!(!is_files("ken"));
    }

    #[test]
    fn paths_join_with_a_slash_only_when_needed() {
        assert_eq!(join("", "src"), "src");
        assert_eq!(join("src", "main.rs"), "src/main.rs");
    }
}
