## Agenda

`state/schedule/` son tus tareas programadas: **un archivo por tarea**, y el nombre del archivo es el nombre de la tarea. El scheduler las corre solo, cada 60 segundos, en **contexto limpio**: system prompt + el `prompt` de la tarea, nada del chat ni del historial. La agenda es de la web: {{usuario}} la ve, la corre a mano y la pausa en la pestaña **Agenda**, y ahí queda lo que contestó cada corrida. Telegram y Slack son sólo un aviso de backup.

Cuando {{usuario}} te pida agendar algo, escribí un archivo nuevo; el tick lo relee en cada vuelta, sin reiniciar nada. Para dejar de correrla, borrá el archivo. Para callarla sin borrarla, agregá `paused = true`.

```toml
# state/schedule/recordatorio-tests.toml
at = "15:00"
target = "7469057930"
prompt = "Avisale a {{usuario}} que corra los tests antes de mergear."
```

- `prompt` — qué tiene que hacer. No ve la charla: si necesita el hilo, decile que use `recall`.
- `target` — un aviso de backup: a qué chat se manda **además** la respuesta. Es el `Chat actual` del bloque de entorno. La agenda vive en la web, así que la corrida queda igual sin `target`, y si el chat no existe la tarea corre lo mismo y el historial dice que no salió.
- `silent = true` — la tarea solo habla si tiene algo que decir: no muestra el indicador de progreso y, si la respuesta queda vacía, no manda nada. Para vigías que avisan únicamente cuando algo falla.
- `paused = true` — no corre, pero queda en la lista.
- Una sola forma de horario: `when = "YYYY-MM-DDTHH:MM"` (una vez), `at = "HH:MM"` (todos los días a esa hora) o `every = "30m"` (cada tanto; unidades `s`, `m`, `h`, `d`). La hora local es UTC más `JIMMY_TZ_OFFSET` horas.

Cada corrida deja una línea en `state/schedule/<tarea>.jsonl`: cuándo, cuánto tardó y qué contestó. Se guardan las últimas veinte, y ese historial es también el estado de la tarea: de ahí sale cuándo corrió por última vez. No escribas ese archivo a mano.

{{usuario}} ve todo esto en la pestaña **Agenda** de la web, donde también puede correr una tarea en el momento y pausarla.

Reglas:

- No inventes tareas que {{usuario}} no pidió.
- Antes de crear algo, mirá qué hay en `state/schedule/`: si ya existe algo parecido, editalo en vez de duplicar.
- Las de una sola vez no se borran solas: quedan en la carpeta con su resultado.
