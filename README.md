# Jimmy

A Telegram and Slack personal assistant. It embeds axe as a library and gives
it a machine of its own: every message it replies to is an axe agent run with
the full `read`, `write`, `edit`, and `bash` toolset on the container
filesystem.

One session per chat — a Telegram chat, or a Slack thread — stored as
append-only JSONL under `$JIMMY_ROOT/chats/<chat_id>/transcript.jsonl`. Telegram
listens by long polling and Slack over Socket Mode, so there is no public URL
and no webhook to configure. Sending a message shows one `pensando`
placeholder, which is replaced by the reply when the agent finishes. There is no
per-token streaming, so the chat does not flicker. That placeholder carries a
stop button: pressing it cancels the turn in progress. On a turn that runs long,
the same message says what the agent is doing — `leyendo src/web.rs` — and
refreshes itself at most every two minutes.

Replies are converted from Markdown to Telegram HTML: bold and italic, inline
code and fenced blocks, links, blockquotes, and monospaced tables. If Telegram
rejects the markup, the message falls back to plain text. Long replies are split
on line boundaries, with code fences closed and reopened across chunks.

## Commands

Messages that start with a known command are handled before they reach the
agent:

- `/status` — context used vs. the window, model, commit, workspace.
- `/compact` — summarize the context now, without waiting for the threshold.
- `/clear` — archive this chat's transcript so the next message starts fresh.
- `/help`, `/start` — list the commands.

Anything else, including unknown `/`-commands, goes to the agent.

## Attachments

Sending a photo, or an image sent as a file, attaches it to the prompt: the
file is downloaded from Telegram and passed to the agent as an inline image, so
the model can look at it. A caption becomes the prompt text; without a caption
the image goes on its own. Non-image documents are ignored.

Voice notes are transcribed with Groq (`whisper-large-v3`) when
`TRANSCRIBE_API_KEY` is set, up to 300 seconds. The transcript is echoed back
and used as the prompt. Without the key, voice notes are rejected. Only
Telegram voice notes are handled; audio sent as a document is ignored.

## Memory

A fact lives on its own, one entry per file, and where it lives says how far it
reaches. `$JIMMY_WORKSPACE/notes/memory/` holds the cross-cutting facts —who the
user is, the map of projects, the platform, how memory itself works— and
`notes/projects/<project>.md` holds the ones that belong to a project.
`jimmy memo add <key> <kind> <text>` writes one.

The prompt gets every cross-cutting fact plus the two newest of the project the
conversation belongs to. The rest is not lost: it stays in its file, and `jimmy
memo show <key>` or `rg` bring it back. Because a new fact has an owner, it never
competes with what was already there, and that is what keeps the prompt small: a
project's manual does not sit in front of every conversation.

Each entry starts with `## key · kind · YYYY-MM-DD`. The key is what makes an
updated fact replace the old one instead of duplicating it, so an entry is a
topic, not a line in a log. `notes/memory.jsonl` is append-only and holds every
state a fact went through: `jimmy memo sync` diffs the facts against it, appends
the changes and reports what it saw, and `jimmy memo miss` records a memory that
failed to surface. `jimmy memo list` prints the files and their keys. Nothing is
ever deleted.

Those commands write one JSON event per run to
`$JIMMY_WORKSPACE/state/memory-events.jsonl`, next to the scheduler's state.
`stats` reads it and appends the memory state to its report: how many facts,
the syncs, the renders and misses.

### Moving an instance to facts

The binary keeps reading `notes/memory.md` while `notes/memory/` does not exist,
so an upgrade needs no migration, and changing your mind is one `rm -rf` away.
Per instance:

1. `jimmy memo migrate` splits `notes/memory.md` into facts, one file per key:
   keys without a slash to `notes/memory/`, the rest to
   `notes/projects/<family>.md`.
2. Fix what landed in the wrong place: a key that names a project without a
   slash (`paper`) belongs in `notes/projects/paper.md`, and a catch-all entry
   like `decisiones-vigentes` is worth splitting into its own topics.
