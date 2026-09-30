"""Registra la imagen del sandbox: el entorno del agente.

El control plane no construye imágenes: esto es un paso de release, como el
build de la imagen del control plane. El agente (el binario, los prompts, las
skills y los CLIs) no va adentro: se publica desde el control plane y se
instala en el sandbox, así una versión nueva del código no obliga a reconstruir
el entorno.

    uv run --with tensorlake python deploy/imagen-sandbox.py [nombre]

El nombre por defecto queda anotado en `TENSORLAKE_IMAGE`.
"""

import sys
from pathlib import Path

import shutil
import tempfile

from tensorlake.image import find_sandbox_image_by_name
from tensorlake.image.sandbox_builder import build_sandbox_image

NOMBRE = "jimmy-entorno"
RAIZ = Path(__file__).resolve().parent.parent


def main() -> int:
    nombre = sys.argv[1] if len(sys.argv) > 1 else NOMBRE
    if find_sandbox_image_by_name(nombre):
        print(f"la imagen {nombre} ya está registrada")
        return 0
    with tempfile.TemporaryDirectory() as contexto:
        shutil.copy(RAIZ / "sandbox.dockerfile", Path(contexto) / "Dockerfile")
        shutil.copy(RAIZ / "mise.toml", Path(contexto) / "mise.toml")
        resultado = build_sandbox_image(
            str(Path(contexto) / "Dockerfile"),
            registered_name=nombre,
            context_dir=contexto,
            cpus=1,
            memory_mb=1024,
            disk_mb=10240,
            builder_disk_mb=10240,
        )
    print(f"imagen {resultado.get('name')} · snapshot {resultado.get('snapshotSizeBytes')} bytes")
    return 0


if __name__ == "__main__":
    sys.exit(main())
