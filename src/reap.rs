use std::collections::HashMap;
use std::time::{Duration, Instant};

const TTL: Duration = Duration::from_secs(30 * 60);

#[derive(Default)]
pub struct Reaper {
    seen: HashMap<i32, Instant>,
}

impl Reaper {
    pub fn reap(&mut self, now: Instant) {
        for pid in self.due(now) {
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    }

    fn due(&mut self, now: Instant) -> Vec<i32> {
        let mut next = HashMap::new();
        let mut ripe = Vec::new();
        for pid in orphans() {
            let first = self.seen.remove(&pid).unwrap_or(now);
            if now.duration_since(first) >= TTL {
                ripe.push(pid);
            } else {
                next.insert(pid, first);
            }
        }
        self.seen = next;
        ripe
    }
}

fn orphans() -> Vec<i32> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let me = std::process::id() as i32;
    entries
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter_map(|name| name.parse::<i32>().ok())
        .filter(|pid| *pid != 1 && *pid != me && parent(*pid) == Some(1))
        .collect()
}

fn parent(pid: i32) -> Option<i32> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("PPid:")?.trim().parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::process::{Command, Stdio};

    fn spawn_orphan() -> i32 {
        let mut child = Command::new("bash")
            .arg("-c")
            .arg("sleep 30 >/dev/null 2>&1 & echo $!")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut out = String::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut out)
            .unwrap();
        child.wait().unwrap();
        out.trim().parse().unwrap()
    }

    fn wait_orphan(pid: i32) {
        for _ in 0..200 {
            if parent(pid) == Some(1) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("el hijo {pid} no quedó reparentado a 1");
    }

    #[test]
    fn orphans_finds_a_reparented_child() {
        let pid = spawn_orphan();
        wait_orphan(pid);
        assert!(orphans().contains(&pid));
    }

    #[test]
    fn a_fresh_orphan_is_not_due() {
        let pid = spawn_orphan();
        wait_orphan(pid);
        let mut reaper = Reaper::default();
        let start = Instant::now();
        assert!(!reaper.due(start).contains(&pid));
        assert!(!reaper
            .due(start + TTL - Duration::from_secs(1))
            .contains(&pid));
    }

    #[test]
    fn an_orphan_is_due_a_full_ttl_after_it_is_seen() {
        let pid = spawn_orphan();
        wait_orphan(pid);
        let mut reaper = Reaper::default();
        let start = Instant::now();
        reaper.due(start);
        assert!(reaper.due(start + TTL).contains(&pid));
    }

    #[test]
    fn orphans_never_lists_self_nor_init() {
        let me = std::process::id() as i32;
        let found = orphans();
        assert!(!found.contains(&1));
        assert!(!found.contains(&me));
    }
}
