//! La base: el esquema y sus migraciones.
//!
//! El esquema vive en `migrations/`, numerado, y se aplica a mano con
//! `jimmy migrate`. El arranque no lo toca: aplicar migraciones en el arranque
//! es aplicarlas en producción.

use postgres::NoTls;
use r2d2_postgres::PostgresConnectionManager;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::Duration;

/// Las migraciones, embebidas. El contenedor no lleva el repo, así que una
/// migración que no esté en el binario no existe.
const MIGRATIONS: &[(&str, &str)] =
    &[("0001_schema", include_str!("../migrations/0001_schema.sql"))];

/// Dos `jimmy migrate` a la vez se pisarían creando las mismas tablas. El lock
/// los pone en fila, y el segundo ya encuentra todo aplicado. Se libera solo
/// cuando la conexión se cierra.
const MIGRATE_LOCK: i64 = 0x006a_696d_6d79;

/// El pool de la instancia. `r2d2` deja las conexiones abiertas: si no, cada
/// consulta pagaría el handshake. Cuatro alcanzan para una instancia donde el
/// que escribe es uno.
pub type Db = r2d2::Pool<PostgresConnectionManager<NoTls>>;

/// El pool, si hay `DATABASE_URL`. Sin ella jimmy sigue en archivos: el volumen
/// es la fuente de verdad hasta que la última tabla esté migrada.
pub fn from_env() -> Result<Option<Db>, String> {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        return Ok(None);
    };
    if url.trim().is_empty() {
        return Ok(None);
    }
    pool(&url).map(Some)
}

pub fn pool(url: &str) -> Result<Db, String> {
    let manager = PostgresConnectionManager::new(config(url)?, NoTls);
    r2d2::Pool::builder()
        .max_size(4)
        .connection_timeout(Duration::from_secs(10))
        .build(manager)
        .map_err(|e| format!("no pude abrir el pool: {e}"))
}

/// La url no viaja en el mensaje: puede llevar la contraseña adentro.
fn config(url: &str) -> Result<postgres::Config, String> {
    url.parse()
        .map_err(|_| "DATABASE_URL no es una url de postgres".to_string())
}

/// Aplica lo que falte y devuelve los nombres que aplicó.
///
/// Una migración ya aplicada y después editada es un error: el esquema de la
/// base y el del binario se separaron, y seguir aplicando la deja mintiendo.
pub fn migrate(url: &str) -> Result<Vec<&'static str>, String> {
    config(url)?;
    let mut client =
        postgres::Client::connect(url, NoTls).map_err(|e| format!("no pude conectar: {e}"))?;
    client
        .execute("SELECT pg_advisory_lock($1)", &[&MIGRATE_LOCK])
        .map_err(|e| e.to_string())?;
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
    fn una_base_que_no_responde_no_es_un_panic() {
        let error = pool("postgres://postgres:x@127.0.0.1:1/nada").unwrap_err();
        assert!(error.contains("no pude abrir el pool"), "{error}");
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
