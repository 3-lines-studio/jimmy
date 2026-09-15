# Jimmy

A Telegram personal assistant. It embeds axe as a library and gives it a
machine of its own: every message it replies to is an axe agent run with the
full `read`, `write`, `edit`, and `bash` toolset on the container filesystem.

One session per Telegram chat, stored as append-only JSONL under
`$JIMMY_ROOT/chats/<chat_id>/transcript.jsonl`. Long polling, so there is no
public URL and no webhook to configure. Sending a message shows one `pensando`
placeholder, which is replaced by the reply when the agent finishes. There is no
per-token streaming, so the chat does not flicker.

Replies are converted from Markdown to Telegram HTML: bold and italic, inline
code and fenced blocks, links, blockquotes, and monospaced tables. If Telegram
rejects the markup, the message falls back to plain text. Long replies are split
on line boundaries, with code fences closed and reopened across chunks.

## Commands

Messages that start with a known command are handled before they reach the
agent:

- `/status` — context used vs. the window, model, commit, workspace.
- `/clear` — archive this chat's transcript so the next message starts fresh.
- `/help`, `/start` — list the commands.

Anything else, including unknown `/`-commands, goes to the agent.

## Attachments

Sending a photo, or an image sent as a file, attaches it to the prompt: the
file is downloaded from Telegram and passed to the agent as an inline image, so
the model can look at it. A caption becomes the prompt text; without a caption
the image goes on its own. Non-image documents are ignored.

## Memory

Long-term memory has two levels. Level 1 is `$JIMMY_WORKSPACE/notes/memory.md`:
a plain Markdown file, written by the agent with `read` and `edit`, injected
whole into the prompt on every message with a 16 KiB cap. Level 2 is
`notes/memory.jsonl`: append-only, holding every state a level-1 entry ever had,
searched with `rg`.

Each level-1 entry starts with `## key · kind · YYYY-MM-DD`. The key is what
makes an updated fact replace the old one instead of duplicating it, so an
entry is a topic, not a line in a log. `jimmy memo sync` diffs level 1 against
level 2, appends the changes and reports what it saw; `jimmy memo demote` moves
the oldest entries down when level 1 outgrows the budget; `jimmy memo miss`
records a memory that failed to surface (`jimmy memo miss "..."`). Nothing is
ever deleted.

Both commands write one JSON event per run to
`$JIMMY_WORKSPACE/state/memory-events.jsonl`, next to the scheduler's state.
`stats` reads it and appends the memory state to its report: level-1 size,
syncs, demotions, truncated renders and misses.

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

The file is meant to be edited by the agent: ask it to schedule something and it
appends a block. The tick picks it up without a restart.

## Config

| Variable | Default | Meaning |
| --- | --- | --- |
| `TELEGRAM_BOT_TOKEN` | — | required |
| `OPENAI_API_KEY` | — | required; DeepSeek (or any OpenAI-compatible) key |
| `AXE_BASE` | `https://api.deepseek.com` | API base URL |
| `AXE_MODEL` | `deepseek-flash` | model name |
| `AXE_CONTEXT_WINDOW` | `1000000` | compaction threshold |
| `JIMMY_ROOT` | `$RAILWAY_VOLUME_MOUNT_PATH` or `/data` | sessions and workspace root |
| `JIMMY_WORKSPACE` | `$JIMMY_ROOT/workspace` | directory the tools run in |
| `TELEGRAM_ALLOWED_USER_IDS` | empty | comma-separated allowlist; empty means anyone |
| `JIMMY_TZ_OFFSET` | `0` | hours added to UTC for `schedule.toml` times |
| `GITHUB_TOKEN` | empty | fine-grained PAT so the agent can clone/push and open PRs |

Set `TELEGRAM_ALLOWED_USER_IDS` before exposing the bot. Empty means any
Telegram user who finds the bot gets shell access to the machine.

## Workspace

`JIMMY_WORKSPACE` (`$JIMMY_ROOT/workspace`) is the agent's working directory.
Jimmy creates it at startup with a fixed layout and `SYSTEM.md` tells the agent
to keep to it:

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

