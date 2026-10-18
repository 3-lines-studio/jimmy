# Durable jimmy and axe

Concepts adapted from `pi/packages/durable`, not its API. The goal: a crash at
any instant loses no admitted input and repeats no side effect already
committed.

## The one rule

A conversation has one commit line. An entry, a document or a task change is
visible only after its commit is stored. Everything else — UI events, status
text, search snippets — is a projection rebuilt from commits.

Invariants:

1. One commit is atomic across entries, documents and tasks of a conversation.
2. Nothing is published before its commit settles.
3. Entries and ids are immutable and never reused.
4. External effects never run inside the commit, and a tool declares whether it
   may run again.
5. A storage error fails the run; it is not retried over a half-written line.

## Concepts

| pi durable | jimmy + axe |
| --- | --- |
| Session / Harness | a conversation's journal plus the worker that leases it |
| Conversation | `chats/<key>` |
| Entry | `Entry`, immutable, id = commit seq |
| Commit | `journal.commit(change)` |
| Document | `docs`: `agent`, `live`, `usage`, `inbox` |
| Task | `Task` state machine: generation, tool, schedule |
| Submission | admitted input with a request id and a settlement |
| Turn / run | axe `run_stream` turn; a run goes from input to answer |
| Registry / extension | axe tools and jimmy skills, resolved from the binary |
| Agent | stored choices; env supplies the defaults |

## State: snapshot + journal

```
chats/<key>/
  state.json      full snapshot: {version, seq, entries, docs, tasks}
  journal.jsonl   commits after the snapshot, one JSON object per line
  uploads/        blobs referenced by entries
```

`commit(change)`:

1. take the next `seq`;
2. append `{seq, at, entries, docs, tasks}` as one line and fsync;
3. apply it in memory;
4. publish it.

`snapshot()` runs every 64 commits or 1 MiB: write `state.tmp`, fsync, rename
over `state.json`, fsync the directory, truncate the journal. Replay skips
journal lines with `seq <= snapshot.seq`, so a crash between the rename and the
truncate is harmless. A torn last line is ignored and truncated before the next
append.

One lease (flock on `journal.lock`) per conversation, held for the whole run.
Readers need no lock: every line is fsynced.

Why a snapshot instead of a pure log: entries stay immutable forever, but open
time stays bounded and compaction costs one snapshot instead of rewriting the
history.

## Entries

Context is derived, never stored:

- the visible range starts after the newest `reset`;
- `compaction` replaces everything before it with its summary plus `retained`;
- tool results follow their call, in call order;
- every `tool_intent` without a result gets an `interrupted` result;
- entries without model messages (`tool_intent`, `usage`) never reach the model.

`reset` (today's `/clear`) and `compaction` are appended, not applied by
rewriting `transcript.jsonl`. Images live in `uploads/`: entries keep the path,
and a snapshot clears the inline payload of every image a compaction or reset
superseded.

## Tasks

A durable state machine with a checkpoint after every phase; a restarted process
reruns the last phase from that checkpoint.

```rust
struct Task {
    id: u64,
    kind: &'static str,
    version: u32,
    input: Value,
    phase: &'static str,
    checkpoint: Value,
    state: State,
    owner: Owner,
    attempts: u32,
}

enum State {
    Pending,
    Running,
    Waiting(Vec<u64>),
    Sleeping { until: i64 },
    Completed,
    Failed { reason: String },
    Aborted,
    Blocked { reason: Blocked },
}
```

Two built-ins: `generation` (`prepare → request → tools → settle`) and `tool`
(`run → settle`). A generation commits the assistant entry and one tool task per
call in the same commit, waits for them, and hands the next turn to a new
generation.

A tool intent without a result means the process died mid-call. On reopen, every
such intent becomes one `interrupted` tool result and nothing runs again: a
completed `bash` is never repeated, and the model sees what did not finish.
Re-running safe reads belongs with the task scheduler, once a phase can resume.

Ownership: a generation owns its tools; aborting a conversation aborts the tree.
`attempts` grows when a `Running` task is found at open and resets on every new
checkpoint. At `restart_limit` (5) the task is parked, shows as blocked, and can
be retried by hand.

Sleep is stored state, not a timer: `until`, no invocation, wake at the earliest
deadline. Today's schedule thread polls every 60 s from TOML and a run log; as
tasks, a due prompt survives a deploy mid-wait and a run resumes from its
checkpoint instead of vanishing.

## Submissions

Input is admitted, not executed:

```
queued → placed → done { answer } | unanswered { reason }
```

`{id, request_id, input, status}`. The same `request_id` returns the existing
submission instead of admitting twice. The Telegram `update_id` and the web POST
id become request ids.

Measured today: `Telegram::offset` starts at 0 on every boot and is never
persisted, so a restart re-delivers unconfirmed updates and reruns committed
prompts. Nothing in the code makes a retried input idempotent.

## Documents

Mutable JSON next to the transcript, changed only in commits:

- `agent` — model, effort, max_tokens, cwd, tool selection. Env is the default,
  not the only source.
- `live` — current run and step, so `⏳ leyendo src/web.rs` survives a restart.
- `usage` — input, output, cached; `/status` stops scanning entries.
- `inbox` — queued submissions.

Whole documents per commit; split into ops only when their size demands it.

## Projections

`conversation.jsonl` (UI events) and search hits are caches over entries. Keep
them for the web, but make them rebuildable from the journal.

## Process model

Today `Pool` spawns one worker per busy chat, the worker owns the transcript, and
`inflight` plus a boot scan guess what to resume. With tasks, the pool claims a
ready task, the worker holds the conversation lease, runs phases, commits
checkpoints and releases. A crash leaves `Running`; the next claim reruns the
phase. `inflight`, `recover()` and `local_resume`'s history guessing disappear.

## Ownership

- axe: `durable` (journal, snapshot, entries, tasks, submissions), context
  derivation, replay policies of its tools. `axe::session` stays the
  model-facing transcript type.
- jimmy: transports become subscribers of committed state and the `live` doc;
  the pool becomes a scheduler; `/clear`, `/compact`, `/status` and schedule
  move onto entries and documents.

## Phases

0. `axe::durable`: commit, replay, snapshot. Tests: torn tail, crash between
   rename and truncate, replay twice gives the same state.
1. jimmy writes through it: input committed before generation, assistant and
   tool entries committed as they finish, no whole-file rewrite. Test: SIGKILL
   mid-tool, reopen, no duplicated side effect, same answer.
2. Tool intents and interrupted results; `reset` and `compaction` as entries;
   image payloads pruned at snapshot.
3. Tasks and scheduler in the pool; parked tasks; schedule as sleeping tasks.
4. Inbox and request ids; `agent`, `live`, `usage`; projection rebuild; delete
   `inflight`, `recover()`, transcript rewriting.

## Status

- Phase 0: journal, snapshot, lease (axe #22).
- Phase 1: jimmy writes through the journal, legacy import, reset as an entry
  (jimmy #246).
- Phase 2: tool intents and interrupted results (axe #23); images in `uploads/`,
  payloads pruned at snapshot (jimmy #247).
- Phase 3: turn tasks with a checkpoint per model turn, restart counting and
  parking (axe #25, jimmy #248). Not yet: sleeping schedule tasks, per-tool task
  records, owners and waiting.

Non-goals: Chord typed document ops, Cloudflare Durable Objects, a runtime
plugin registry, code mode. We take the invariants and the shapes, not the API.
