---
name: python
description: Python and Node in the ephemeral container: the volume's venv, `uv` and node_modules.
---

# python

- The container is **ephemeral**: anything you install outside the volume is lost on every redeploy.
- The volume's venv is `/data/venv`. The first time, `uv venv --allow-existing /data/venv` (uv downloads its interpreter to the volume and takes a while); you install with `uv pip install --python /data/venv/bin/python <package>` and run with `/data/venv/bin/python script.py`. Plain `python3` lives in the image: it does not persist.
- In Node/bun, install inside the workspace so `node_modules` stays on the volume.
- Libraries: `Pillow` and `matplotlib` for images (or HTML+CSS with `browse --shot`), `openpyxl` for spreadsheets, `python-pptx` for decks.