3. `jimmy memo list` to check, then `jimmy memo sync`.
4. Drop `jimmy memo demote` from that instance's `state/schedule.toml`: the
   command is gone.

Facts are per instance: the deploy carries the mechanism and the prompt, not
what another jimmy learned.

## Scheduled tasks

`$JIMMY_WORKSPACE/state/schedule.toml` holds tasks the agent runs on a clock. A
thread wakes every 60 seconds, re-reads the file, and runs whatever is due.
Each run is a fresh agent run in a clean context — the system prompt and the
task's `prompt`, nothing else — and the reply is sent to the task's chat. Runs
are not written to the chat transcript.

```toml
[[task]]
name = "morning-report"
chat = 123456789
at = "09:00"
prompt = "Summarize what is still pending."
```

Exactly one schedule key per task: `when = "YYYY-MM-DDTHH:MM"` runs once, `at =
"HH:MM"` runs daily, `every = "30m"` runs on an interval (`s`, `m`, `h`, `d`).
Times are local: UTC plus `JIMMY_TZ_OFFSET` hours. A one-shot task is removed
from the file once it fires; a daily task fires once per local date; an interval
task fires once the interval has elapsed since its last run, so a restart
catches up on a missed run. A task that fails reports the error to its chat,
and each task is capped at 6 runs per hour.

A task with `silent = true` only speaks when it has something to say: it skips
the progress placeholder and an empty reply is not sent (the usual `✅ listo`
fallback does not apply). It is meant for watchdogs that should report failures
and stay quiet otherwise. Anything the reply does contain is sent as usual.

The file is meant to be edited by the agent: ask it to schedule something and it
appends a block. The tick picks it up without a restart.

## Config

| Variable | Default | Meaning |
| --- | --- | --- |
| `JIMMY_TRANSPORT` | `telegram` | `telegram` or `slack` |
| `TELEGRAM_BOT_TOKEN` | — | required by the `telegram` transport |
| `SLACK_BOT_TOKEN` | — | required by the `slack` transport (bot token, `xoxb-`) |
| `SLACK_APP_TOKEN` | — | required by the `slack` transport (app-level token, `xapp-`) |
| `OPENAI_API_KEY` | — | required; DeepSeek (or any OpenAI-compatible) key |
| `TRANSCRIBE_API_KEY` | empty | Groq key for voice transcription; empty rejects voice notes |
| `GEMINI_API_KEY` | empty | Google AI Studio key for the `gen-image` tool; empty disables it |
| `AXE_BASE` | `https://api.deepseek.com` | API base URL |
| `AXE_MODEL` | `deepseek-flash` | model name |
| `AXE_CONTEXT_WINDOW` | `1000000` | compaction threshold |
| `JIMMY_ROOT` | `$RAILWAY_VOLUME_MOUNT_PATH` or `/data` | sessions and workspace root |
| `JIMMY_WORKSPACE` | `$JIMMY_ROOT/workspace` | directory the tools run in |
| `JIMMY_SKILLS` | `$JIMMY_ROOT/skills` | first directory with Agent Skills; the builtin one comes after (see below) |
| `JIMMY_ALLOWED_USER_IDS` | empty | comma-separated allowlist; falls back to `TELEGRAM_ALLOWED_USER_IDS`; empty means anyone |
| `JIMMY_WEB_PORT` | empty | port for the web frontend; empty means there is no web |
| `JIMMY_WEB_EMAILS` | empty | comma-separated mails that can ask for a link; empty means nobody |
| `RESEND_API_KEY` | empty | Resend key that sends the link; empty shows the link on screen |
| `JIMMY_WEB_FROM` | empty | sender of that mail, e.g. `Jimmy <jimmy@ejemplo.com>` |
| `JIMMY_WEB_URL` | `https://<host>` | public URL the link points to |
| `JIMMY_WEB_DEV` | empty | `1` returns the link in the response instead of mailing it; development and tests only |
| `JIMMY_TZ_OFFSET` | `0` | hours added to UTC for `schedule.toml` times |
| `JIMMY_PROMPT` | the default list of fragments | comma-separated fragment names, in order |
| `JIMMY_VARS` | empty | comma-separated `clave=valor` pairs for fragment placeholders |
| `JIMMY_COMMIT_SHA` | empty | commit shown in `/status` and the runtime context, for runs outside Railway |
| `GITHUB_TOKEN` | empty | fine-grained PAT so the agent can clone/push and open PRs |
| `RAILWAY_VOLUME_MOUNT_PATH` | injected | Railway's volume mount path; the default for `JIMMY_ROOT` |
| `RAILWAY_GIT_COMMIT_SHA` | injected | Railway sets this; the commit shown in `/status` |
| `RAILWAY_PROJECT_ID` | injected | Railway sets this; marks the runtime context as Railway |

