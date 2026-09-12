# Jimmy

A Telegram personal assistant. It embeds axe as a library and gives it a
machine of its own: every message it replies to is an axe agent run with the
full `read`, `write`, `edit`, and `bash` toolset on the container filesystem.

One session per Telegram chat, stored as append-only JSONL under
`$JIMMY_ROOT/chats/<chat_id>/transcript.jsonl`. Long polling, so there is no
public URL and no webhook to configure. A live status message is edited while
the agent works; the final answer is sent as a fresh message.

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

Set `TELEGRAM_ALLOWED_USER_IDS` before exposing the bot. Empty means any
Telegram user who finds the bot gets shell access to the machine.

## Model, base URL, and system prompt

Model and base URL are environment variables: `AXE_MODEL` and `AXE_BASE`. To
point at any OpenAI-compatible endpoint, set both.

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

Edit `mise.toml` or bump `WAX_VERSION` and rebuild to change the versions.
Tools added at runtime with `mise use -g` land in the image filesystem, not on
`/data`, so they do not survive a Railway redeploy.

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
src/main.rs       config, long-poll loop, per-chat locking
src/telegram.rs   Bot API client (ureq)
src/agent.rs      axe run, streaming status message, session persistence
mise.toml         global mise tool set baked into the image
SYSTEM.md         the assistant's system prompt, baked into the image
```
