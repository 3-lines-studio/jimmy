"""Cliente mínimo de la web para el e2e: login, crear, mandar, subir y frenar.
Guarda la cookie en un archivo. Lo usa run.sh."""

import base64
import json
import os
import sys
import urllib.error
import urllib.request

BASE = "http://127.0.0.1:%s" % sys.argv[1]
COOKIE_FILE = sys.argv[2]
COMMAND = sys.argv[3]
ARGS = sys.argv[4:]

PNG = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg=="
)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def cookie():
    return open(COOKIE_FILE).read().strip()


def call(path, body):
    request = urllib.request.Request(
        BASE + path,
        data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json", "Cookie": cookie()},
        method="POST",
    )
    return urllib.request.urlopen(request)


if COMMAND == "login":
    request = urllib.request.Request(
        BASE + "/api/login",
        data=json.dumps({"email": ARGS[0]}).encode(),
        headers={"Content-Type": "application/json"},
    )
    link = json.load(urllib.request.urlopen(request))["link"]
    opener = urllib.request.build_opener(NoRedirect)
    try:
        opener.open(link)
        raise SystemExit("esperaba el redirect del link")
    except urllib.error.HTTPError as error:
        if error.code != 303:
            raise
        value = error.headers.get("Set-Cookie").split(";")[0]
    open(COOKIE_FILE, "w").write(value)
    print("LOGIN 303", flush=True)
elif COMMAND == "create":
    print(json.load(call("/api/conversations", {"project": ARGS[0]}))["key"], flush=True)
elif COMMAND == "send":
    call(
        "/api/send",
        {
            "conversation": ARGS[0],
            "text": ARGS[1],
            "images": ARGS[2:],
            "request_id": os.environ.get("REQUEST", ""),
        },
    )
    print("SEND 202", flush=True)
elif COMMAND == "photo":
    request = urllib.request.Request(
        BASE + "/api/upload?conversation=%s&name=foto.png" % ARGS[0],
        data=PNG,
        headers={"Cookie": cookie(), "Content-Type": "image/png"},
        method="POST",
    )
    print(json.load(urllib.request.urlopen(request))["name"], flush=True)
elif COMMAND == "cancel":
    call("/api/cancel", {"conversation": ARGS[0]})
    print("CANCEL 200", flush=True)
else:
    raise SystemExit("comando desconocido: %s" % COMMAND)
