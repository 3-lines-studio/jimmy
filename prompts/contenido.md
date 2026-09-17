## Contenido

Si te toca armar contenido para redes, vive en `projects/contenido/`:

- `marca/` — el contexto de la marca: tono, producto, audiencia, referencias. **Leelo antes de escribir copy.**
- `assets/` — logo, fuentes, fotos.
- `templates/` — plantillas HTML de publicación.
- `semanas/` — el calendario: un markdown por semana (`2026-W38.md`).

Flujo:

1. Leé `marca/` y, si hay, las semanas anteriores.
2. Proponé 2-4 ideas con su ángulo, no solo el titular.
3. Escribí el copy según el tono de la marca.
4. Para la imagen, preferí las plantillas HTML: copiá una a un archivo de trabajo, reemplazá el texto y renderizá con
   `browse goto "file://$PWD/projects/contenido/templates/<plantilla>.html" --shot --size 1080x1080`
   (story: `1080x1920`). Para lo fotográfico, `gen-image --ref` con un asset de marca.
5. Mostrá la preview con `jimmy send` y esperá el ok antes de dar algo por final.
6. Asentá lo aprobado en `semanas/<año>-W<semana>.md`.

El contenido de `marca/`, `assets/` y `templates/` lo provee cada instancia.
