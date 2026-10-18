---
name: agenda
description: Scheduled tasks in state/schedule, one TOML per task, run by the scheduler.
---

# agenda

`state/schedule/` holds your scheduled tasks: **one file per task**, and the file name is the task name. The scheduler runs them by itself, every 60 seconds, in a **clean context**: system prompt plus the task's `prompt`, nothing of the chat or the history. The agenda belongs to the web: the user sees it, runs it by hand and pauses it in the **Agenda** tab, and the answer to every run is kept there. Telegram and Slack are only a backup notice.

When the user asks you to schedule something, write a new file; the tick rereads it on every turn, with no restart. To stop running it, delete the file. To silence it without deleting it, add `paused = true`.

```toml
# state/schedule/record-tests.toml
at = "15:00"
target = "123456789"
prompt = "Tell the user to run the tests before merging."
```

- `prompt` — what it has to do. It does not see the chat: if it needs the thread, tell it to use `recall`.
- `target` — a backup notice: which chat the answer is sent to **as well**. It is the `Chat actual` of the environment block. The agenda lives in the web, so the run is the same without `target`, and if the chat does not exist the task still runs and the history says it did not go out.
- `silent = true` — the task only speaks when it has something to say: it shows no progress indicator and, if the answer comes out empty, sends nothing. For watchdogs that report only failures.
- `paused = true` — it does not run, but stays in the list.
- One way to set the schedule: `when = "YYYY-MM-DDTHH:MM"` (once), `at = "HH:MM"` (every day at that hour) or `every = "30m"` (every so often; units `s`, `m`, `h`, `d`). Local time is UTC plus `JIMMY_TZ_OFFSET` hours.

Every run leaves one line in `state/schedule/<task>.jsonl`: when, how long it took and what it answered. The last twenty are kept, and that history is also the task's state: it is where "when did it last run" comes from. Do not write that file by hand.

The user sees all of this in the **Agenda** tab of the web, where they can also run a task on the spot and pause it.

Rules:

- Do not invent tasks the user did not ask for.
- Before creating one, look at what is in `state/schedule/`: if something similar exists, edit it instead of duplicating it.
- One-off tasks do not delete themselves: they stay in the folder with their result.
