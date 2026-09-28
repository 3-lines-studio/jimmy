---
name: python
description: Python y Node en el contenedor efímero: el venv del volumen, `uv` y node_modules.
---

# python

- El contenedor es **efímero**: lo que instales fuera del volumen se pierde en cada redeploy.
- El venv del volumen es `/data/venv`. La primera vez `uv venv --allow-existing /data/venv` (uv baja su intérprete al volumen y tarda); instalás con `uv pip install --python /data/venv/bin/python <paquete>` y corrés con `/data/venv/bin/python script.py`. `python3` a secas vive en la imagen: no persiste.
- En Node/bun, instalá dentro del workspace y `node_modules` queda en el volumen.
- Librerías: `Pillow` y `matplotlib` para imágenes (o HTML+CSS con `browse --shot`), `openpyxl` para planillas, `python-pptx` para presentaciones.
