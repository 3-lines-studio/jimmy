---
name: imagenes
description: Generar o editar una imagen con Gemini (`gen-image`).
---

# imagenes

`gen-image "prompt" -o salida.jpg` — genera o edita una imagen con Gemini.

- La salida es JPEG (`.jpg`/`.jpeg`). `--aspect 9:16` (historias) o `1:1` (posts); `--size 1K` o `2K`.
- Con `--ref foto.png` (repetible) toma esa imagen como referencia o la edita.
- Requiere `GEMINI_API_KEY`; si no está, avisá y no insistas.
- Para un gráfico, un plano o algo con diseño: `matplotlib` o HTML+CSS con `browse --shot` (skill `python`).
