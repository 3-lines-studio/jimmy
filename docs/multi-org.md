# Multi-org

Cómo se convierte este jimmy en una app multi-usuario y multi-org: varias
personas, cada una en varias orgs (la personal, la del socio, la de la
empresa), con un servicio persistente que guarda lo de todos y ejecuta en
sandboxes por org.

Se construye sobre jimmy y axe. No se forkea: `axe` ya es una librería y el
código nuevo vive en este repo, en la rama de trabajo.

## La partición

Dos estados, dos dueños:

- **Control plane** — servicio persistente en Railway con Postgres. Guarda lo
  que hay que consultar entre orgs y usuarios: orgs, usuarios, membresías,
  metadata de conversaciones y proyectos, agenda, cuotas y medidores.
- **Sandbox** — el filesystem de la org, montado por el sandbox. Guarda lo que
  solo le importa al turno que está corriendo: el workspace (repos,
  `node_modules`, `target/`), los transcripts, los archivos subidos y el
  binario de jimmy.

La regla: la DB guarda lo que se lee desde afuera del sandbox; el FS guarda lo
que solo toca el sandbox que trabaja. Los secretos no viven en ninguno de los
dos (ver heimdall, más abajo).

## Tablas

| Tabla | Qué guarda |
| --- | --- |
| `orgs` | `id`, `slug`, `name`, `plan`, `created_at` |
| `users` | `id`, `email`, `name` |
| `memberships` | `(org_id, user_id, role)`: un usuario está en varias orgs |
| `projects` | `id`, `org_id`, `name`, `created_at` |
| `conversations` | `id`, `org_id`, `project_id`, `title`, `created_by`, `last_turn_at`, `transport` |
| `schedules` | `id`, `org_id`, `conversation_id`, `project_id`, `when`/`at`/`every`, `prompt`, `target`, `silent`, `paused` |
| `sandboxes` | `id`, `org_id`, `provider`, `provider_id`, `snapshot_id`, `version`, `status`, `last_seen` |
| `turns` | el evento de uso, una fila por turno (ver Métricas) |

El transcript y el log de cada conversación no van acá: van al FS de la org,
que es donde se producen y donde sobreviven a la suspensión del sandbox.
La DB guarda el índice y la metadata.

## El adapter

El control plane no sabe de Tensorlake: habla con un trait.

```rust
trait Sandbox {
    fn ensure(&self, org: &Org) -> Handle;              // crea o despierta
    fn turn(&self, h: &Handle, req: Turn) -> Stream<Event>;
    fn suspend(&self, h: &Handle);
    fn destroy(&self, h: &Handle);
}
```

Dos implementaciones:

- `Local` — procesos en el contenedor del control plane, que es el pool de hoy.
- `Tensorlake` — la API de sandboxes, con el mismo protocolo JSONL de
  `protocol.rs` viajando por el stream del `run`.

El agente no cambia: cambia el transporte del pipe. Eso permite arrancar con
`Local` (comportamiento idéntico al actual) y mudar sin big bang.

## Un sandbox por org

No por usuario: el workspace es de la org y dos personas de la misma empresa
trabajan sobre lo mismo. Los turnos se serializan por proyecto, que es la
forma barata de que dos agentes no pisen el mismo archivo.

## La imagen y el binario

La imagen del sandbox queda congelada y sin jimmy adentro:

`debian:bookworm-slim` + `git`, `curl`, `ca-certificates`, `python3`,
`build-essential`.

Así no hay que reconstruirla por un cambio de código, y sigue arrancando en
los ~2 s medidos. El binario de jimmy vive en el FS de la org:

```
. jimmy/bin/<version>/jimmy
.jimmy/current -> bin/<version>
```

Actualizar es escribir un archivo. El control plane publica la versión nueva y
el sandbox la toma; si un turno no arranca, hay A/B con un solo reintento:

1. Se escribe `bin/<nueva>`; `current` sigue apuntando a la vieja.
2. Turno de humo: arranque y ping por el protocolo.
3. Si responde, `current` pasa a la nueva; si no, se borra y queda la vieja.
4. Cada turno registra qué versión corrió, así el rollback es volver el symlink.

## heimdall

Todo org necesita secretos, así que heimdall viene integrado por defecto: no es
una opción que alguien tenga que instalar.

heimdall ya es multi-tenant por diseño. Su API v1 (`/v1/secrets`,
`/v1/keys`, `/v1/tokens`, `/v1/audit`, `/v1/environments`) resuelve por
`project` y `env`, y sus tokens tienen alcance: `scoped()` rechaza lo que el
token no cubre. Entonces:

- Un servicio heimdall y **un proyecto por org**.
- Al crear la org se crea su proyecto y su token de administración.
- **La UI de secretos vive dentro de jimmy**: alta, baja, listado y audit
  contra la API v1, con la sesión de jimmy como identidad.
- El sandbox recibe un **token efímero** (`--ttl`, `--keys` acotadas al
  proyecto de su org) y resuelve los secretos con `heimdall run` adentro. Los
  valores nunca pasan por la DB del control plane.

## Métricas

Se instrumentan desde el diseño, no después. Una fila por turno:

```
org, user, conversation, project, sandbox, provider, version,
started_at, ended_at, cpu_seconds, ram_gb_seconds, disk_gb_seconds,
tokens_in, tokens_out, tokens_cached, tool_calls, turns, exit
```

Tres fuentes que ya existen: `machine::usage` y `memlog` para la máquina,
`end.usage` en `agent.rs` para los tokens, y el proveedor de sandboxes para su
propio consumo (que es lo que factura). El adapter es el punto donde se juntan.

Con eso salen solos los dos planes: el liviano y el power user se facturan con
la misma tabla.

## Transports

La web es el canal fuerte, pero varias orgs van a usar Slack al mismo tiempo:
las credenciales del transporte son por org (en heimdall, no en el código), y
el control plane levanta una conexión por org con su token. Telegram igual.

## Previews

El preview deja de ser un puerto local: el sandbox publica el puerto, el
control plane hace de proxy con un token por org.

## Backups

El FS de la org ya tiene snapshots. Para el control plane, backups de Postgres.
Está anotado: se resuelve a futuro y no condiciona el diseño.

## Pasos

1. El trait `Sandbox` con la implementación `Local` y el pool detrás. Cero
   cambio de comportamiento.
2. `orgs`, `users` y `memberships` en Postgres, con el auth apuntando ahí.
3. Agenda y metadata a la DB; transcripts y workspace al FS.
4. La implementación `Tensorlake`, la imagen mínima y el binario en el FS.
5. heimdall por org y su UI.
6. Cuotas, medidores y los dos planes.
