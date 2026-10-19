# Paridad v1 → v2

Medido contra `web/*.js` (6.910 líneas entre JS y CSS) y probado en Chromium contra un
`jimmy` real con `JIMMY_TRANSPORT=none` y el modelo falso de `tests/e2e/model.py`.

## En paridad (medido)

- Transcripción: mismo `inner_text` que la v1 en la misma conversación
  (`bob / decime hola / ▶ / 1 Comando / · 2 ms / La tool devolvió hola.`).
- Sidebar con proyectos y conversaciones, pestañas con punto de estado y cierre.
- `/api/state`, `/api/history?before=`, `/api/stream` (SSE) y `/api/send` con `request_id`.
- Markdown (pulldown-cmark: tablas, tachado, listas de tareas) con botón de copiar
  código y de copiar el mensaje, y HTML crudo escapado.
- Tarjetas de tool con agrupación en `steps`, `+a -d`, `describeTool` y tiempos.
- Composer: enviar, adjuntar imágenes (`/api/upload`), quitar adjunto, cancelar el turno.
- Tema claro/oscuro con `localStorage` y `prefers-color-scheme`.
- Sidebar: avatares con cara y tono del nombre (`uicore::project`), nombres con la
  primera en mayúscula, tamaño en KB/MB/GB, caret para colapsar (recordado en
  `localStorage`), menú kebab con renombrar, duplicar y quitar proyecto, "Ver más"
  después de 10 conversaciones.
- Pie con CPU y disco (`uicore::machine`).
- Mismo CSS de la v1 (`/theme.css`, `/style.css`, `/avatar.css`, `/vault.css`); la v2
  sólo agrega `#main { height: 100% }`.

## Falta (en orden de valor)

1. Sidebar: chips de Agenda/Vault, "Ver archivos" en el menú, buscador, puntos de
   no leído / atención, renombrar y borrar conversación desde la fila, y el gaze
   de los ojos (puntero).
2. Pestañas: persistencia en `localStorage` + `#hash`, contador en `document.title`,
   "Mirando", botón Cancelar.
3. Mensajes: `jump` (subir/bajar), cargar anteriores, auto-scroll con `follow`,
   Enter envía / Shift+Enter salta línea, pegar imágenes, encoger >1 MB antes de subir.
4. Markdown: resaltado de sintaxis (`highlight`/`highlightDiff` de `markdown.js`),
   `table-wrap`, checkboxes.
5. Fotografías: `shrink` con canvas y nombres `.jpg` equivalentes.
6. `agenda.js`, `vault.js`, `files.js`: pestañas y paneles completos.
7. `machine.js`: CPU/disco al pie. `project.js` y `previews`: tarjetas de preview y stop.
8. `secret.js`: diálogo de secretos del vault.
9. `share_target` (texto/url de la PWA) y pull-to-refresh.
10. Login propio: la v2 manda a `/login` de la v1.
11. Tests del resto: `tool` y `markdown` ya viven en `ui-v2/core` (crate `uicore`, sin
    dioxus) y `make test` corre sus 7 tests en el host. Faltan los de `event` y `api`.

## Divergencias conocidas

- EventSource nativo reconecta solo; la v1 hace backoff propio (sin `onopen` explícito).
- El "escribiendo" se limpia al llegar un delta; la v1 lo apaga a los 4 s.
- Sin atajos ni arrastrar-y-soltar.
