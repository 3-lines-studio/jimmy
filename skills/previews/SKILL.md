---
name: previews
description: Run a project and serve it at /preview/<name>/, with its recipes and its log.
---

# previews

`jimmy preview start <name> [--cmd 'command'] [--cwd dir]` runs a project and serves it at `/preview/<name>/` on this web, under your own session.

- **Recipes** in `state/previews.toml` keep each project's start command, so the usual call is plain `jimmy preview start bifrost`: `jimmy preview recipes` lists them, and a `--cmd` given on the spot wins over the recipe.
- The command runs with a **clean environment** (`PATH`, `HOME`, `LANG`, `TZ`, plus `PORT` and `PREVIEW_NAME`), without jimmy's secrets: whatever the project needs is declared in its recipe, in the open. Its output lands in `state/previews/<name>.log`.
- **The project runs bare**, the way it would on your machine: the proxy strips the prefix before handing it a request and puts the prefix back on everything that comes out pointing at the root (HTML, CSS, JavaScript and redirects). There is no base path to configure. The limits: a JSON body carrying paths, or a path built at runtime, is not rewritten. Start it with `--strictPort` so it does not drift off the port jimmy gave it.
- **A preview goes down by itself after 30 minutes with no visits** (every request through the proxy resets that clock) and the next deploy takes it too: it is for looking at work in progress, not for hosting. Anyone who walks in after that gets a 404 with the last lines of its log.
- `jimmy preview list` shows what is up (and how long since anyone visited) and `jimmy preview stop <name>` takes them down.
