# Multi-org

Cómo se convierte este jimmy en una app multi-usuario y multi-org: varias
personas, cada una en varias orgs (la personal, la del socio, la de la
empresa), con un servicio persistente que guarda lo de todos y ejecuta en
sandboxes por org.

Se construye sobre jimmy y axe. No se forkea: `axe` ya es una librería y el
código nuevo vive en este repo, en la rama de trabajo.

## La partición

Dos estados, dos dueños:

- **Control plane** — servicio persistente en Railway con su base. Guarda lo
  que hay que consultar entre orgs y usuarios, y lo que hay que poder mostrar
  sin despertar a nadie: orgs, usuarios, membresías, sesiones, el índice de
  proyectos y conversaciones, la agenda, los medidores y el estado de cada
  sandbox.
- **Sandbox** — el filesystem de la org, montado en su sandbox, y **el
  workspace es de ahí**: los repos, `node_modules`, `target/`, los adjuntos,
  los transcripts, el perfil de Chromium y todo lo que deja el agente al correr,
  más el binario de jimmy. El control plane no es dueño de nada de eso.

La regla: la DB guarda lo que hay que consultar entre orgs; el sandbox guarda
todo lo que produce y consume el trabajo. **El control plane no abre un archivo
del workspace**: se lo pide al adapter, que hoy es local (el disco del control
plane, el layout de siempre bajo `orgs/<id>`) y mañana es el sandbox de la org.
Los secretos no viven en ninguno de los dos (ver heimdall, más abajo).

Que el control plane no toque el filesystem es lo que hace que las dos
implementaciones sean intercambiables; que hoy la local sea el disco de siempre
es lo que permite llegar ahí sin mudar nada de lo que ya existe.

## Tablas

Toda tabla arranca igual, sin excepciones: el `id` --un ulid, opaco y ordenable
por cuándo se creó--, `created_at`, `updated_at` y `deleted_at`. La columna del
identificador se llama `id` en todas: el valor es un ulid, el rol es el de
siempre. Toda columna que guarda un id termina en `_id` y nombra lo que guarda
(`org_id`, `user_id`, `personal_of_id`), así el nombre dice qué hay adentro sin
que haya que ir a mirar la definición. La baja es lógica: una fila que se fue
queda con `deleted_at` y las consultas filtran. Una tabla nueva que no cumpla
esto rompe el test que lo exige en `src/store.rs`, que además comprueba que el
id sea de tipo texto y no un entero.

| Tabla | Lo propio |
| --- | --- |
| `users` | `email`, `name` |
| `orgs` | `name`, `dir` (su directorio, con sus conversaciones y su workspace adentro), `plan` (`free` o `paid`: el sandbox llega con el pago), `personal_of_id` (el usuario dueño, si es la org personal de alguien) |
| `memberships` | `org_id`, `user_id`, `role`, únicos por par |
| `sessions` | `token`, `user_id`, `expires_at` |

Las que faltan —`schedules`, `sandboxes` y `turns`— entran con los pasos que
vienen, con el mismo encabezado.

El transcript y el log de cada conversación no van acá: van al FS de la org,
que es donde se producen y donde sobreviven a la suspensión del sandbox. Cada
org tiene **su directorio** (`orgs.dir`), y adentro está todo lo suyo —sus
`chats/`, su `workspace/`, sus `files/`—, así el aislamiento sale del sistema de
archivos y no de un `WHERE org_id = ?` que se puede olvidar. La org que se quedó
la raíz (la primera) usa el workspace de siempre; las demás arrancan con el
suyo, vacío.

`projects` y `conversations` son la excepción, y son una **copia**: el FS sigue
mandando, pero cuando el trabajo vive en un sandbox el control plane no puede
leer su volumen sin despertarlo, y armar el sidebar a fuerza de listados son
viajes de ~200 ms cada uno. El índice se sincroniza entero desde adentro —una
sola operación, `jimmy conversations --json`, que arma la lista con el mismo
código que la arma adentro— y la web la muestra sin tocar el sandbox. La lista
es lo único que se copia: los archivos no, porque son miles y cambian todo el
tiempo. El árbol y el contenido se le piden al sandbox cuando alguien los mira,
y si está dormido se despierta, igual que para el browser.

La base es SQLite en el volumen (`jimmy.db`), con el esquema armado al abrir y
sin migraciones, igual que heimdall: un proceso escribe, la concurrencia es
baja y no hay servicio nuevo que provisionar ni TLS que resolver. Es la misma
decisión que ya tomó heimdall (rusqlite bundled). Sale a Postgres el día que
haga falta más de un escritor —varias réplicas del control plane—, y para eso
la capa de datos tiene que estar sola en un módulo, que es como está.

## El adapter

