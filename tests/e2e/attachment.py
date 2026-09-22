"""Sube una imagen por la web, la manda sin texto y la vuelve a bajar. Después
manda una con `jimmy send`, como haría el asistente, y espera a que se vea en el
log. Lo usa run.sh."""

import base64
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request

BASE = "http://127.0.0.1:%s" % sys.argv[1]
EMAIL = sys.argv[2]
PROJECT = sys.argv[3]
BIN = sys.argv[4]
ROOT = sys.argv[5]
PNG_PATH = sys.argv[6]

# Un PNG de 1x1 rojo.
PNG = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg=="
)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def call(path, body=None, cookie=None, method=None, data=None, content_type="application/json"):
    headers = {}
    if cookie:
        headers["Cookie"] = cookie
    if body is not None:
        data = json.dumps(body).encode()
    if data is not None:
        headers["Content-Type"] = content_type
    request = urllib.request.Request(BASE + path, data=data, headers=headers, method=method)
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

created = call("/api/conversations", {"project": PROJECT}, cookie)
assert created.status == 200, created.status
key = json.load(created)["key"]
print("KEY %s" % key, flush=True)

uploaded = call(
    "/api/upload?conversation=%s&name=circulo.png" % key,
    cookie=cookie,
    method="POST",
    data=PNG,
    content_type="image/png",
)
assert uploaded.status == 200, uploaded.status
name = json.load(uploaded)["name"]
print("NAME %s" % name, flush=True)

served = call("/api/file?conversation=%s&name=%s" % (key, name), cookie=cookie)
assert served.status == 200, served.status
bytes_back = served.read()
assert bytes_back == PNG, "el adjunto volvió cambiado"
print("FILE %s %s %s" % (served.headers.get("Content-Type"), len(bytes_back), name), flush=True)

sent = call("/api/send", {"conversation": key, "text": "", "images": [name]}, cookie)
assert sent.status == 202, sent.status
print("SEND %s" % sent.status, flush=True)

with open(PNG_PATH, "wb") as file:
    file.write(PNG)

env = dict(os.environ, JIMMY_ROOT=ROOT, JIMMY_WORKSPACE=os.path.join(ROOT, "workspace"))
command = [BIN, "send", PNG_PATH, "--target", key, "--caption", "el círculo"]
assistant = subprocess.run(command, env=env, capture_output=True, text=True)
assert assistant.returncode == 0, assistant.stderr
kept = assistant.stdout.strip()
print("SENT-CLI %s" % kept, flush=True)

log = os.path.join(ROOT, "chats", key, "conversation.jsonl")
call("/api/send", {"conversation": key, "text": "¿y eso?"}, cookie)
deadline = time.time() + 30
written = ""
while time.time() < deadline:
    written = open(log).read() if os.path.exists(log) else ""
    if '"event":"image"' in written:
        break
    time.sleep(0.2)
assert '"event":"image"' in written, written
assert '"caption":"el círculo"' in written, written
print("LOG image %s" % kept, flush=True)

served = call("/api/file?conversation=%s&name=%s" % (key, kept), cookie=cookie)
assert served.status == 200, served.status
bytes_back = served.read()
assert bytes_back == PNG, "la imagen del asistente volvió cambiada"
print("FILE2 %s %s" % (served.headers.get("Content-Type"), len(bytes_back)), flush=True)
