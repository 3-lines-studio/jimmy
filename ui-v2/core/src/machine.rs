const GB: f64 = 1_000_000_000.0;
const MB: f64 = 1_000_000.0;

fn decimal(value: f64) -> String {
    format!("{value:.1}").replace('.', ",").replace(",0", "")
}

pub fn size(bytes: u64) -> String {
    let bytes = bytes as f64;
    if bytes >= GB {
        return format!("{} GB", decimal(bytes / GB));
    }
    format!("{} MB", (bytes / MB).round() as u64)
}

pub fn ratio(used: u64, total: u64) -> String {
    format!("{} / {}", size(used), size(total))
}

pub fn percent(used: u64, total: u64) -> String {
    let value = match total {
        0 => 0.0,
        total => (used as f64 / total as f64) * 100.0,
    };
    format!("{}%", value.round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn megabytes_have_no_decimals_and_gigabytes_one() {
        assert_eq!(size(29 * 1_000_000), "29 MB");
        assert_eq!(size(1_000_000_000), "1 GB");
        assert_eq!(size(1_600_000_000), "1,6 GB");
    }

    #[test]
    fn usage_is_shown_against_its_ceiling() {
        assert_eq!(ratio(1_900_000_000, 32_000_000_000), "1,9 GB / 32 GB");
        assert_eq!(ratio(29_000_000, 32_000_000_000), "29 MB / 32 GB");
    }

    #[test]
    fn the_percentage_is_rounded() {
        assert_eq!(percent(2_300_000_000, 4_800_000_000), "48%");
        assert_eq!(percent(0, 32_000_000_000), "0%");
        assert_eq!(percent(4_800_000_000, 4_800_000_000), "100%");
    }
}