El control plane no sabe de Tensorlake: le pide la máquina de una org a
`remote::ensure`, que lee su fila en `machines` —el proveedor, el nombre del
sandbox y el filesystem que monta— y lo crea si no está o lo despierta si está
dormido. Sin fila, el trabajo corre acá y no hay nada que despertar.

Esa fila elige dónde corre el turno, no dónde corren las herramientas:
con sandbox, el worker va adentro y sus herramientas son las de allá. El
trait `Machine` de axe queda para lo que el control plane tenga que leer o
escribir del volumen de una org sin despertarla —la web—, con dos
implementaciones: `Local` (este contenedor) y `Tensorlake` (su volumen por
HTTP).

El trabajo adentro de esa org lo hace una **máquina**: el volumen y el shell
juntos, que es el trait `Machine` de axe (`read`, `write`, `list`, `remove`,
`run`). El path que una herramienta lee es el mismo que ve un comando, así que
vienen juntos y apuntarlos a lugares distintos no se puede ni escribir.

Dos implementaciones:

- `Local` — este contenedor, su filesystem y sus procesos: el pool de hoy.
- `Tensorlake` — la API de sandboxes: los archivos por `/files`, los comandos
  por `/processes`, y el `Local` de adentro (el bash) como primitivo para todo
  lo que la API no sabe hacer, como leer un pedazo de un archivo grande.

El agente corre en el control plane, con sus herramientas apuntadas a la máquina
de la org: sin binario adentro del sandbox, sin versiones que publicar y sin los
secretos del modelo de ese lado.

El `Workspace` de la web, abajo, se apoya en la misma máquina: deja de saber si
el volumen está de este lado o del otro.

```rust
/// El workspace de una org, sin decir dónde está: lo que hay adentro y lo que
/// se puede hacer con eso. La web no arma caminos, pide nombres.
trait Workspace {
    fn label(&self) -> String;                       // cómo se llama, para mostrar
    fn projects(&self) -> Result<Vec<Project>, String>;
    fn conversations(&self, project: &str) -> Result<Vec<Conversation>, String>;
    fn tree(&self, project: &str, path: &str) -> Result<Vec<Entry>, String>;
    fn read_file(&self, project: &str, path: &str, limit: Option<u64>) -> Result<(Vec<u8>, u64), String>;
    fn window(&self, key: &str, end: usize) -> Result<Window, String>;
    fn read_attachment(&self, key: &str, name: &str) -> Result<Vec<u8>, String>;
    fn read_attachments(&self, key: &str, names: &[String]) -> Result<Vec<Image>, String>;
    fn write_attachment(&self, key: &str, name: &str, data: &[u8]) -> Result<String, String>;
    fn writable(&self, key: &str) -> Result<Conversation, String>;
    fn create_project(&self, name: &str) -> Result<(), String>;
    fn rename_project(&self, from: &str, to: &str) -> Result<(), String>;
    fn duplicate_project(&self, from: &str, to: &str) -> Result<(), String>;
    fn delete_project(&self, name: &str, force: bool) -> Result<(), String>;
    fn create_conversation(&self, project: &str, title: &str) -> Result<String, String>;
    fn rename_conversation(&self, key: &str, title: &str) -> Result<(), String>;
    fn delete_conversation(&self, key: &str) -> Result<(), String>;
}
```

## Cómo se le habla a Tensorlake

El control plane no usa el SDK: habla la API con HTTP. Son dos hosts y no hacen
falta dependencias nuevas (`ureq` ya estaba en el árbol; el crate `tensorlake`
arrastra Docker, PNG, Rayon y DES para dos endpoints).

- **Ciclo de vida**, en `api.tensorlake.ai/v1/namespaces/default/sandboxes`:
  `POST` crea el sandbox **y monta el filesystem de la org en el mismo pedido**;
  `GET` lista, `DELETE` termina, `POST /<id>/suspend` y `/resume` lo duermen y
  lo despiertan.
- **El sandbox**, en `<nombre>.sandbox.tensorlake.ai/api/v1`: `POST /processes`
  lanza el worker con la entrada por pipe y la salida guardada,
  `POST /processes/<pid>/stdin` le manda un comando, `GET
  /processes/<pid>/stdout/follow` devuelve los eventos por SSE y
  `DELETE /processes/<pid>` lo baja.

Medido: cada evento del stream llega con **61-70 ms** de retraso desde Railway,
que es el mismo camino que recorre un turno local. El `follow_output` del SDK de
Python, en cambio, entrega todo junto al final: por eso el adapter no lo usa.


## Un sandbox por org

No por usuario: el workspace es de la org y dos personas de la misma empresa
trabajan sobre lo mismo. Los turnos se serializan por proyecto, que es la
forma barata de que dos agentes no pisen el mismo archivo.

