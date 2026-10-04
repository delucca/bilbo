# Responses API mock: the first reply of a turn is an exec_command call whose cmd is the first line of the file in argv[3]; the reply to its output is a plain message. Logs every request body.
import json, sys, itertools
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
LOG, CMDFILE = sys.argv[2], sys.argv[3]
n = itertools.count(1)
def sse(events):
    return "".join(f"event: {e['type']}\ndata: {json.dumps(e)}\n\n" for e in events).encode()
class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def log_message(self, *a): pass
    def do_GET(self):
        body = json.dumps({"data": [], "models": []}).encode()
        self.send_response(200); self.send_header("content-type","application/json"); self.send_header("content-length",str(len(body))); self.end_headers(); self.wfile.write(body)
    def do_POST(self):
        raw = self.rfile.read(int(self.headers.get("content-length", 0)))
        i = next(n)
        body = json.loads(raw or b"{}")
        with open(LOG, "a") as f:
            f.write(json.dumps({"n": i, "path": self.path, "body": body}) + "\n")
        has_output = any(it.get("type") == "function_call_output" for it in body.get("input", []))
        rid = f"resp_{i}"
        if has_output:
            item = {"type": "message", "role": "assistant", "id": f"msg_{i}", "content": [{"type": "output_text", "text": "DONE"}]}
        else:
            cmd = open(CMDFILE).readline().strip()
            item = {"type": "function_call", "id": f"fc_{i}", "call_id": f"call_{i}", "name": "exec_command", "arguments": json.dumps({"cmd": cmd, "yield_time_ms": 30000})}
        events = [
            {"type": "response.created", "response": {"id": rid}},
            {"type": "response.output_item.done", "item": item},
            {"type": "response.completed", "response": {"id": rid, "usage": {"input_tokens": 100, "input_tokens_details": {"cached_tokens": 0}, "output_tokens": 10, "output_tokens_details": {"reasoning_tokens": 0}, "total_tokens": 110}}},
        ]
        payload = sse(events)
        self.send_response(200); self.send_header("content-type", "text/event-stream"); self.send_header("content-length", str(len(payload))); self.end_headers(); self.wfile.write(payload)
ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
