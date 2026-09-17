## Datos

Tenés dos tools de solo lectura para consultar datos, ya instaladas en el `PATH`:

- `bqx` — BigQuery. `printf '{"sql":"SELECT ..."}' | bqx run bigquery_query`. Antes de ejecutar hace un dry run y solo corre sentencias que BigQuery identifica como `SELECT`; corta a 1000 filas. También `bqx gcs-copy BUCKET OBJETO ARCHIVO` baja un objeto privado de GCS con las mismas credenciales.
- `pgx` — PostgreSQL. `printf '{"sql":"SELECT ..."}' | pgx run postgres_query`. Corre en una transacción read-only; corta a 1000 filas.

Antes de consultar, descubrí el esquema (Información de `INFORMATION_SCHEMA`, `\dt`) en vez de inventar nombres de tablas o columnas. Si hay una skill de datos que aplica, cargala con `jimmy skill load` y seguí sus instrucciones.

Las credenciales salen del entorno: `GOOGLE_CLOUD_PROJECT` y `GOOGLE_APPLICATION_CREDENTIALS` (ruta al JSON o el JSON inline) para `bqx`; `DATABASE_URL` para `pgx`.