## Dónde corre un turno

Una org tiene su raíz y su workspace adentro de esa raíz: eso es un `Place`, y
es todo lo que hace falta para correr un turno suyo. El agente del control
plane es uno solo —el bus, los candados y el pool son los mismos— y
`Agent::at(&place)` devuelve ese mismo agente apuntado a otra org: mismo
modelo, mismos candados, otro lugar donde trabajar. La web usa ese clon en
cada request, así que el turno de una org resuelve su conversación y su log
donde corresponde y no en la raíz del control plane.

El entorno del worker es del turno, no del pool: viaja en `Sandbox::turn`
junto con la conversación y el comando. Es lo que el worker necesita para
construirse del otro lado: su raíz, su workspace, el modelo y lo que el
prompt necesita para armarse.

El turno de una org **corre adentro de su sandbox**: el control plane le pide
a `remote::ensure` que lo despierte, publica el agente en su volumen —el
binario, los prompts y las skills, con la huella del binario como versión— y
lanza el worker ahí por la API de procesos. Los archivos que el agente lee y
escribe son los de su volumen, sin viajes por HTTP, y los eventos vuelven por
el mismo protocolo de siempre: el control plane sigue escribiendo el log que
mira la web. El worker no abre la base, y no hay dos lugares: su raíz adentro
del sandbox es donde el volumen se monta (`/work`), así que `remote::al_sandbox`
traduce el path que la org tiene acá por el de adentro. Si el sandbox no
contesta, el turno se frena y lo dice: nunca se cae al disco local por las
dudas, que sería escribir lo de una org en el lugar de otra.

La agenda es de cada org y vive en la base, no en el workspace: el control
plane la lee y la escribe sin despertar a nadie. El reloj es uno y cada vuelta
reclama lo vencido —lo reclama uno solo, y ese es el que lo corre— con el
agente apuntado a la org que la pidió. Cada tarea lleva su `next_run_at`, que
se recalcula al correr, en lugar de deducir si le toca mirando el historial; y
si el proceso se muere a mitad de un turno, el reclamo viejo se suelta solo.

## La imagen y el binario

Son dos cosas y cambian por motivos distintos.

**El entorno** es la imagen del sandbox (`sandbox.dockerfile`, `make
imagen-sandbox`): el sistema, las toolchains con las que el agente trabaja en
los proyectos, y lo que necesitan sus herramientas —Chromium con su venv para
`browse`, `ffmpeg`—. Es el mismo que el del control plane: `debian:bookworm-slim`
con `git`, `curl`, `build-essential`, `chromium`, `ffmpeg`; `mise` con Rust,
Go, Node, Bun, Python y las CLI (`jq`, `rg`, `fd`, `gh`, `golangci-lint`),
heimdall, `bqx` y `pgx`; y `/opt/browse-venv` con playwright. Se construye con
el SDK (una vez por cambio del entorno, ~1 GB de snapshot) y el sandbox la
arranca en 5-13 s medidos.

**El agente** —el binario, los prompts, las skills y los CLIs `browse`,
`recall`, `stats` y `gen-image`— no va en la imagen: se publica desde el
control plane y cada sandbox lo instala en su disco, en los paths de siempre
(`/usr/local/bin/jimmy`, `/usr/local/share/jimmy/...`), la primera vez de cada
sandbox. Así una versión nueva del código no obliga a reconstruir el entorno, y
adentro hay un solo agente, el mismo que arma el turno.

La copia publicada vive en el volumen de la org (`/work/.jimmy/<huella>/`), con
la huella del binario como nombre: no hay forma de que un turno corra con el
binario de antes, y una instancia que no se acuerde de haber publicado lo
vuelve a subir sin romper nada. Cuesta la primera vez —el POC midió 11,9 MB en
1,07-8,34 s— y después cada sandbox nuevo paga sólo el `cp -a`.

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

### El descriptor

heimdall resuelve hoy por `project/env` y sus tokens matchean exacto o `*`
(`covers`), sin comodines parciales: un token no puede cubrir "todos los
proyectos de una org". Para la jerarquía del producto — org, proyecto dentro de
la org, entorno — hay que agregarle un nivel arriba:

- `secrets` y `tokens` pasan de `(project, env)` a `(org, project, env)`, que es
  también la clave primaria.
- `covers` se chequea por nivel, así un token `org=acme, project=*, env=dev`
  alcanza para todo lo de esa org en dev.
- La API v1 y la UI de heimdall suman el parámetro.

Es un cambio transversal sobre un store que ya es propio (SQLite, ~2k líneas) y
conviene hacerlo temprano, con un solo proyecto cargado y la migración barata.
Mientras no esté, el proyecto de heimdall se llama igual que la org y no se
nota.

