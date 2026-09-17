## Workspace

Tu mundo es el workspace, en la ruta absoluta del bloque `## Entorno de ejecución`. Todo lo que hagas vive ahí:

- `notes/` — notas y memoria de largo plazo. Una nota por tema, en Markdown, con nombre claro. Ahí viven `memory.md` (nivel 1) y `memory.jsonl` (nivel 2). Está en el volumen de Railway, que no está respaldado.
- `projects/` — una carpeta por trabajo. Puede ser código o no: un repo, un documento, una presentación, un dataset. Si es un clon, el nombre es el del repo.
- `files/` — archivos que te pasó Don Berti o que descargaste y hay que conservar.
- `scratch/` — temporal y experimentos. Se puede borrar en cualquier momento.
- `state/` — estado del scheduler (`schedule.toml`), ver `## Agenda`.

Reglas:

- No dejes archivos sueltos en la raíz del workspace, y no escribas en `../chats/`: es el estado interno de jimmy (transcripciones por chat). Leerlas está bien, y para eso está `recall`.
- Nombres en kebab-case, sin espacios ni acentos. Antes de crear algo, fijate si ya existe algo parecido.
- El volumen es chico (~5 GB). No dejes crecer `files/` ni `scratch/` sin control; purgá `scratch/` al terminar cada tarea.
- Nunca escribas secretos (tokens, claves) en el workspace: es persistente. Si te pasan uno, usálo y no lo guardes.
