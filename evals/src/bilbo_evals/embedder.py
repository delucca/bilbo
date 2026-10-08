"""The pinned embedder: llama-server launch, a recording proxy, the vector cache and the index check."""

from __future__ import annotations

import fcntl
import json
import os
import re
import shutil
import socket
import struct
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import numpy as np

from bilbo_evals import passages, sandbox
from bilbo_evals.common import CACHE_DIR, Refused, out, sha256_bytes, sha256_file, tool_version

MODEL = "qwen3-embedding-0.6b"
GGUF_SHA256 = "06507c7b42688469c4e7298b0a1e16deff06caf291cf0a5b278c308249c3e439"
GGUF_FILE = "Qwen3-Embedding-0.6B-Q8_0.gguf"
QUERY_PREFIX = "Instruct: Given a question, retrieve notes that answer it\nQuery: "
BATCH = 16
START_TIMEOUT = 180.0

_DIRECT = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def default_gguf() -> Path:
    xdg = os.environ.get("XDG_CACHE_HOME")
    cache = Path(xdg) / "bilbo" if xdg and os.path.isabs(xdg) else Path.home() / ".cache/bilbo"
    return cache / "models" / GGUF_FILE


def server_args(gguf: Path, port: int) -> list[str]:
    """The list of host::model::server_args."""
    return [
        "--model", str(gguf), "--alias", MODEL, "--embedding", "--pooling", "last",
        "--host", "127.0.0.1", "--port", str(port),
        "--ctx-size", "4096", "--batch-size", "4096", "--ubatch-size", "4096", "--parallel", "1",
    ]  # fmt: skip


def check_gguf(gguf: Path) -> str:
    if not gguf.is_file():
        raise Refused(f"no embedding model at {gguf}; pass --model")
    found = sha256_file(gguf)
    if found != GGUF_SHA256:
        raise Refused(f"{gguf} is not the pinned model\n  pinned: {GGUF_SHA256}\n  found:  {found}")
    return found


def _post(url: str, body: dict, timeout: float = 300) -> dict:
    req = urllib.request.Request(
        url, data=json.dumps(body).encode(), headers={"Content-Type": "application/json"}, method="POST"
    )
    with _DIRECT.open(req, timeout=timeout) as r:
        return json.loads(r.read())


def _get(url: str, timeout: float = 5) -> bytes:
    with _DIRECT.open(url, timeout=timeout) as r:
        return r.read()


def _free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _llama_version(llama_server: str) -> str:
    p = subprocess.run([llama_server, "--version"], capture_output=True, text=True, errors="replace", timeout=60)
    for line in (p.stdout + "\n" + p.stderr).splitlines():
        if line.strip().startswith("version:"):
            return line.strip()
    return tool_version([llama_server])


class Server:
    def __init__(self, url: str, version: str, gguf: Path | None, gguf_sha256: str | None, llama_server: str,
                 proc: subprocess.Popen | None = None) -> None:  # fmt: skip
        self.url, self.version, self.gguf, self.gguf_sha256 = url, version, gguf, gguf_sha256
        self.llama_server, self._proc = llama_server, proc
        self._scratch: Path | None = None

    @classmethod
    def start(cls, gguf: Path, llama_server: str = "llama-server") -> "Server":
        sha = check_gguf(gguf)
        if shutil.which(llama_server) is None and not Path(llama_server).is_file():
            raise Refused(f"`{llama_server}` is missing; put llama-server on PATH or pass --llama-server")
        version = _llama_version(llama_server)
        port = _free_port()
        proc = subprocess.Popen(
            [llama_server, *server_args(gguf, port)],
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )  # fmt: skip
        server = cls(f"http://127.0.0.1:{port}", version, gguf, sha, llama_server, proc)
        try:
            server._ready(START_TIMEOUT)
        except BaseException:
            server.stop()
            raise
        return server

    @classmethod
    def attach(cls, url: str, version: str = "attached") -> "Server":
        server = cls(url.rstrip("/"), version, None, None, "attached")
        server._ready(30)
        return server

    def _ready(self, timeout: float) -> None:
        deadline = time.monotonic() + timeout
        while True:
            if self._proc is not None and self._proc.poll() is not None:
                raise Refused(f"llama-server exited with {self._proc.returncode} before it was ready")
            try:
                _get(f"{self.url}/health")
                break
            except (urllib.error.URLError, OSError):
                if time.monotonic() > deadline:
                    raise Refused(f"the embedder at {self.url} did not answer /health in {timeout:.0f}s") from None
                time.sleep(0.5)
        try:
            vectors = self.embed(["probe"])
        except (urllib.error.URLError, OSError, KeyError, ValueError) as e:
            raise Refused(f"the embedder at {self.url} cannot embed: {e}") from e
        if len(vectors) != 1 or not vectors[0]:
            raise Refused(f"the embedder at {self.url} answered no vector for a probe")

    def embed(self, texts: list[str]) -> list[list[float]]:
        vectors: list[list[float]] = []
        for i in range(0, len(texts), BATCH):
            batch = texts[i : i + BATCH]
            data = _post(f"{self.url}/v1/embeddings", {"model": MODEL, "input": batch})["data"]
            data.sort(key=lambda d: d.get("index", 0))
            if len(data) != len(batch):
                raise Refused(f"the embedder answered {len(data)} vectors for {len(batch)} inputs")
            vectors += [d["embedding"] for d in data]
        return vectors

    def cache_dir(self) -> Path:
        """Where this server's vectors are kept: per GGUF hash and server version; a throwaway folder for an attached one."""
        if self.gguf_sha256 is None:
            if self._scratch is None:
                self._scratch = Path(tempfile.mkdtemp(prefix="bilbo-evals-vectors-"))
            return self._scratch
        return CACHE_DIR / self.gguf_sha256 / re.sub(r"[^A-Za-z0-9._()-]+", "_", self.version)

    def stop(self) -> None:
        if self._proc is not None:
            self._proc.terminate()
            try:
                self._proc.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self._proc.kill()
                self._proc.wait()
            self._proc = None
        if self._scratch is not None:
            shutil.rmtree(self._scratch, ignore_errors=True)
            self._scratch = None


