use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const HEADER: &str = "ts,current,anon,file,kernel,procs";
const CURRENT: &str = "/sys/fs/cgroup/memory.current";
const STAT: &str = "/sys/fs/cgroup/memory.stat";
const MB: f64 = 1024.0 * 1024.0;

pub fn sample(workspace: &Path) {
    let Ok(stat) = std::fs::read_to_string(STAT) else {
        return;
    };
    let Ok(current) = std::fs::read_to_string(CURRENT) else {
        return;
    };
    let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(workspace.join("state/memory.csv"))
    else {
        return;
    };
    if file.metadata().map(|meta| meta.len()).unwrap_or(0) == 0 {
        let _ = writeln!(file, "{HEADER}");
    }
    let current = current.trim().parse().unwrap_or(0);
    let _ = writeln!(file, "{}", line(&stat, current, procs()));
}

fn line(stat: &str, current: u64, procs: usize) -> String {
    format!(
        "{},{:.1},{:.1},{:.1},{:.1},{}",
        now(),
        mb(current),
        mb(value(stat, "anon")),
        mb(value(stat, "file")),
        mb(value(stat, "kernel")),
        procs
    )
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

fn mb(bytes: u64) -> f64 {
    bytes as f64 / MB
}

fn value(stat: &str, key: &str) -> u64 {
    stat.lines()
        .filter_map(|line| line.split_once(' '))
        .find(|(name, _)| *name == key)
        .and_then(|(_, raw)| raw.trim().parse().ok())
        .unwrap_or(0)
}

fn procs() -> usize {
    std::fs::read_dir("/proc")
        .map(|dir| {
            dir.flatten()
                .filter(|entry| entry.file_name().to_string_lossy().parse::<i32>().is_ok())
                .count()
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_reads_a_field_without_confusing_similar_names() {
        let stat = "anon 1048576\nanon_thp 99\nfile 2097152\nkernel 524288\n";
        assert_eq!(value(stat, "anon"), 1048576);
        assert_eq!(value(stat, "file"), 2097152);
        assert_eq!(value(stat, "kernel"), 524288);
        assert_eq!(value(stat, "missing"), 0);
    }

    #[test]
    fn line_has_one_column_per_header_field() {
        let row = line("anon 1048576\nfile 2097152\nkernel 524288\n", 4194304, 42);
        let fields: Vec<&str> = row.split(',').collect();
        assert_eq!(fields.len(), HEADER.split(',').count());
        assert!(fields[0].parse::<u64>().is_ok());
        assert_eq!(&fields[1..5], ["4.0", "1.0", "2.0", "0.5"]);
        assert_eq!(fields[5], "42");
    }
}
