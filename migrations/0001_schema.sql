-- El estado de jimmy: los chats, sus dos logs, los adjuntos, las sesiones de la
-- web, la agenda y la memoria.
--
-- Los ids son ULIDs en texto: 48 bits de tiempo y 80 de azar en base32, así que
-- ordenar por el id es ordenar por cuándo se creó, y no se derivan de nada.
--
-- Todas las tablas cierran con la misma tríada, en el mismo orden: `created_at`,
-- `updated_at` y `deleted_at`. En los logs no se edita ni se borra nada, así que
-- ahí quedan sin uso, pero la forma es una sola y no hay excepciones que
-- recordar. Los índices únicos son parciales, porque una fila borrada no tiene
-- por qué bloquear el nombre.

CREATE TABLE chats (
    id                text PRIMARY KEY,
    key               text NOT NULL,
    title             text NOT NULL DEFAULT '',
    inflight_at       timestamptz,
    resume_archive_id text,
    created_at        timestamptz NOT NULL DEFAULT now(),
    updated_at        timestamptz NOT NULL DEFAULT now(),
    deleted_at        timestamptz
);
CREATE UNIQUE INDEX chats_key ON chats (key) WHERE deleted_at IS NULL;

CREATE TABLE archives (
    id         text PRIMARY KEY,
    chat_id    text NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    title      text NOT NULL DEFAULT '',
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    deleted_at timestamptz
);
CREATE INDEX archives_chat ON archives (chat_id) WHERE deleted_at IS NULL;

ALTER TABLE chats ADD CONSTRAINT chats_resume_archive
    FOREIGN KEY (resume_archive_id) REFERENCES archives(id) ON DELETE SET NULL;

-- El historial que el modelo vuelve a leer. `archive_id` en nulo es la sesión
-- viva: archivar es un UPDATE, no mover filas.
CREATE TABLE messages (
    id         text PRIMARY KEY,
    seq        bigint GENERATED ALWAYS AS IDENTITY,
    chat_id    text NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    archive_id text REFERENCES archives(id) ON DELETE CASCADE,
    entry      jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    deleted_at timestamptz
);
CREATE INDEX messages_live
    ON messages (chat_id, seq) WHERE archive_id IS NULL AND deleted_at IS NULL;
CREATE INDEX messages_archive
    ON messages (archive_id, seq) WHERE archive_id IS NOT NULL AND deleted_at IS NULL;

-- Los eventos que un cliente lee para ponerse al día: `conversation` es lo que
-- ve una persona, `worker` lo que reporta el proceso del turno.
CREATE TABLE events (
    id         text PRIMARY KEY,
    seq        bigint GENERATED ALWAYS AS IDENTITY,
    chat_id    text NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    stream     text NOT NULL,
    payload    jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    deleted_at timestamptz
);
CREATE INDEX events_chat ON events (chat_id, stream, seq) WHERE deleted_at IS NULL;

-- El blob vive en el bucket; acá queda con qué nombre lo pidió quien lo subió.
CREATE TABLE uploads (
    id           text PRIMARY KEY,
    chat_id      text NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    name         text NOT NULL,
    size         bigint NOT NULL DEFAULT 0,
    content_type text NOT NULL DEFAULT 'application/octet-stream',
    object_key   text NOT NULL,
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now(),
    deleted_at   timestamptz
);
CREATE UNIQUE INDEX uploads_name ON uploads (chat_id, name) WHERE deleted_at IS NULL;

CREATE TABLE sessions (
    id         text PRIMARY KEY,
    token      text NOT NULL UNIQUE,
    email      text NOT NULL,
    expires_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    deleted_at timestamptz
);
CREATE INDEX sessions_expiry ON sessions (expires_at) WHERE deleted_at IS NULL;

CREATE TABLE tasks (
    id           text PRIMARY KEY,
    name         text NOT NULL,
    prompt       text NOT NULL,
    when_at      timestamptz,
    at           text,
    every        text,
    target       text,
    silent       boolean NOT NULL DEFAULT false,
    paused       boolean NOT NULL DEFAULT false,
    next_run_at  timestamptz,
    last_read_at timestamptz,
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now(),
    deleted_at   timestamptz
);
CREATE UNIQUE INDEX tasks_name ON tasks (name) WHERE deleted_at IS NULL;
CREATE INDEX tasks_next ON tasks (next_run_at) WHERE NOT paused AND deleted_at IS NULL;

CREATE TABLE task_runs (
    id         text PRIMARY KEY,
    seq        bigint GENERATED ALWAYS AS IDENTITY,
    task_id    text NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    started_at timestamptz NOT NULL,
    ms         integer NOT NULL DEFAULT 0,
    answer     text NOT NULL DEFAULT '',
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    deleted_at timestamptz
);
CREATE INDEX task_runs_task ON task_runs (task_id, seq) WHERE deleted_at IS NULL;

-- Un hecho: la clave es estable y el ámbito dice hasta dónde llega. El cuerpo
-- del log es append-only: nada se borra.
CREATE TABLE facts (
    id         text PRIMARY KEY,
    scope      text NOT NULL DEFAULT '',
    key        text NOT NULL,
    kind       text NOT NULL,
    body       text NOT NULL,
    seen       bigint NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    deleted_at timestamptz
);
CREATE UNIQUE INDEX facts_key ON facts (scope, key) WHERE deleted_at IS NULL;

CREATE TABLE fact_log (
    id         text PRIMARY KEY,
    seq        bigint GENERATED ALWAYS AS IDENTITY,
    scope      text NOT NULL DEFAULT '',
    key        text NOT NULL,
    op         text NOT NULL,
    body       text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    deleted_at timestamptz
);
CREATE INDEX fact_log_key ON fact_log (scope, key, seq) WHERE deleted_at IS NULL;
