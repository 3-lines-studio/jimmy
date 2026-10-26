# Jimmy

A personal assistant that lives in a web frontend, with Slack as an optional
channel. It embeds axe as a library and gives
it the machine of its own: every message it replies to is an axe agent run with
the full `read`, `write`, `edit`, and `bash` toolset on the container
filesystem.

One session per chat — a web conversation or a Slack thread — stored
under `$JIMMY_ROOT/chats/<chat_id>/` as a snapshot plus an append-only journal. Every
admitted input, assistant message, and tool result is committed and fsynced before it is
shown, so a restart resumes from the last one instead of repeating it. The web
follows the turn over an event stream; Slack listens over Socket Mode, so there
is no public URL and no webhook to configure. A running turn can be cancelled
from the web, and after two minutes the stream says what the agent is doing —
`leyendo src/web.rs`.

## Commands

Messages that start with a known command are handled before they reach the
agent:

- `/status` — context used vs. the window, model, commit, workspace.
- `/compact` — summarize the context now, without waiting for the threshold.
- `/clear` — commit a reset so the next message starts fresh; the history stays stored.
- `/help`, `/start` — list the commands.

Anything else, including unknown `/`-commands, goes to the agent.

## Attachments

An image uploaded from the web or sent to Slack is passed to the agent as an
inline image, so the model can look at it. A caption becomes the prompt text;
without a caption the image goes on its own. Non-image documents are ignored.

## Memory

There is none built in. `$JIMMY_WORKSPACE/notes/` is the agent's memory: plain
markdown it writes, edits and deletes as it sees fit, searched with `rg`. The
prompt says that much; facts are per instance, and the deploy carries none of
what another jimmy wrote.

## Workspace

`JIMMY_WORKSPACE` (`$JIMMY_ROOT/workspace`) is the agent's working directory.
Jimmy creates it at startup with a fixed layout and the default prompt
(`prompts/jimmy.md`) tells the agent to keep to it:

- `notes/` — long-lived notes and memory
- `projects/` — one directory per piece of work; code or not (a repo, a document,
  a presentation, a dataset)
- `files/` — documents to keep
- `scratch/` — temporary, safe to delete
- `state/` — the scheduler's task list and run state

`$JIMMY_ROOT/chats/<chat_id>/` holds each chat's history: `state.json` is the
latest snapshot and `journal.jsonl` the commits after it. Both are Jimmy's own
state and the agent is told to leave them alone. A chat created before the
journal existed has its `transcript.jsonl` imported on first open and renamed
to `transcript.imported.jsonl`.

## Model, base URL, and system prompt

Model and base URL are `AXE_MODEL` and `AXE_BASE`; set both to point at any
OpenAI-compatible endpoint.

```sh
AXE_BASE=https://api.deepseek.com AXE_MODEL=deepseek-chat
AXE_BASE=https://openrouter.ai/api/v1 AXE_MODEL=x-ai/grok-4.6
AXE_BASE=https://api.z.ai/api/coding/paas/v4 AXE_MODEL=glm-5.3-flash
```

`AXE_REASONING_EFFORT` sets `reasoning_effort` (`none`, `low`, `high`, `max`;
`none` turns thinking mode off). It defaults to `max`, the most reasoning DeepSeek
serves. `AXE_MAX_TOKENS` sets `max_tokens`, which counts reasoning tokens too;
`0`, the default, leaves it to the server (8K without thinking, 64K with it,
128K at `max`, which is what runs here). Every request carries the chat as
`user_id`, so context cache, content safety and scheduling stay isolated per
chat.

Axe contributes the tool list; Jimmy appends one system prompt of its own, and
then the runtime context. That prompt is
[`prompts/jimmy.md`](prompts/jimmy.md), compiled in with `include_str!`: there is
no setting for it and nothing to override at runtime. Fork the repo to change
it.

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
`/data`, so they do not survive a container redeploy.

`uv` is pointed at the volume — `UV_PYTHON_INSTALL_DIR=/data/uv/python`,
`UV_CACHE_DIR=/data/uv/cache`, `UV_PYTHON_PREFERENCE=only-managed` — so
`uv venv /data/venv` uses an interpreter uv keeps on `/data` and the venv
survives redeploys. That is what the agent is told to use for persistent Python
packages; `node_modules` inside the workspace also lives on the volume.

## Self-improvement

Jimmy can read and change its own source. The repo is public, so it clones
without credentials; `GITHUB_TOKEN` is what lets the agent push a branch and
open a pull request. `axe` is public too, so the build fetches it without
credentials. Repos live in `projects/<name>/` and changes go through a pull
request; the workflow — reuse the clone, reset to `dev`, branch,
`make fmt lint test`, push, `gh pr create --base dev` — is written for the agent
in `## Proyectos y git` of `prompts/git.md`.

Deploy branches: `dev` is the development line, `main` is production. Every
agent change lands on `dev`; promoting to `main` is a `dev → main` pull request
the owner approves. There is one running instance, and it is deployed from
`main`.

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
cp .env.example .env   # fill in the key and your user id
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

## Deploy

Jimmy runs as a container on a VPS behind a shared Caddy edge, deployed with
[`bee`](https://github.com/3-lines-studio/bee) (private). `bee.yaml` and
`compose.yaml` on `main` describe it: host `contabo-internal`, domain
`jimmy.3lines.studio`, `tls: http`, `strategy: restart` and
`web.health_path: /login`.

`strategy: restart` is deliberate: Jimmy is one process that owns `/data` and
polls the transports, so two slots would answer the same message twice.

A release is `bee build --tag <sha>` and then `bee deploy --image ...@sha256:...`
(or the same from the control plane). bee rsyncs the compose file, uploads
`.env` from the operator machine with mode 0600, recreates the container and
waits for the health and smoke checks; if they fail it restores the previous
image. Nothing deploys on push, and `.github/workflows/release.yml` only
publishes an image, on `main` and on `v*` tags.

The volume is mounted at `/data` and Jimmy uses it as `JIMMY_ROOT`, so the
sessions and the workspace land there. Set `JIMMY_ROOT` to override.

Set `JIMMY_COMMIT_SHA` in `.env`: bee does not inject it, and without it
`/status`, the runtime context and the versioned asset URLs of the web frontend
have no commit to show.

## Layout

```
src/main.rs       config, event loop, per-session locking, send/skill CLIs
src/transport/    the transport seam, with the Slack adapter
src/agent.rs      axe turn loop, runtime context, session persistence
src/schedule.rs   scheduled tasks: clean-context runs, retried and parked
src/skill.rs      the skills directory: list and load
src/markdown.rs   splitting long messages, closing and reopening code fences
bin/              the CLIs the agent gets: browse, gen-image, stats
web/              the vanilla-JS frontend, embedded with include_str!
mise.toml         global mise tool set baked into the image
prompts/jimmy.md  the system prompt, compiled in with include_str!
```

## License

MIT, see [LICENSE](LICENSE).
