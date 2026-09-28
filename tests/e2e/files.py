"""Mira el árbol de un proyecto por la web, baja un archivo y comprueba que no
se puede salir de la carpeta del proyecto. Lo usa run.sh."""

import base64
import json
import os
import sys
import urllib.error
import urllib.request

BASE = "http://127.0.0.1:%s" % sys.argv[1]
EMAIL = sys.argv[2]
PROJECT = sys.argv[3]
ROOT = sys.argv[4]

PNG = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg=="
)
NOTA = "hola desde el árbol\n"


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def call(path, cookie=None):
    headers = {"Cookie": cookie} if cookie else {}
    request = urllib.request.Request(BASE + path, headers=headers)
    try:
        return urllib.request.urlopen(request)
    except urllib.error.HTTPError as error:
        return error


body = json.dumps({"email": EMAIL}).encode()
request = urllib.request.Request(
    BASE + "/api/login", data=body, headers={"Content-Type": "application/json"}
)
link = json.load(urllib.request.urlopen(request))["link"]
opener = urllib.request.build_opener(NoRedirect)
try:
    opener.open(link)
    raise SystemExit("esperaba el redirect del link")
except urllib.error.HTTPError as error:
    assert error.code == 303, error.code
    cookie = error.headers.get("Set-Cookie").split(";")[0]

project = os.path.join(ROOT, "workspace", "projects", PROJECT)
os.makedirs(os.path.join(project, "src"), exist_ok=True)
with open(os.path.join(project, "nota.txt"), "w") as file:
    file.write(NOTA)
with open(os.path.join(project, "src/nota.rs"), "w") as file:
    file.write("fn main() {}\n")
with open(os.path.join(project, ".env"), "w") as file:
    file.write("SECRETO=1\n")
with open(os.path.join(project, "logo.png"), "wb") as file:
    file.write(PNG)
os.makedirs(os.path.join(project, "target"), exist_ok=True)
with open(os.path.join(project, "target/gordo"), "w") as file:
    file.write("no se lista\n")

tree = call("/api/tree?project=%s" % PROJECT, cookie)
assert tree.status == 200, tree.status
entries = json.load(tree)["entries"]
listed = ", ".join("%s:%s" % (entry["name"], entry["kind"]) for entry in entries)
assert listed == "src:dir, logo.png:image, nota.txt:text", listed
print("TREE %s" % listed, flush=True)

deeper = call("/api/tree?project=%s&path=src" % PROJECT, cookie)
assert deeper.status == 200, deeper.status
assert json.load(deeper)["entries"][0]["name"] == "nota.rs"
print("TREE2 src/nota.rs", flush=True)

raw = call("/api/raw?project=%s&path=nota.txt" % PROJECT, cookie)
assert raw.status == 200, raw.status
assert raw.headers.get("Content-Type").startswith("text/plain"), raw.headers
assert raw.read() == NOTA.encode(), "el archivo volvió cambiado"
print("TEXT %s" % raw.headers.get("Content-Type"), flush=True)

image = call("/api/raw?project=%s&path=logo.png" % PROJECT, cookie)
assert image.status == 200, image.status
assert image.headers.get("Content-Type") == "image/png", image.headers
assert image.read() == PNG, "la imagen volvió cambiada"
print("IMAGE image/png", flush=True)

escapes = [
    "/api/tree?project=%s&path=.." % PROJECT,
    "/api/raw?project=%s&path=../../../prompts/jimmy.md" % PROJECT,
    "/api/raw?project=%s&path=.env" % PROJECT,
    "/api/tree?project=..",
    "/api/tree?project=no-existe",
    "/api/raw?project=%s&path=no-esta.txt" % PROJECT,
]
for path in escapes:
    answer = call(path, cookie)
    assert answer.status in (400, 404), "%s dio %s" % (path, answer.status)
print("ESC %s rutas rechazadas" % len(escapes), flush=True)

assert call("/api/tree?project=%s" % PROJECT, None).status == 401
print("ANON 401", flush=True)
