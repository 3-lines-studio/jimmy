---
name: heimdall
description: El store de secretos: inyectarle variables a un comando con `heimdall run`.
---

# heimdall

`heimdall run [--project X] [--config Y] -- comando` le inyecta a ese comando las variables del entorno; el proyecto y el entorno salen del `heimdall.yaml` del repo, que se busca del directorio actual hacia arriba, y **lo que ya está definido en el shell no se pisa**. `heimdall help` lista el resto: `setup`, `set`, `ls`, `login`, `token`, `audit`.

- Habla el dialecto de `doppler run` (`-p`, `-c`, `--preserve-env` repetible, el `--`), y la imagen trae un symlink `doppler → heimdall`, así que un repo que usa Doppler corre sin tocar el Makefile.
- El alcance lo pone tu token y no el repo: un `heimdall.yaml` que apunte a `prd` te da 403. El token de jimmy es `*/dev`.
