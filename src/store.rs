//! La base: el esquema y sus migraciones.
//!
//! El esquema vive en `migrations/`, numerado, y se aplica a mano con
//! `jimmy migrate`. El arranque no lo toca: aplicar migraciones en el arranque
//! es aplicarlas en producción.

use postgres::NoTls;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Las migraciones, embebidas. El contenedor no lleva el repo, así que una
/// migración que no esté en el binario no existe.
const MIGRATIONS: &[(&str, &str)] =
    &[("0001_schema", include_str!("../migrations/0001_schema.sql"))];

/// Aplica lo que falte y devuelve los nombres que aplicó.
///
/// Una migración ya aplicada y después editada es un error: el esquema de la
/// base y el del binario se separaron, y seguir aplicando la deja mintiendo.
pub fn migrate(url: &str) -> Result<Vec<&'static str>, String> {
    url.parse::<postgres::Config>()
        .map_err(|_| "DATABASE_URL no es una url de postgres".to_string())?;
    let mut client =
        postgres::Client::connect(url, NoTls).map_err(|e| format!("no pude conectar: {e}"))?;
    client
        .batch_execute(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                 name       text PRIMARY KEY,
                 checksum   text NOT NULL,
                 created_at timestamptz NOT NULL DEFAULT now(),
                 updated_at timestamptz NOT NULL DEFAULT now(),
                 deleted_at timestamptz
             )",
        )
        .map_err(|e| e.to_string())?;
    let applied: HashMap<String, String> = client
        .query("SELECT name, checksum FROM schema_migrations", &[])
        .map_err(|e| e.to_string())?
        .iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    let mut done = Vec::new();
    for &(name, sql) in MIGRATIONS {
        let checksum = hex(&Sha256::digest(sql.as_bytes()));
        match applied.get(name) {
            Some(seen) if *seen == checksum => continue,
            Some(_) => return Err(format!("{name} cambió después de aplicada")),
            None => {}
        }
        let mut tx = client.transaction().map_err(|e| e.to_string())?;
        tx.batch_execute(sql).map_err(|e| format!("{name}: {e}"))?;
        tx.execute(
            "INSERT INTO schema_migrations (name, checksum) VALUES ($1, $2)",
            &[&name, &checksum],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        done.push(name);
    }
    Ok(done)
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn las_migraciones_van_en_orden_y_sin_repetir() {
        let declaradas: Vec<&str> = MIGRATIONS.iter().map(|(name, _)| *name).collect();
        let mut ordenadas = declaradas.clone();
        ordenadas.sort_unstable();
        assert_eq!(ordenadas, declaradas, "no están en orden");
        ordenadas.dedup();
        assert_eq!(ordenadas.len(), declaradas.len(), "hay nombres repetidos");
    }

    #[test]
    #[ignore = "necesita DATABASE_URL"]
    fn migrar_dos_veces_no_cambia_nada() {
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
        migrate(&url).expect("la primera corrida");
        let segunda = migrate(&url).expect("la segunda corrida");
        assert!(segunda.is_empty(), "la segunda corrida no aplica nada");
    }
}
