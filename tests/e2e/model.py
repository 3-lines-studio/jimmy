"""Un modelo de mentira, compatible con OpenAI.

Primer pedido: pide una tool. Después: contesta. La demora es para que el turno
dure lo suficiente como para mirarlo en vivo por el SSE.
"""

import http.server, json, sys, time

PORT = int(sys.argv[1])
TOOL = (
    'data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1",'
    '"function":{"name":"bash","arguments":"{\\"command\\":\\"echo hola\\"}"}}]}}]}\n\n'
    'data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}\n\n'
    "data: [DONE]\n\n"
)
ANSWER = (
    'data: {"choices":[{"delta":{"content":"La tool devolvió "}}]}\n\n'
    'data: {"choices":[{"delta":{"content":"hola."}}]}\n\n'
    'data: {"choices":[{"delta":{},"finish_reason":"stop"}]}\n\n'
    "data: [DONE]\n\n"
)
rounds = 0


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_POST(self):
        global rounds
        length = int(self.headers.get("Content-Length", 0))
        self.rfile.read(length)
        rounds += 1
        time.sleep(2)
        payload = (TOOL if rounds == 1 else ANSWER).encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Content-Length", str(len(payload)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, *args):
        pass


server = http.server.ThreadingHTTPServer(("127.0.0.1", PORT), Handler)
print("PORT %d" % server.server_address[1], flush=True)
server.serve_forever()