Set `JIMMY_ALLOWED_USER_IDS` before exposing the bot. Empty means any user who
finds the bot gets shell access to the machine. To find your own id, put any
placeholder in the list, send the bot a message and read the
`jimmy: ignoré un mensaje de <id>` line it logs.

### Web

With `JIMMY_WEB_PORT` set, Jimmy also serves the same conversations the
transports have, plus the ones created in the browser. There are no passwords:
whoever is in `JIMMY_WEB_EMAILS` asks for a link, the link arrives by mail
(Resend, with `RESEND_API_KEY` and `JIMMY_WEB_FROM`) and lasts fifteen minutes
and one use. Without a key nobody gets in: `JIMMY_WEB_DEV=1` returns the link in
the response instead, which is how it works locally and in the tests. Sessions
live in `$JIMMY_ROOT/sessions.json`, last thirty days, and survive a redeploy. Conversations that come from a transport show up read-only.

An image attached in the composer is uploaded as it is picked (10 MB per file) to
`$JIMMY_ROOT/chats/<key>/uploads/`, and the message carries the file name, not
the bytes: the log keeps the name and `GET /api/file` serves the file, so the
image is still there after a reload and on another device. A message can be just
an image, with no text. The images Jimmy sends with `jimmy send` land in the same
place and are shown the same way, so a chart or a screenshot it made appears in
the conversation.

The page is quiet on purpose: the reply is the content, and what the agent did
on the way is one folded line per run of steps — `6 pasos · 1m 51s` — which
opens into the individual tool calls. A tool that fails opens its group and says
so in red. It follows the system theme; the button in the sidebar overrides it
and remembers.

Searching the sidebar looks through what was said in every conversation, and
each result opens its transcript with the matches marked.

### Previews

`jimmy preview start <name> --cmd 'command' [--cwd dir]` runs a project as a
child of the main process and serves it at `/preview/<name>/` on the web port,
behind the same session as the rest of the UI: no extra domain, no second
certificate. A preview is never an orphan, which is what keeps the reaper's
hands off it. The command gets `PORT` and `PREVIEW_NAME` in its environment, and
the last lines of its output land in
`$JIMMY_WORKSPACE/state/previews/<name>.log`.

A preview runs bare, the way it would on your machine. The proxy strips the
prefix on the way in and puts it back on everything that comes out pointing at
the root — HTML attributes, CSS `url()`, JavaScript string literals and
redirects — so nothing has to know where it lives. A streamed body, a binary or
a WebSocket upgrade are copied raw.

What the rewrite cannot see: a JSON payload that carries paths, or a path built
at runtime by the app. Start the project with its strict-port flag so it cannot
drift to another port.

A preview that nobody visits takes itself down after thirty minutes: every
request through the proxy resets that clock, so looking at it or iterating on it
keeps it up. A redeploy takes the rest. They are for looking at work in progress,
not for hosting. `jimmy preview list` shows what is up — and how long since the
last visit — and `jimmy preview stop <name>` takes one down. Names are slugs:
lowercase, digits and dashes.