class VectorCache:
    """Append-only `vectors.bin`: per record the 32-byte SHA-256 of the input, a uint32 dimension and float32 values."""

    def __init__(self, dir: Path) -> None:
        self.dir = dir
        self._lock = threading.Lock()
        self._vectors: dict[bytes, np.ndarray] = {}
        dir.mkdir(parents=True, exist_ok=True)
        self._file = dir / "vectors.bin"
        if self._file.exists():
            self._load()

    def _load(self) -> None:
        """Read the whole records; a partial record at the end is cut off so later appends stay aligned."""
        with open(self._file, "r+b") as f:
            fcntl.flock(f, fcntl.LOCK_EX)
            data = f.read()
            at = 0
            while at + 36 <= len(data):
                (dim,) = struct.unpack_from("<I", data, at + 32)
                end = at + 36 + 4 * dim
                if end > len(data):
                    break
                self._vectors[data[at : at + 32]] = np.frombuffer(data, dtype="<f4", count=dim, offset=at + 36).copy()
                at = end
            if at < len(data):
                f.truncate(at)

    @staticmethod
    def _key(text: str) -> bytes:
        return bytes.fromhex(sha256_bytes(text.encode()))

    def get(self, text: str) -> np.ndarray | None:
        with self._lock:
            return self._vectors.get(self._key(text))

    def put(self, text: str, vec) -> None:
        arr = np.asarray(vec, dtype="<f4")
        key = self._key(text)
        with self._lock:
            if key in self._vectors:
                return
            self._vectors[key] = arr
            with open(self._file, "ab") as f:
                fcntl.flock(f, fcntl.LOCK_EX)
                f.write(key + struct.pack("<I", len(arr)) + arr.tobytes())

    def __len__(self) -> int:
        return len(self._vectors)


class _ProxyHandler(BaseHTTPRequestHandler):
    def log_message(self, *args) -> None:
        pass

    def _send(self, code: int, body: dict) -> None:
        data = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self) -> None:
        try:
            data = _get(f"{self.server.upstream}{self.path}")
        except (urllib.error.URLError, OSError) as e:
            self._send(502, {"error": str(e)})
            return
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self) -> None:
        if self.path != "/v1/embeddings":
            self._send(404, {"error": "not found"})
            return
        body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))))
        texts = body["input"] if isinstance(body["input"], list) else [body["input"]]
        with self.server.lock:
            self.server.recorded.extend(texts)
        vectors: dict[int, list[float]] = {}
        missing: list[int] = []
        for i, text in enumerate(texts):
            cached = self.server.cache.get(text)
            if cached is None:
                missing.append(i)
            else:
                vectors[i] = [float(x) for x in cached]
        if missing:
            try:
                answer = _post(f"{self.server.upstream}/v1/embeddings", {**body, "input": [texts[i] for i in missing]})
            except (urllib.error.URLError, OSError, ValueError) as e:
                self._send(502, {"error": f"upstream embedder: {e}"})
                return
            data = sorted(answer["data"], key=lambda d: d.get("index", 0))
            if len(data) != len(missing):
                self._send(502, {"error": f"upstream embedder answered {len(data)} vectors for {len(missing)} inputs"})
                return
            for i, item in zip(missing, data):
                self.server.cache.put(texts[i], item["embedding"])
                vectors[i] = [float(x) for x in np.asarray(item["embedding"], dtype="<f4")]
        self._send(200, {
            "object": "list", "model": body.get("model", MODEL),
            "data": [{"object": "embedding", "index": i, "embedding": vectors[i]} for i in range(len(texts))],
        })  # fmt: skip


