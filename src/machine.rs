//! Lo que la máquina está usando ahora mismo: la memoria del contenedor, el
//! volumen y cuántos procesos hay.
//!
//! La memoria se lee del cgroup, así que es la que ve el que mira el servicio
//! desde afuera e incluye la page cache y el slab, no sólo el heap de los
//! procesos. Por eso van desglosadas: para saber si lo que crece es mío o es el
//! cache de los builds.

use serde::Serialize;
use std::path::Path;

const CGROUP: &str = "/sys/fs/cgroup";

#[derive(Debug, Serialize)]
pub struct Machine {
    pub memory: Memory,
    pub disk: Disk,
    pub processes: usize,
}

#[derive(Debug, Serialize)]
pub struct Memory {
    pub used: u64,
    pub total: u64,
    pub anon: u64,
    pub cache: u64,
    pub kernel: u64,
}

#[derive(Debug, Serialize)]
pub struct Disk {
    pub used: u64,
    pub total: u64,
}

pub fn usage(volume: &Path) -> Machine {
    Machine {
        memory: memory(),
        disk: disk(volume),
        processes: processes(),
    }
}

fn memory() -> Memory {
    let stat = cgroup("memory.stat");
    Memory {
        used: number(&cgroup("memory.current")),
        total: number(&cgroup("memory.max")),
        anon: field(&stat, "anon"),
        cache: field(&stat, "file"),
        kernel: field(&stat, "kernel"),
    }
}

fn cgroup(name: &str) -> String {
    std::fs::read_to_string(format!("{CGROUP}/{name}")).unwrap_or_default()
}

/// El volumen del workspace, que es el que se llena. `df` cuenta lo mismo: los
/// bloques menos los libres.
fn disk(volume: &Path) -> Disk {
    let Ok(path) = std::ffi::CString::new(volume.as_os_str().as_encoded_bytes()) else {
        return Disk { used: 0, total: 0 };
    };
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } != 0 {
        return Disk { used: 0, total: 0 };
    }
    let block = stat.f_frsize as u64;
    Disk {
        used: (stat.f_blocks as u64 - stat.f_bfree as u64) * block,
        total: stat.f_blocks as u64 * block,
    }
}

fn processes() -> usize {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .parse::<i32>()
                .is_ok_and(|pid| pid > 0)
        })
        .count()
}

/// El número que hay escrito, o cero: un tope sin límite se llama `max`.
fn number(text: &str) -> u64 {
    text.trim().parse().unwrap_or(0)
}

/// La línea del `memory.stat` que arranca con ese nombre.
fn field(stat: &str, name: &str) -> u64 {
    stat.lines()
        .find_map(|line| {
            let (key, value) = line.split_once(' ')?;
            (key == name).then_some(value)
        })
        .map(number)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_tope_sin_limite_vale_cero() {
        assert_eq!(number("max\n"), 0);
        assert_eq!(number("1936084992\n"), 1936084992);
        assert_eq!(number(""), 0);
    }

    #[test]
    fn el_stat_se_lee_por_nombre_y_no_por_posicion() {
        let stat = "anon 30408704\nfile 69615616\nkernel 275836928\nslab 106657912\n";
        assert_eq!(field(stat, "anon"), 30408704);
        assert_eq!(field(stat, "file"), 69615616);
        assert_eq!(field(stat, "kernel"), 275836928);
        assert_eq!(field(stat, "sock"), 0);
    }

    #[test]
    fn la_maquina_se_lee_de_verdad() {
        let usage = usage(Path::new("/"));
        assert!(usage.memory.used > 0, "{usage:?}");
        assert!(usage.memory.anon > 0, "{usage:?}");
        assert!(usage.disk.total > 0, "{usage:?}");
        assert!(usage.processes > 0, "{usage:?}");
    }
}