The system prompt is axe's, not a Jimmy-specific one. Jimmy calls
`axe::system_prompt`, the same function the `axe` binary uses: axe's built-in
prompt plus the `SYSTEM.md` that axe reads from its config directory.

This repo ships `SYSTEM.md`. The Dockerfile copies it to
`/root/.config/axe/SYSTEM.md` and sets `XDG_CONFIG_HOME=/root/.config`, so the
image uses it. Edit the file and rebuild to change it, or mount over it at
runtime:

```sh
docker run ... -v $(PWD)/SYSTEM.md:/root/.config/axe/SYSTEM.md jimmy
```

## Tools

The image ships:

- `wax` for fetching pages as Markdown, pinned by the `WAX_VERSION` build arg.
  Chromium is installed and `WAX_NO_SANDBOX=1` is set, so pages that need
  rendering work inside the container. The system prompt already tells the
  agent to fetch with `wax <url>`.
- mise, with a global config copied from `mise.toml` to
  `/root/.config/mise/config.toml`: `go`, `node`, `python`, `bun`, `uv`,
  `github-cli` (`gh`), `jq`, `ripgrep` (`rg`), `fd`, `golangci-lint`. The
  shims live in `/root/.local/share/mise/shims` and are on `PATH`.
- Rust nightly with `cargo` (rustup, minimal profile) in `/root/.cargo`, so the
  agent can build and test itself. `git` is configured to authenticate to
  GitHub through `gh`, which reads `GITHUB_TOKEN`.

Edit `mise.toml` or bump `WAX_VERSION` and rebuild to change the versions.
Tools added at runtime with `mise use -g` land in the image filesystem, not on
`/data`, so they do not survive a Railway redeploy.

## Self-improvement

Jimmy can read and change its own source. The repo is private, so
`GITHUB_TOKEN` is what lets it clone and push; `axe` is public, so the build
fetches it without credentials. Repos live in `projects/<name>/` and changes go
through a pull request; the workflow — reuse the clone, reset to `main`, branch,
`make fmt lint test`, push, `gh pr create` — is written for the agent in
`## Proyectos y git` of `SYSTEM.md`. You review and merge, and Railway redeploys
`main`.

`GITHUB_TOKEN` is a fine-grained PAT for this repo with **Contents: RW** and
**Pull requests: RW**.

Guardrails:

- `git config --system` sets a `Jimmy` commit identity and the credential
  helper; `GIT_TERMINAL_PROMPT=0` makes git fail instead of hanging.
- Enable branch protection on `main` (require a pull request) so the flow is
  enforced, not just requested by the prompt.
- The agent runs with unsandboxed bash, so a leaked `GITHUB_TOKEN` is the blast
  radius; scope it to this repo only.

## Build and run

```sh
cp .env.example .env   # fill in the token, key, and your user id
make build
make run
```

`make build` runs `docker build -t jimmy .`. axe is a pinned git dependency, so
the build fetches it from GitHub. The build must use nightly Rust because axe's
manifest declares a nightly `cargo-features` entry; the image installs nightly
with rustup.

`make run` mounts `./data` at `/data`, so sessions and the workspace persist
across restarts.

To follow a newer axe, bump the `rev` in `Cargo.toml`:

```sh
git -C ../axe rev-parse origin/main   # copy the sha into Cargo.toml
cargo +nightly build --release        # refreshes Cargo.lock
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
src/main.rs       config, long-poll loop, per-chat locking, memo CLI
src/telegram.rs   Bot API client (ureq)
src/agent.rs      axe turn loop, runtime context, session persistence
src/audio.rs      voice transcription via Groq
src/schedule.rs   scheduled tasks, clean-context runs
src/markdown.rs   Markdown to Telegram HTML, message splitting
src/memo.rs       the two-level memory: sync, demote, miss
bin/              the CLIs the agent gets: search, recall, browse,
                  send-media, stats
mise.toml         global mise tool set baked into the image
SYSTEM.md         the assistant's system prompt, baked into the image
```