The sidebar lists the running previews above the projects, each one a link that
opens it in a new tab. They come in `GET /api/state` as `previews`, so the panel
refreshes with the rest of the sidebar.

### Recipes

A project's start-up line is worth writing once. `$JIMMY_WORKSPACE/state/previews.toml`
holds one recipe per name, and then `jimmy preview start <name>` needs no `--cmd`:

```toml
[bifrost]
about = "the App Router demo: Vite build, Go SSR"
cwd = "projects/bifrost/example/app-router-demo"
cmd = "make dev"
```

`cwd` is relative to the workspace and defaults to it. `about` is only for the
listing. `jimmy preview recipes` prints them all with their command, and a
`--cmd` given on the spot wins over the recipe, so one-off previews still work.
When a name has no recipe and no command, the error lists the recipes that do
exist instead of just saying no.

The command runs with a clean environment: `PATH`, `HOME`, `LANG`, `TZ`, plus
`PORT` and `PREVIEW_NAME`. It does not inherit jimmy's own environment, so a
preview cannot read jimmy's secrets by accident — what a project needs, it
declares in its own recipe, in plain sight.

### Slack app

The `slack` transport uses Socket Mode, so no public URL is needed. In the app:

- Enable Socket Mode and create an app-level token with the `connections:write`
  scope (`SLACK_APP_TOKEN`).
- Bot scopes: `app_mentions:read`, `im:history`, `chat:write`, `files:read`,
  `files:write`.
- Event subscriptions: `app_mention` and `message.im`.
- The bot token goes in `SLACK_BOT_TOKEN`. The allowlist holds Slack user ids.

A channel mention opens a thread and jimny answers there: one thread is one
session. A direct message is a single continuous session.

## Skills

A skill is a folder of instructions and bundled files the agent loads on
demand, so they stay out of the context until a task matches. Each skill is
`<dir>/<name>/SKILL.md`, with `name` and `description` in its frontmatter:

```
skills/charts/SKILL.md
skills/charts/render_chart.py
```

`skills/` in the repo ships in the image at `/usr/local/share/jimmy/skills`, so
every instance gets them from a deploy. `JIMMY_SKILLS` (default
`$JIMMY_ROOT/skills`, on the volume) comes first and shadows the builtin ones:
that is where an instance keeps its own. The prompt carries the index —one line
per skill, from the frontmatter— and the agent loads one with
`jimmy skill load <name>`. `jimmy skill list` does the same by hand.

## Data tools

The image ships `bqx` (read-only BigQuery) and `pgx` (read-only PostgreSQL), the
same CLI tools the AX ecosystem uses:

```sh
printf '{"sql":"SELECT 1"}' | bqx run bigquery_query
printf '{"sql":"SELECT 1"}' | pgx run postgres_query
```

Both stop at 1,000 rows; `bqx` dry-runs and only executes what BigQuery
classifies as `SELECT`, `pgx` runs in a read-only transaction. `bqx gcs-copy
BUCKET OBJECT FILE` downloads a private GCS object. They are inert until an
instance adds the `data` prompt fragment (`JIMMY_PROMPT=...,data,...`) and its
credentials: `GOOGLE_CLOUD_PROJECT` and `GOOGLE_APPLICATION_CREDENTIALS` for
`bqx`, `DATABASE_URL` for `pgx`. Versions are pinned in the `Dockerfile`
(`BQX_VERSION`, `PGX_VERSION`).

## Content

The `contenido` prompt fragment, opt-in via `JIMMY_PROMPT`, expects a content
operation under `projects/contenido/`: `marca/` (tone, product, audience,
references), `assets/`, `templates/` (HTML publication layouts) and `semanas/`
(one markdown per week). It tells the agent to read the brand, propose angles,
fill a template, render it with `browse --shot` and preview it with
`jimmy send`. The image ships no brand and no templates: each instance provides
its own, usually by cloning a private repo there.

