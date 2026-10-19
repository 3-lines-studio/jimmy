pub const BODIES: [&str; 4] = ["round", "squircle", "square", "egg"];
pub const EYES: [&str; 4] = ["bar", "dot", "square", "slit"];

pub fn hue(name: &str) -> u32 {
    let mut hash: u32 = 0;
    for character in name.chars() {
        hash = (hash.wrapping_mul(31).wrapping_add(character as u32)) % 360;
    }
    hash
}

pub fn color(name: &str) -> String {
    format!("hsl({} 55% 42%)", hue(name))
}

fn hash(name: &str, seed: u32) -> u32 {
    let mut value: u32 = 2166136261 ^ seed;
    for character in name.chars() {
        value ^= character as u32;
        value = value.wrapping_mul(16777619);
    }
    value ^= value >> 15;
    value = value.wrapping_mul(2246822507);
    value ^ (value >> 13)
}

pub fn face(name: &str) -> (&'static str, &'static str) {
    (
        BODIES[(hash(name, 7) % BODIES.len() as u32) as usize],
        EYES[(hash(name, 53) % EYES.len() as u32) as usize],
    )
}

pub fn initials(name: &str) -> String {
    name.chars().take(3).collect()
}

pub fn title(name: &str) -> String {
    let mut characters = name.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => String::new(),
    }
}

pub fn size(bytes: u64) -> String {
    const KB: f64 = 1000.0;
    const MB: f64 = 1_000_000.0;
    const GB: f64 = 1_000_000_000.0;
    let bytes = bytes as f64;
    if bytes >= GB {
        let value = format!("{:.1}", bytes / GB).replace('.', ",");
        return format!("{} GB", value.replace(",0", ""));
    }
    if bytes >= MB {
        return format!("{} MB", (bytes / MB).round() as u64);
    }
    format!("{} KB", (bytes / KB).round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_color_comes_from_the_name() {
        assert_eq!(color("jimmy"), color("jimmy"));
        assert_ne!(color("jimmy"), color("bifrost"));
        assert!(hue("heimdall") < 360);
    }

    #[test]
    fn forty_projects_spread_over_every_body_and_eye() {
        let names: Vec<String> = (0..40).map(|index| format!("proyecto-{index}")).collect();
        let bodies: std::collections::HashSet<&str> = names.iter().map(|name| face(name).0).collect();
        let eyes: std::collections::HashSet<&str> = names.iter().map(|name| face(name).1).collect();
        assert_eq!(bodies.len(), BODIES.len());
        assert_eq!(eyes.len(), EYES.len());
    }

    #[test]
    fn the_pill_is_the_first_three_letters() {
        assert_eq!(initials("jimmy"), "jim");
        assert_eq!(initials("go"), "go");
    }

    #[test]
    fn the_title_capitalizes_the_first_letter() {
        assert_eq!(title("jimmy"), "Jimmy");
        assert_eq!(title("ñandú"), "Ñandú");
        assert_eq!(title("ken-viejo"), "Ken-viejo");
    }

    #[test]
    fn the_size_is_decimal() {
        assert_eq!(size(45000), "45 KB");
        assert_eq!(size(8192), "8 KB");
        assert_eq!(size(999), "1 KB");
        assert_eq!(size(0), "0 KB");
        assert_eq!(size(1000000), "1 MB");
        assert_eq!(size(716800), "717 KB");
        assert_eq!(size(3149824), "3 MB");
        assert_eq!(size(1208803328), "1,2 GB");
        assert_eq!(size(2000000000), "2 GB");
    }
}
