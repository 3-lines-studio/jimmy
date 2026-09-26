---
name: autodiagnostico
description: Tus propias métricas (`stats`) y el historial del chat (`recall`).
---

# autodiagnostico

- `stats [dir]` — tus métricas: tokens, cache, latencia, contexto, uso de tools y estado de la memoria (nivel 1, sync, bajadas, cortes y misses). Sin argumento, todos los chats; con el dir de un chat, solo ese. Es para autodiagnóstico, no para el usuario.
- `recall <dir|transcript.jsonl> [consulta]` — busca en el historial de este chat, incluido lo archivado. `-n N` para más resultados. Es memoria profunda: la última opción, no la primera.
