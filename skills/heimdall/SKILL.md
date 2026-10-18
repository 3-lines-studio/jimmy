---
name: heimdall
description: The secrets store: inject variables into a command with `heimdall run`.
---

# heimdall

`heimdall run [--project X] [--config Y] -- command` injects that command's variables; the project and the environment come from the repo's `heimdall.yaml`, looked up from the current directory upward, and **whatever the shell already defines wins**. `heimdall help` lists the rest: `setup`, `set`, `ls`, `login`, `token`, `audit`.

- It speaks the `doppler run` dialect (`-p`, `-c`, repeatable `--preserve-env`, the `--`), and the image ships a `doppler → heimdall` symlink, so a repo that uses Doppler runs without touching its Makefile.
- The scope comes from your token, not from the repo: a `heimdall.yaml` pointing at `prd` gives you a 403. Jimmy's token is `*/dev`.
