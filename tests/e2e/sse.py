"""Se loguea, mira el estado y se queda escuchando el stream de una
conversación, imprimiendo cada evento. Lo usa run.sh."""

import json, socket, sys, time, urllib.error, urllib.request

BASE = "http://127.0.0.1:%s" % sys.argv[1]
EMAIL = sys.argv[2]
CONVERSATION = sys.argv[3]
SECONDS = float(sys.argv[4])


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


body = json.dumps({"email": EMAIL}).encode()
request = urllib.request.Request(
    BASE + "/api/login", data=body, headers={"Content-Type": "application/json"}
)
link = json.load(urllib.request.urlopen(request))["link"]
print("LINK " + link.split("token=")[0] + "token=...", flush=True)

opener = urllib.request.build_opener(NoRedirect)
try:
    opener.open(link)
    raise SystemExit("esperaba el redirect del link")
except urllib.error.HTTPError as e:
    assert e.code == 303, e.code
    cookie = e.headers.get("Set-Cookie").split(";")[0]
print("LOGIN 303", flush=True)

request = urllib.request.Request(BASE + "/api/state", headers={"Cookie": cookie})
print("STATE " + json.dumps(json.load(urllib.request.urlopen(request))), flush=True)

stream = socket.create_connection(("127.0.0.1", int(sys.argv[1])))
stream.sendall(
    (
        "GET /api/stream?conversation=%s HTTP/1.1\r\nHost: jimmy\r\nCookie: %s\r\n\r\n"
        % (CONVERSATION, cookie)
    ).encode()
)
stream.settimeout(2)
buffer = b""
deadline = time.time() + SECONDS
while time.time() < deadline:
    try:
        chunk = stream.recv(4096)
    except socket.timeout:
        continue
    if not chunk:
        break
    buffer += chunk
    while b"\n\n" in buffer:
        block, buffer = buffer.split(b"\n\n", 1)
        text = block.decode("utf-8", "replace").strip().replace("\n", " | ")
        for line in text.split(" | "):
            if line.startswith("data: ") or line.startswith(": "):
                print("SSE " + line[:200], flush=True)
print("SSE-END", flush=True)
