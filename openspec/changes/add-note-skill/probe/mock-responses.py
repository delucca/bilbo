# Minimal OpenAI Responses API mock: streams one assistant message per request, logs bodies.
import json, sys, itertools
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
LOG = sys.argv[2]
n = itertools.count(1)
class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def log_message(self, *a): pass
    def do_GET(self):
        body = json.dumps({"data": [], "models": []}).encode()
        self.send_response(200); self.send_header("content-type","application/json"); self.send_header("content-length",str(len(body))); self.end_headers(); self.wfile.write(body)
    def do_POST(self):
        raw = self.rfile.read(int(self.headers.get("content-length", 0)))
        i = next(n)
        with open(LOG, "a") as f:
            f.write(json.dumps({"n": i, "path": self.path, "body": json.loads(raw or b"{}")}) + "\n")
        text = f"MOCK REPLY {i}"
        rid = f"resp_{i}"
        events = [
            {"type": "response.created", "response": {"id": rid}},
            {"type": "response.output_item.done", "item": {"type": "message", "role": "assistant", "id": f"msg_{i}", "content": [{"type": "output_text", "text": text}]}},
            {"type": "response.completed", "response": {"id": rid, "usage": {"input_tokens": 50000, "input_tokens_details": {"cached_tokens": 0}, "output_tokens": 10, "output_tokens_details": {"reasoning_tokens": 0}, "total_tokens": 50010}}},
        ]
        payload = "".join(f"event: {e['type']}\ndata: {json.dumps(e)}\n\n" for e in events).encode()
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("content-length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)
ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
