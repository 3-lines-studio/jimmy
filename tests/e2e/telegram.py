"""Un Telegram de mentira.

Entrega por getUpdates lo que aparezca en el inbox (una línea JSON por
mensaje) y anota en stdout lo que jimmy manda con sendMessage y
editMessageText. Lo levanta run.sh; no sirve para nada más.
"""

import base64, http.server, json, os, sys, time

PORT = int(sys.argv[1])
INBOX = sys.argv[2]
sent = 0

def await_update():
    global sent
    deadline = time.time() + 20
    while True:
        lines = []
        if os.path.exists(INBOX):
            lines = [line for line in open(INBOX).read().splitlines() if line.strip()]
        if len(lines) > sent:
            sent += 1
            return [json.loads(lines[sent - 1])]
        if time.time() > deadline:
            return []
        time.sleep(0.5)

class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_POST(self):
        method = self.path.rsplit("/", 1)[-1]
        length = int(self.headers.get("Content-Length", 0))
        body = json.loads(self.rfile.read(length) or b"{}")
        if method == "getUpdates":
            result = await_update()
        elif method == "sendMessage":
            print("SEND " + json.dumps(body, ensure_ascii=False), flush=True)
            result = {"message_id": 1}
        elif method == "editMessageText":
            print("EDIT " + json.dumps(body, ensure_ascii=False), flush=True)
            result = {"message_id": 1}
        elif method == "getFile":
            result = {"file_path": "foto.png"}
        else:
            result = True
        payload = json.dumps({"ok": True, "result": result}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def do_GET(self):
        if "/file/" not in self.path:
            self.send_error(404)
            return
        data = base64.b64decode(
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAFgwJ/lK3r9wAAAABJRU5ErkJggg=="
        )
        self.send_response(200)
        self.send_header("Content-Type", "image/png")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        pass

server = http.server.ThreadingHTTPServer(("127.0.0.1", PORT), Handler)
print("PORT %d" % server.server_address[1], flush=True)
server.serve_forever()
