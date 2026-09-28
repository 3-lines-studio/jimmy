---
name: browse
description: Navegar con un Chromium de verdad: páginas con JavaScript, cookies, capturas, videos y secuencias de pasos.
---

# browse

Un Chromium real, para lo que `fetch` no alcanza: JavaScript, cookies o una captura. La tool `browse` y el binario `browse` son lo mismo: pasale `url` para una página o `steps` para una secuencia.

- `goto <url>` — abre la página y devuelve el texto con los controles y un selector usable. `--session NOMBRE` reusa las cookies de esa sesión y `--shot` guarda un PNG, que mirás con `read`.
- `run <archivo|->` — corre una secuencia de pasos en un solo navegador, que es lo que hace falta cuando una acción depende de la anterior. Los pasos son una lista JSON: `[{"action":"goto","url":"..."},{"action":"fill","selector":"#user","value":"pepe"},{"action":"click","selector":"#go"},{"action":"read"}]`. Acciones: `goto`, `click`, `fill`, `press`, `select`, `wait`, `read`, `html`, `elements`, `shot`, `url`, `highlight` (marca el elemento), `scroll`.
- `files --session NOMBRE` lista las capturas y videos de la sesión, y `sessions` lista las sesiones. Los artefactos quedan en `/data/files/browse/<sesión>/` y se borran solos a los 7 días.
- Grabás y capturás a `--size WxH` (default 1920x1080). Logueate una vez y esa sesión queda logueada: las cookies viven en `/data/state/browser/<sesión>/`.
- Para una demo: todos los pasos en un solo `run`, `--record --slow 600`, y un `highlight` antes de cada click. El video sale a 25 fps y arranca con alrededor de un segundo en blanco.
