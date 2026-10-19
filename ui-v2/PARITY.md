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
- Pestañas: se guardan en `localStorage` (`jimmy-tabs`), el `#hash` apunta a la activa,
  al recargar se restauran, y el título lleva el contador `(n) Jimmy`.
- Composer: Enter envía, Shift+Enter salta línea, pegar imágenes las sube.
- Fila de conversación: lápiz para renombrar y × para borrar (`/api/rename`,
  `/api/delete-conversation`), con `.slot` como en la v1.
- `#tab-actions`: "Mirando" y "Cancelar" cuando el turno corre.
- `jump`: ver anteriores (`/api/history?before=`) y bajar al final, con auto-scroll.
- Share target: `?text=`/`?url=` de la PWA entran al borrador.
- Al cambiar de pestaña se cierra el SSE de la anterior, como la v1 (batería).
- Vault: chip `Vault`, panel con entornos agrupados por proyecto (general primero),
  crear/borrar entorno, claves con revelar/copiar/editar/borrar, etiqueta `var` para
  variables, copiar como `.env`, tokens con alcance, último uso, revocar y token
  recién creado mostrado una vez, y audit en castellano (`leyó`, `escribió`, …).
  `uicore::vault` con los 8 tests de `vault.test.js`.
- Agenda: chip con badge y punto de error, panel con tareas, horario (`agendaWhen`),
  última corrida, pills de no leído/pausada/bloqueada, desplegar corridas con markdown,
  correr ahora, pausar/reanudar y "Marcar todo leído" (`/api/agenda*`). Se recarga cada
  60 s como la v1. `uicore::agenda` con 6 tests portados de `agenda.test.js`.
- Pestaña Archivos: árbol por proyecto (`/api/tree`), carpetas que se abren al hacer
  clic, imagen, markdown y texto con copiar (`/api/raw`), título "Archivos: Ken" y
  pill del proyecto.
- Mismo CSS de la v1 (`/theme.css`, `/style.css`, `/avatar.css`, `/vault.css`); la v2
  sólo agrega `#main { height: 100% }`.

## Falta (en orden de valor)

1. Sidebar: gaze de los ojos, y la tarjeta de `secret.js` en el composer (pegar un
   secreto o escribir `/secret` para guardarlo en el vault).
2. Markdown: resaltado de sintaxis (`highlight`/`highlightDiff`), `table-wrap`, checkboxes.
3. Fotografías: `shrink` con canvas y nombres `.jpg` equivalentes (>1 MB se suben enteras).
4. Previews: tarjetas `#previews` con stop (`/api/preview/stop` ya existe).
6. Pull-to-refresh y `hashchange` (hoy el hash se escribe y se lee al recargar).
7. El scroll sigue al final siempre: la v1 respeta si el usuario está mirando arriba.
8. Login propio: la v2 manda a `/login` de la v1 cuando `/api/state` responde 401.
9. Tests de `event` y `api`; `uicore` tiene 22.

## Divergencias conocidas

- EventSource nativo reconecta solo; la v1 hace backoff propio (sin `onopen` explícito).
- El "escribiendo" se limpia al llegar un delta; la v1 lo apaga a los 4 s.
- Sin atajos ni arrastrar-y-soltar.
