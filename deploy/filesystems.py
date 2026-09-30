"""El filesystem de una org, que es el volumen donde vive su trabajo.

Lo crea el SDK de Tensorlake, que es el único que sabe hablar con ese servicio;
el control plane lo llama por `uv run --with tensorlake python`. Idempotente: si
el filesystem ya está, crear no hace nada.

    uv run --with tensorlake python deploy/filesystems.py crear jimmy-<org>
"""

import sys

from tensorlake.filesystem import FilesystemClient


def nombres(cliente):
    return {info.name for info in cliente.list()}


def main(argv):
    if len(argv) != 2 or argv[0] not in ("crear", "borrar", "listar"):
        print(__doc__)
        return 2
    cliente = FilesystemClient()
    if argv[0] == "listar":
        print("\n".join(sorted(nombres(cliente))))
        return 0
    nombre = argv[1]
    esta = nombre in nombres(cliente)
    if argv[0] == "crear":
        if esta:
            print(f"{nombre} ya está")
            return 0
        cliente.create(nombre)
        print(f"{nombre} creado")
        return 0
    if not esta:
        print(f"{nombre} no está")
        return 0
    cliente.delete(nombre)
    print(f"{nombre} borrado")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
