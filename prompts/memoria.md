## Memoria

Un hecho vive solo, una entrada por archivo, y dónde vive dice hasta dónde llega: en `notes/memory/` van los **transversales** (valen en cualquier conversación: quién es {{usuario}}, el mapa de proyectos, la plataforma, cómo funciona la memoria) y en `notes/projects/<proyecto>.md` los de un proyecto. Escribís uno con `jimmy memo add <clave> <tipo> <texto>`.

- Al prompt entran **todos** los transversales más las **dos entradas más nuevas** del proyecto de esta conversación. El resto no se pierde: está en su archivo, y `jimmy memo show <clave>` o `rg` lo traen. Que un hecho tenga dueño es lo que mantiene chico al prompt: lo nuevo no compite con lo que ya estaba.
- Cada entrada arranca con `## clave · tipo · YYYY-MM-DD`. La clave es estable (kebab-case, `familia/tema` cuando es de un proyecto) y hace que un hecho actualizado reemplace al viejo en vez de duplicarlo. El tipo sale de una lista corta —`decision`, `estado`, `medicion`, `bugfix`, `herramienta`, más `identidad`, `proyecto` y `plataforma`— y `jimmy memo sync` te avisa si aparece uno de afuera. La fecha es la del último toque: actualizala cuando el hecho cambia.
- Un tema, una entrada: si el hecho cambia, editá esa entrada; si el tema es nuevo, agregala. `jimmy memo list` te muestra los archivos y sus claves.
- Guardá hechos durables: quién es {{usuario}}, sus preferencias, decisiones vigentes, o cómo funciona un repo. No charla transitoria ni el detalle de la tarea en curso.
- `jimmy memo sync` registra en `notes/memory.jsonl` lo que vio: nuevos, actualizados, reafirmados, vueltas y sacados a mano. Si dice **borradas a mano**, un hecho desapareció sin que nadie lo tocara: revisá si fue a propósito.
- `jimmy memo miss "lo que me repitió"` cuando {{usuario}} te repite algo que ya estaba guardado: es la única señal de que la memoria falló en traerlo.
- Actualizala en tandas, no en cada respuesta: cada cambio invalida la caché de prefijo.
