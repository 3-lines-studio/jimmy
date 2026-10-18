---
name: heimdall
description: The secrets store: inject variables into a command with `heimdall run`.
---

# heimdall

`heimdall run [--project X] [--config Y] -- command` injects that command's variables; the project and the environment come from the repo's `heimdall.yaml`, looked up from the current directory upward, and **whatever the shell already defines wins**. `heimdall help` lists the rest: `setup`, `set`, `ls`, `login`, `token`, `audit`.

- It speaks the `doppler run` dialect (`-p`, `-c`, repeatable `--preserve-env`, the `--`), and the image ships a `doppler → heimdall` symlink, so a repo that uses Doppler runs without touching its Makefile.
- No server and no token: Jimmy opens the vault at `/data/heimdall` and hands the directory down as `HEIMDALL_DIR`, so `heimdall run` reads it in place. Whoever can open that directory is the admin and nothing gives you a 403, so only point a command at the environment you were asked to touch. `login` and `token` only mean something against a server.