## Workspace

`JIMMY_WORKSPACE` (`$JIMMY_ROOT/workspace`) is the agent's working directory.
Jimmy creates it at startup with a fixed layout and `prompts/workspace.md` tells
the agent to keep to it:

- `notes/` — long-lived notes and memory
- `projects/` — one directory per piece of work; code or not (a repo, a document,
  a presentation, a dataset)
- `files/` — documents to keep
- `scratch/` — temporary, safe to delete
- `state/` — the scheduler's task list and run state

`$JIMMY_ROOT/chats/<chat_id>/transcript.jsonl` holds each chat's history. It is
Jimmy's own state and the agent is told to leave it alone.

## Model, base URL, and system prompt

Model and base URL are `AXE_MODEL` and `AXE_BASE`; set both to point at any
OpenAI-compatible endpoint.

```sh
AXE_BASE=https://api.deepseek.com AXE_MODEL=deepseek-chat
AXE_BASE=https://openrouter.ai/api/v1 AXE_MODEL=x-ai/grok-4.6
AXE_BASE=https://api.z.ai/api/coding/paas/v4 AXE_MODEL=glm-5.3-flash
```

The system prompt is assembled from fragments. Axe contributes the tool list;
Jimmy appends the fragments it is told to, in order, separated by a blank line,
then the runtime context and the memory.

`JIMMY_PROMPT` is a comma-separated list of fragment names. The default is
`identidad,estilo,codigo,jimmy,herramientas,dev,workspace,memoria,agenda,git`.
Each name resolves to `<name>.md`, first under `$JIMMY_ROOT/prompts` and then
under the image's `/usr/local/share/jimmy/prompts`. A name with no file is a
startup error, so a typo fails loudly instead of dropping a fragment in silence.

The image ships the default fragments. Override one by dropping a file with the
same name in `$JIMMY_ROOT/prompts`; that copy wins. Swap the set with
`JIMMY_PROMPT`:

```sh
JIMMY_PROMPT=identidad,estilo,herramientas,workspace,memoria,agenda
```

A fragment may carry `{{clave}}` placeholders. `JIMMY_VARS` supplies them as
comma-separated `clave=valor` pairs. The default fragments use `{{usuario}}` and
`{{asistente}}`:

```sh
JIMMY_VARS="usuario=Don Berti,asistente=Jimmy"
```

A placeholder with no value is a startup error. Single braces are left alone, so
a fragment can carry JSON like `{"action":"goto"}`.

The list and the vars are read once at startup and the assembled prompt does not
change while the process lives, so the model's prefix cache stays warm.

## Tools

The image ships:

- `search` and `fetch`, from axe itself: DuckDuckGo and a page-to-Markdown
  reader. Chromium is installed, so pages that build themselves with
  JavaScript get rendered. `browse` drives that same Chromium when the agent
  needs a session, a click, or a screenshot.
- mise, with a global config copied from `mise.toml` to
  `/root/.config/mise/config.toml`: `go`, `node`, `python`, `bun`, `uv`,
  `github-cli` (`gh`), `jq`, `ripgrep` (`rg`), `fd`, `golangci-lint`, `rust`
  and `mr-boxington`. The shims live in `/root/.local/share/mise/shims` and are
  on `PATH` ahead of `/root/.cargo/bin`, so Cargo commands go through `mbx`.
- Rust nightly with `cargo` (minimal profile, `rustfmt` and `clippy`) comes
  from that mise config, and `mr-boxington` caches builds across projects, so
  the agent can build and test itself. `git` is configured to authenticate to
  GitHub through `gh`, which reads `GITHUB_TOKEN`.

Edit `mise.toml` and rebuild to change the versions.
Tools added at runtime with `mise use -g` land in the image filesystem, not on
`/data`, so they do not survive a Railway redeploy.

