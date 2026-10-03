---
name: imagenes
description: Generar o editar una imagen con Gemini (`gen-image`).
---

# imagenes

`gen-image "prompt" -o salida.jpg` — genera o edita una imagen con Gemini.

- La salida es siempre JPEG (`.jpg`/`.jpeg`): es el único formato que acepta la API.
- `--size` fija el área total, no el lado mayor: `1K` son ~1 MP, `2K` ~4 MP y `4K` ~16 MP. En `9:16`, `1K` da 768x1376 y `2K` 1536x2752.
- `2K` es el default y alcanza para Instagram: cubre los 1080 de ancho en cualquier aspect y pesa ~3 MB. Con `1K` no llegás (768 de ancho en vertical) y `4K` sobra.
- Al editar una foto, pasá el `--aspect` de la entrada: una foto 4:3 va con `--aspect 4:3`, porque el default `1:1` la recorta. Para piezas nuevas, `9:16` (historias) o `1:1` (posts).
- La API normaliza la referencia: una foto de miles de píxeles no da más detalle que una de ~1 MP, solo suma segundos de subida.
- Con `--ref foto.png` (repetible) toma esa imagen como referencia o la edita.
- Requiere `GEMINI_API_KEY`; si no está, avisá y no insistas.
- Para un gráfico, un plano o algo con diseño: `matplotlib` o HTML+CSS con `browse --shot` (skill `python`).
