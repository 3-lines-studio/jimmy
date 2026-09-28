//! El vigilante de la memoria que ya no se puede culpar a nadie: cuando no hay
//! workers, ni previews, ni nada corriendo, lo que el contenedor pesa es lo que
//! no se suelta, y el reinicio es la única salida.
//!
//! Mide `memory.current` entero y no el `anon`, porque lo que se cobra es el
//! cgroup: la page cache y el slab que dejan los builds no los reclama nadie,
//! que el techo son 32 GB y el kernel no siente presión. `memory.high` y
//! `memory.reclaim` están montados de solo lectura, así que sólo queda nacer de
//! nuevo.

const LIMIT: u64 = 500 * 1024 * 1024;

/// Cinco vueltas es lo que tarda el reaper en bajar a un huérfano (`reap::TTL`):
/// la sexta es la del avistamiento y la séptima la del kill, que esta lectura
/// todavía puede ver alta. Una vuelta más y el que pesa ya no puede ser un
/// huérfano.
const TICKS: u32 = 7;

#[derive(Default)]
pub struct Watch {
    high: u32,
}

impl Watch {
    /// Una vuelta del reaper. `used` son los bytes del cgroup enteros, cache y
    /// slab adentro, y `quiet` que no hay nadie corriendo.
    pub fn overdue(&mut self, used: u64, quiet: bool) -> bool {
        self.high = if quiet && used > LIMIT {
            self.high + 1
        } else {
            0
        };
        self.high >= TICKS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALTO: u64 = LIMIT + 1;

    #[test]
    fn con_alguien_corriendo_no_cuenta() {
        let mut watch = Watch::default();
        for _ in 0..TICKS * 10 {
            assert!(!watch.overdue(ALTO, false));
        }
    }

    #[test]
    fn la_memoria_baja_vuelve_a_contar_de_cero() {
        let mut watch = Watch::default();
        for _ in 0..TICKS - 1 {
            watch.overdue(ALTO, true);
        }
        watch.overdue(LIMIT, true);
        for _ in 0..TICKS - 1 {
            assert!(!watch.overdue(ALTO, true));
        }
    }

    #[test]
    fn la_memoria_alta_sostenida_se_lo_lleva() {
        let mut watch = Watch::default();
        for _ in 0..TICKS - 1 {
            assert!(!watch.overdue(ALTO, true));
        }
        assert!(watch.overdue(ALTO, true));
    }
}
