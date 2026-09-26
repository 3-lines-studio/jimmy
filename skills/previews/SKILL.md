---
name: previews
description: Levantar un proyecto y servirlo en /preview/<nombre>/, con sus recetas y su log.
---

# previews

`jimmy preview start <nombre> [--cmd 'comando'] [--cwd dir]` levanta un proyecto y lo sirve en `/preview/<nombre>/` de esta web, con tu misma sesión.

- Las **recetas** de `state/previews.toml` guardan el comando de arranque de cada proyecto, así que lo normal es `jimmy preview start bifrost` a secas: `jimmy preview recipes` lista las que hay y un `--cmd` dado en el momento gana sobre la receta.
- El comando corre con un **entorno limpio** (`PATH`, `HOME`, `LANG`, `TZ`, más `PORT` y `PREVIEW_NAME`), sin los secretos de jimmy: lo que el proyecto necesite lo declara su receta, a la vista. Su salida queda en `state/previews/<nombre>.log`.
- **El proyecto se arranca pelado**, como en tu máquina: el proxy le saca el prefijo antes de pasarle el pedido y le devuelve puesto el prefijo a todo lo que sale apuntando a la raíz (HTML, CSS, JavaScript y los redirects). No hay que configurarle ningún base. Los límites: un JSON que devuelve paths, o un path armado en runtime, no se reescriben. Arrancalo con `--strictPort` para que no se corra al puerto que jimmy le dio.
- **Un preview se baja solo a los 30 minutos sin visitas** (cada pedido al proxy corre ese reloj) y también se lo lleva el próximo deploy: es para mirar trabajo en curso, no para hostear. Si alguien entra y ya no está, ve un 404 con las últimas líneas de su log.
- `jimmy preview list` muestra los que están arriba (y hace cuánto que nadie los visita) y `jimmy preview stop <nombre>` los baja.
