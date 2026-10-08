"""A fake llama-server: /health and /v1/embeddings with a deterministic hashed bag of words."""

from __future__ import annotations

import hashlib
import json
import math
import re
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

DIM = 64


def embed(text: str) -> list[float]:
    vec = [0.0] * DIM
    for token in re.findall(r"[a-z0-9]+", text.lower()):
        vec[int.from_bytes(hashlib.sha256(token.encode()).digest()[:4], "big") % DIM] += 1.0
    norm = math.sqrt(sum(x * x for x in vec)) or 1.0
    return [x / norm for x in vec]


class _Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def _send(self, code: int, body: dict) -> None:
        data = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        if self.path == "/health":
            self._send(200, {"status": "ok"})
        else:
            self._send(404, {"error": "not found"})

    def do_POST(self):
        if self.path != "/v1/embeddings":
            self._send(404, {"error": "not found"})
            return
        body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))))
        inputs = body["input"] if isinstance(body["input"], list) else [body["input"]]
        self.server.inputs.extend(inputs)
        self._send(200, {
            "object": "list", "model": body.get("model", "fake"),
            "data": [{"object": "embedding", "index": i, "embedding": embed(t)} for i, t in enumerate(inputs)],
        })


class FakeEmbedder:
    def __init__(self) -> None:
        self._server = ThreadingHTTPServer(("127.0.0.1", 0), _Handler)
        self._server.inputs = []
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)

    def start(self) -> "FakeEmbedder":
        self._thread.start()
        return self

    @property
    def port(self) -> int:
        return self._server.server_address[1]

    @property
    def url(self) -> str:
        return f"http://127.0.0.1:{self.port}"

    @property
    def inputs(self) -> list[str]:
        return self._server.inputs

    def stop(self) -> None:
        self._server.shutdown()
        self._server.server_close()
        self._thread.join(timeout=5)
