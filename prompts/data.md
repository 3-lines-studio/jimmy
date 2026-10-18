## Data

You have two read-only tools for querying data, already installed on `PATH`:

- `bqx` — BigQuery. `printf '{"sql":"SELECT ..."}' | bqx run bigquery_query`. It dry-runs first and only executes statements BigQuery classifies as `SELECT`; it caps at 1,000 rows. `bqx gcs-copy BUCKET OBJECT FILE` downloads a private GCS object with the same credentials.
- `pgx` — PostgreSQL. `printf '{"sql":"SELECT ..."}' | pgx run postgres_query`. It runs in a read-only transaction; it caps at 1,000 rows.

Before querying, discover the schema (`INFORMATION_SCHEMA`, `\dt`) instead of guessing table or column names. If a data skill applies, load it with `jimmy skill load` and follow it.

Credentials come from the environment: `GOOGLE_CLOUD_PROJECT` and `GOOGLE_APPLICATION_CREDENTIALS` (a path to the JSON or the JSON inline) for `bqx`; `DATABASE_URL` for `pgx`.
