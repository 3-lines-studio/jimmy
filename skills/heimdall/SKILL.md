---
name: heimdall
description: The secrets store: inject variables into a command with `heimdall run`.
---

# heimdall

`heimdall run [--project X] [--config Y] -- command` injects that command's variables; the project and the environment come from the repo's `heimdall.yaml`, looked up from the current directory upward, and **whatever the shell already defines wins**. `heimdall help` lists the rest: `setup`, `set`, `ls`, `login`, `token`, `audit`.

- It speaks the `doppler run` dialect (`-p`, `-c`, repeatable `--preserve-env`, the `--`), and the image ships a `doppler → heimdall` symlink, so a repo that uses Doppler runs without touching its Makefile.
- The variables are already there: before every turn the worker puts `general/<env>` in its own environment and on top of that the project of the nearest `heimdall.yaml` or, without one, the folder under `projects/`, so `$STRIPE_KEY` works like any other variable. A name the shell already defines wins. `HEIMDALL_PROJECT` and `HEIMDALL_ENV` (default `dev`) override both.
- `heimdall run` is still the explicit way when you want another project or another environment, and it reads the same vault in place: no server, no token, `HEIMDALL_DIR` points at `/data/heimdall`. Whoever can open that directory is the admin and nothing gives you a 403, so only point a command at the environment you were asked to touch. `login` and `token` only mean something against a server.
- Values are cut out of tool output before it reaches the journal or the model, so a stray `printenv` is not the end of the world. A transformed value (base64, reversed, split across lines) escapes that net: don't do it.
