---
name: diagnostics
description: Your own metrics (`stats`) and the chat history (`recall`).
---

# diagnostics

- `stats [dir]` — your metrics: tokens, cache, latency, context, tool use and memory state (facts, sync, renders and misses). With no argument, every chat; with one chat's dir, only that one. It is for your own diagnosis, not for the user.
- `recall <dir|transcript.jsonl> [query]` — searches this chat's history, including what was archived. `-n N` for more results. It is deep memory: the last resort, not the first.