`uv` is pointed at the volume — `UV_PYTHON_INSTALL_DIR=/data/uv/python`,
`UV_CACHE_DIR=/data/uv/cache`, `UV_PYTHON_PREFERENCE=only-managed` — so
`uv venv /data/venv` uses an interpreter uv keeps on `/data` and the venv
survives redeploys. That is what the agent is told to use for persistent Python
packages; `node_modules` inside the workspace also lives on the volume.

## Self-improvement

Jimmy can read and change its own source. The repo is private, so
`GITHUB_TOKEN` is what lets it clone and push; `axe` is public, so the build
fetches it without credentials. Repos live in `projects/<name>/` and changes go
through a pull request; the workflow — reuse the clone, reset to `dev`, branch,
`make fmt lint test`, push, `gh pr create --base dev` — is written for the agent
in `## Proyectos y git` of `prompts/git.md`.

Deploy branches: `dev` is the development line and backs the owner's instance,
`main` is production and backs the personal instance. Every agent change lands
on `dev`; promoting to `main` is a `dev → main` pull request the owner approves.
So point each Railway service at the branch it should track — the personal
service at `main`, the owner's at `dev` — and Railway redeploys that branch on
push.

`GITHUB_TOKEN` is a fine-grained PAT for this repo with **Contents: RW** and
**Pull requests: RW**.

Guardrails:

- `git config --system` sets a `Jimmy` commit identity and the credential
  helper; `GIT_TERMINAL_PROMPT=0` makes git fail instead of hanging.
- Protect `main` and `dev` (require a pull request) so the flow is enforced,
  not just requested by the prompt.
- The agent runs with unsandboxed bash, so a leaked `GITHUB_TOKEN` is the blast
  radius; scope it to this repo only.

## Build and run

```sh
cp .env.example .env   # fill in the token, key, and your user id
make build
make run
```

Requires Docker: the `Makefile` targets wrap `docker build` and `docker run`.

`make build` runs `docker build -t jimmy .`. axe is a pinned git dependency, so
the build fetches it from GitHub. The build must use nightly Rust because axe's
manifest declares a nightly `cargo-features` entry; the image installs nightly
with mise.

`make run` mounts `./data` at `/data`, so sessions and the workspace persist
across restarts.

To follow a newer axe, bump the `rev` in `Cargo.toml`:

```sh
git -C ../axe rev-parse origin/main   # copy the sha into Cargo.toml
cargo build --release                 # refreshes Cargo.lock
```

## Railway

Deploy as a worker (no exposed port). Railway builds from the `Dockerfile` and
fetches axe from GitHub.

For persistence, attach a Railway volume to the service and choose a mount
path. Railway passes the path as `RAILWAY_VOLUME_MOUNT_PATH`, and Jimmy uses it
as `JIMMY_ROOT`, so the sessions and the workspace land on the volume. Set
`JIMMY_ROOT` to override.

Attach the volume from the service dashboard (Volumes > New Volume) or the CLI:

```sh
railway volume add --mount-path /data
```

Caveats: a service can have only one volume, replicas cannot be used with a
volume, and each redeploy briefly stops the service to avoid concurrent mounts.
The image runs as root, so no `RAILWAY_RUN_UID` tuning is needed.

## Layout

```
src/main.rs       config, event loop, per-session locking, memo/send/skill CLIs
src/transport/    the transport seam, with the Telegram and Slack adapters
src/agent.rs      axe turn loop, runtime context, session persistence
src/audio.rs      voice transcription via Groq
src/schedule.rs   scheduled tasks, clean-context runs
src/skill.rs      the skills directory: list and load
src/markdown.rs   Markdown to Telegram HTML, message splitting
src/memo.rs       the memory facts: render, sync, add, migrate, miss
src/prompt.rs     assemble the system prompt from fragments
bin/              the CLIs the agent gets: browse, gen-image, recall, stats
web/              the browser frontend, embedded with include_str!
mise.toml         global mise tool set baked into the image
prompts/          system prompt fragments, baked into the image
```
