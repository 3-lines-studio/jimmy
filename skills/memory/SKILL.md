---
name: memory
description: Long-lived facts in notes/memory and notes/projects, written with `jimmy memo`.
---

# memory

A fact lives alone, one entry per file, and where it lives says how far it reaches: `notes/memory/` holds the **cross-cutting** ones (true in any conversation: who the user is, the project map, the platform, how memory works) and `notes/projects/<project>.md` the ones about a single project. You write one with `jimmy memo add <key> <type> <text>`.

- The prompt gets **every** cross-cutting entry plus the **two newest** of this conversation's project. The rest is not lost: it stays in its file, and `jimmy memo show <key>` or `rg` brings it back. Every fact having an owner is what keeps the prompt small: the new does not compete with what was already there.
- Each entry opens with `## key · type · YYYY-MM-DD`. The key is stable (kebab-case, `family/topic` for a project) and makes an updated fact replace the old one instead of duplicating it. The type comes from a short list — `decision`, `estado`, `medicion`, `bugfix`, `herramienta`, plus `identidad`, `proyecto` and `plataforma` — and `jimmy memo sync` warns you when one from outside shows up. The date is the last touch: bump it when the fact changes.
- One topic, one entry: if the fact changes, edit that entry; if the topic is new, add one. `jimmy memo list` shows the files and their keys.
- Keep durable facts: who the user is, their preferences, standing decisions, or how a repo works. Not small talk, not the detail of the task at hand.
- `jimmy memo sync` records in `notes/memory.jsonl` what it saw: new, updated, reaffirmed, turned and hand-deleted. If it says **borradas a mano**, a fact vanished without anyone touching it: check whether that was on purpose.
- `jimmy memo miss "what was repeated"` when the user repeats something that was already stored: it is the only signal that memory failed to bring it.
- Update it in batches, not on every answer: each change invalidates the prefix cache.