No hace falta un nivel *usuario* aparte: la org personal de cada uno es su
scope, y quien está en varias orgs tiene un scope por org. Así el descriptor
canónico queda `org/project/env` para todas.

## Env vars

Dos mundos que no se mezclan: el control plane nunca le pasa su entorno al
sandbox, y el sandbox nunca ve las credenciales del control plane.

- **Del control plane**: la base (`DATABASE_URL`), `HEIMDALL_URL` y el token de
  administración, la API del proveedor de sandboxes, el mail y la clave de
  sesiones. No salen de ahí.
- **Del sandbox**: las del turno (`AXE_BASE`, `AXE_MODEL`, las `AXE_*`), su
  `JIMMY_ROOT` apuntando al FS de la org, su `HOME` y los secretos del
  proyecto. Nada más. Hoy la clave del modelo viaja con ellas, porque el worker
  adentro llama al modelo directo: el proxy del control plane es el paso que lo
  saca de ahí y el lugar donde caen los medidores.
- **De heimdall al sandbox**: sólo `HEIMDALL_URL` y un token efímero de su org.
  El token de administración no entra.

Hoy el worker hereda el entorno del proceso padre: `worker_env` en `agent.rs`
agrega lo del turno y deja el resto. Con multi-org eso cambia a un spawn con el
entorno vacío y sólo lo de la lista.

Regla de nombres: las variables reservadas (`PATH`, `HOME`, `JIMMY_ROOT`, las
`AXE_*`, las `HEIMDALL_*`) no se pueden pisar desde un secreto de usuario; el
alta las rechaza.

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

El cobro es lo último y va por seat: primero se mide todo, y con la tabla llena
se decide el precio. La cuota se agrega por `(org, user)`, que es la unidad que
se cobra.

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

El orden manda: cada paso deja algo andando y verificable antes del siguiente.

1. **Hecho** — El trait `Sandbox` con la implementación `Local` y el pool
   detrás. Cero cambio de comportamiento.
2. **Hecho** — `orgs`, `users`, `memberships` y `sessions` en la base, con el
   auth apuntando ahí.
3. **Hecho** — La agenda a la DB, con cada turno corriendo en el workspace de su
   org.
4. **Hecho** — La máquina remota: la costura de axe (el trait `Machine`,
   `Local`, `build_tools_on`), el cliente de Tensorlake con los archivos y el
   shell, y el `impl Machine` que los junta.
5. **Hecho** — La compuerta: `el_bash_remoto_se_comporta_igual` corre el mismo
   comando por las dos vías y compara los textos.
6. **Hecho** — El turno adentro del sandbox: la fila de `machines` elige dónde
   corre, `remote::ensure` lo despierta antes del turno, el agente se publica en
   el volumen y el worker arranca ahí por la API de procesos. Los archivos que
   toca son los suyos y la agenda corre igual que la web. Sin fila, el trabajo
   corre acá.
7. **Hecho** — La web leyendo el volumen: la lista sale del índice —sin
   despertar a nadie— y los archivos y los adjuntos se le piden al sandbox en
   vivo, que para eso se despierta, como con el browser. Lo que la web escribe lo
   escriben las mismas funciones que corren adentro, por el CLI del agente, y la
   copia del índice se refresca en el momento. El log de cada conversación lo
   sigue escribiendo el control plane, que es donde se produce.
8. **Hecho** — Los adjuntos de los dos lados: lo que sube la web llega al
   volumen, donde lo ve el agente, y lo que manda el asistente espera en la cola
   del chat —que vive adentro, porque la escribe el CLI— y el control plane la
   lee de ahí y la vacía. Probado de punta a punta con un archivo de verdad.
9. **Hecho** — El alta de una org: con el plan en `paid` se crea su filesystem
   —lo hace el SDK de Tensorlake, por `uv run --with tensorlake python
   deploy/filesystems.py crear`, que es el único que sabe hablar con ese
   servicio— y queda su fila en `machines`. El filesystem lleva el nombre de la
   fila: es el volumen que el sandbox monta y nada más. Idempotente. Falta la
   pasarela: hoy el plan se marca a mano (`jimmy orgs plan <mail> paid`).
10. **Hecho** — El entorno del agente adentro del sandbox: `sandbox.dockerfile`
    con las toolchains, Chromium con su venv y `ffmpeg`, en una imagen que se
    registra aparte del código. El agente sigue llegando publicado.
11. **El proxy del modelo** en el control plane: la clave deja de viajar al
    sandbox y ahí caen los medidores.
12. heimdall por org y su UI.
13. Cuotas, medidores y los dos planes.
14. Los transports por org (Slack y Telegram con sus credenciales), los previews
    y los backups.
