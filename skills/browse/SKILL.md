---
name: browse
description: Drive a real Chromium: pages with JavaScript, cookies, screenshots, videos and step sequences.
---

# browse

A real Chromium, for what `fetch` cannot reach: JavaScript, cookies or a screenshot. The `browse` tool and the `browse` binary are the same thing: pass `url` for a page or `steps` for a sequence.

- `goto <url>` — opens the page and returns the text with its controls and a usable selector. `--session NAME` reuses that session's cookies and `--shot` saves a PNG you read with `read`.
- `run <file|->` — runs a step sequence in a single browser, which is what you need when one action depends on the previous one. Steps are a JSON list: `[{"action":"goto","url":"..."},{"action":"fill","selector":"#user","value":"pepe"},{"action":"click","selector":"#go"},{"action":"read"}]`. Actions: `goto`, `click`, `fill`, `press`, `select`, `wait`, `read`, `html`, `elements`, `shot`, `url`, `highlight` (marks the element), `scroll`.
- `files --session NAME` lists that session's screenshots and videos, and `sessions` lists the sessions. Artifacts land in `/data/files/browse/<session>/` and are deleted after 7 days.
- You record and capture at `--size WxH` (default 1920x1080). Log in once and the session stays logged in: cookies live in `/data/state/browser/<session>/`.
- For a demo: every step in one `run`, `--record --slow 600`, and a `highlight` before each click. The video comes out at 25 fps and opens with about a second of blank.