class Proxy:
    """Sits between bilbo and the embedder: records every input and answers from the vector cache when it can."""

    def __init__(self, upstream_url: str, cache_dir: Path) -> None:
        self.cache = VectorCache(cache_dir)
        self._server = ThreadingHTTPServer(("127.0.0.1", 0), _ProxyHandler)
        self._server.upstream = upstream_url.rstrip("/")
        self._server.cache = self.cache
        self._server.recorded = []
        self._server.lock = threading.Lock()
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)

    @classmethod
    def start(cls, upstream_url: str, cache_dir: Path) -> "Proxy":
        proxy = cls(upstream_url, cache_dir)
        proxy._thread.start()
        return proxy

    @property
    def url(self) -> str:
        return f"http://127.0.0.1:{self._server.server_address[1]}"

    def inputs(self) -> list[str]:
        with self._server.lock:
            return list(self._server.recorded)

    def stop(self) -> None:
        self._server.shutdown()
        self._server.server_close()
        self._thread.join(timeout=5)


_INDEX_LINE = re.compile(r"embedded (\d+), kept (\d+), dropped (\d+)")


def index_and_check(sb: sandbox.Sandbox, bilbo_exe: Path, server: Server, ds=None) -> dict:
    """Index the sandbox's store through the recording proxy and compare what bilbo embedded with the port's inputs.

    `embedded` counts the distinct inputs bilbo sent (index.rs dedups by key), so `embedded + kept` is compared with the
    distinct inputs of the port. A difference of the input sets comes back as parity "failed", not as an error.
    """
    expected = passages.inputs(sb.store)
    distinct = set(expected)
    proxy = Proxy.start(server.url, server.cache_dir())
    try:
        sandbox.write_config(sb, proxy.url)
        p = sandbox.bilbo(sb, bilbo_exe, ["index"], timeout=3600)
        recorded = set(proxy.inputs())
    finally:
        proxy.stop()
        sandbox.write_config(sb, server.url)
    log = (p.stdout + p.stderr).strip()
    if p.exit != 0:
        raise Refused(f"bilbo index exited {p.exit}: {log}")
    withheld = [line for line in log.splitlines() if "withheld" in line]
    if withheld:
        raise Refused(withheld[0].removeprefix("bilbo: ").strip())
    m = _INDEX_LINE.search(log)
    if not m:
        raise Refused(f"bilbo index printed no `embedded` line: {log}")
    embedded, kept = int(m[1]), int(m[2])
    differences = [{"side": "harness-only", "input": t} for t in sorted(distinct - recorded)] if kept == 0 else []
    differences += [{"side": "bilbo-only", "input": t} for t in sorted(recorded - distinct)]
    if not differences and embedded + kept != len(distinct):
        raise Refused(
            f"bilbo index embedded {embedded} and kept {kept}, but the store has {len(distinct)} distinct passage inputs"
        )
    return {
        "embedded": embedded,
        "inputs": len(distinct),
        "parity": "failed" if differences else "ok",
        "differences": differences,
    }


def record(server: Server) -> dict:
    return {
        "model": MODEL,
        "gguf_sha256": server.gguf_sha256,
        "llama_server": server.llama_server,
        "llama_server_version": server.version,
        "query_prefix": QUERY_PREFIX,
    }


def _shorten(text: str, n: int = 80) -> str:
    one = text.replace("\n", "\\n")
    return one if len(one) <= n else one[: n - 3] + "..."


def cmd_parity(args) -> int:
    import uuid

    from bilbo_evals.common import err

    source = Path(args.dataset) / "store"
    if not source.is_dir():
        raise Refused(f"no store at {source}")
    url = getattr(args, "embedder_url", None)
    server = Server.attach(url) if url else Server.start(Path(args.model), args.llama_server)
    sb = sandbox.create(f"parity-{uuid.uuid4().hex[:8]}")
    try:
        shutil.copytree(source, sb.store, dirs_exist_ok=True)
        result = index_and_check(sb, Path(args.bilbo), server)
    finally:
        sandbox.destroy(sb)
        server.stop()
    if result["parity"] == "ok":
        out(f"parity ok: {result['inputs']} inputs compared, no difference")
        return 0
    for d in result["differences"]:
        err(f"{d['side']}: {_shorten(d['input'])}")
    err(f"parity failed: {len(result['differences'])} of {result['inputs']} inputs differ")
    return 1
