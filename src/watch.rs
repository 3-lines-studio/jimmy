//! El vigilante de la memoria que ya no se puede culpar a nadie: cuando no hay
//! workers, ni previews, ni nada corriendo, lo que el contenedor pesa es lo que
//! no se suelta, y el reinicio es la única salida.
//!
//! Mide `memory.current` entero y no el `anon`, porque lo que se cobra es el
//! cgroup: la page cache y el slab que dejan los builds no los reclama nadie,
//! que el techo lo declara `memory.max` y el kernel no siente presión.
//! `memory.high` y `memory.reclaim` están montados de solo lectura, así que
//! sólo queda nacer de nuevo.

use std::time::Duration;

/// Lo que dura una vuelta: el `sleep` del hilo que mira en `main`.
pub const TICK: Duration = Duration::from_secs(60);

/// El cgroup sin nadie corriendo se mide contra una fracción del techo y no
/// contra un número fijo: lo que el contenedor pesa en reposo — la page cache
/// que dejan los builds, el slab — es el mismo sea el plan que sea, y un piso
/// bajo mata a un jimmy que no hizo nada. Un cuarto del techo es holgado para
/// lo medido (la instancia de dev llegó a 2,6 GB antes de morir) y sigue
/// estando lejos del OOM que el reinicio evita.
const SHARE: u64 = 4;

/// Cuando el cgroup no declara techo, `memory.max` dice `max` y vale cero: ahí
/// el número es un piso elegido para no matar temprano, no una medida.
const NO_LIMIT: u64 = 8 * 1024 * 1024 * 1024;

/// Media hora larga por encima del techo, en vueltas de `TICK`: lo que tarda un
/// build en crecer y en soltarse, más la vuelta que deja ver el peso ya sin
/// nadie corriendo.
const TICKS: u32 = 32;

#[derive(Default)]
pub struct Watch {
    high: u32,
}

impl Watch {
    /// Una vuelta del vigilante. `used` y `total` son los bytes del cgroup enteros
    /// —`memory.current` y `memory.max`, cache y slab adentro— y `quiet` que no
    /// hay nadie corriendo.
    pub fn overdue(&mut self, used: u64, total: u64, quiet: bool) -> bool {
        self.high = if quiet && used > limit(total) {
            self.high + 1
        } else {
            0
        };
        self.high >= TICKS
    }
}

const fn limit(total: u64) -> u64 {
    if total == 0 {
        NO_LIMIT
    } else {
        total / SHARE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOTAL: u64 = 32 * 1024 * 1024 * 1024;
    const ALTO: u64 = limit(TOTAL) + 1;
    const REPOSO: u64 = 569 * 1024 * 1024;

    #[test]
    fn con_alguien_corriendo_no_cuenta() {
        let mut watch = Watch::default();
        for _ in 0..TICKS * 10 {
            assert!(!watch.overdue(ALTO, TOTAL, false));
        }
    }

    #[test]
    fn la_memoria_baja_vuelve_a_contar_de_cero() {
        let mut watch = Watch::default();
        for _ in 0..TICKS - 1 {
            watch.overdue(ALTO, TOTAL, true);
        }
        watch.overdue(limit(TOTAL), TOTAL, true);
        for _ in 0..TICKS - 1 {
            assert!(!watch.overdue(ALTO, TOTAL, true));
        }
    }

    #[test]
    fn el_peso_del_contenedor_en_reposo_no_basta_para_salir() {
        let mut watch = Watch::default();
        for vuelta in 0..TICKS * 2 {
            assert!(
                !watch.overdue(REPOSO, TOTAL, true),
                "salió en la vuelta {vuelta}"
            );
        }
    }

    #[test]
    fn no_sale_antes_de_media_hora() {
        let mut watch = Watch::default();
        for vuelta in 0..TICKS - 2 {
            assert!(
                !watch.overdue(ALTO, TOTAL, true),
                "salió en la vuelta {vuelta}"
            );
        }
    }

    #[test]
    fn la_memoria_alta_sostenida_se_lo_lleva() {
        let mut watch = Watch::default();
        for _ in 0..TICKS - 1 {
            assert!(!watch.overdue(ALTO, TOTAL, true));
        }
        assert!(watch.overdue(ALTO, TOTAL, true));
    }

    #[test]
    fn un_cgroup_sin_techo_declarado_usa_el_piso() {
        assert_eq!(limit(0), NO_LIMIT);
        assert_eq!(limit(TOTAL), TOTAL / SHARE);
    }
}
