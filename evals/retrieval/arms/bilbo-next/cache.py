"""A local HTTP server that stands at a provider's endpoint and keeps the answers that judge a request (../README.md).
Usage: cache.py --dir DIR --upstream URL --record FILE --timeout SECONDS [--write] [--ceiling N] [--port N]
[--tool PROGRAM]. It prints `port <n>` on standard output when it listens. With --write the real key is in its
environment, as BILBO_EVAL_CACHE_KEY. With --tool the way is `tool`: a request is what tool.py sends, the upstream
is the tool's name and version, PROGRAM is started for an ask, and the key is the token it is given."""

import argparse
import contextlib
import hashlib
import http.client
import json
import os
import signal
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit

STAND_IN = "replay-key-not-a-secret"
KEY_HEADERS = ("x-api-key", "authorization")
LEFT_OUT = KEY_HEADERS + ("host",)
RELAYED = ("retry-after", "request-id", "x-request-id")
REJECTED_NOT = (401, 402, 403, 404, 408, 409, 429)
FAILED = ("rate_limit_exceeded", "server_error", "data_residency_mismatch")
MISS = b'{"type":"error","error":{"type":"not_found_error","message":"no stored answer"}}'
TIMED_OUT = b'{"type":"error","error":{"type":"timeout_error","message":"the cache stopped waiting for the provider"}}'
UNREACHED = b'{"type":"error","error":{"type":"api_error","message":"the cache could not reach the provider"}}'
TOKEN_VARIABLE = "CLAUDE_CODE_OAUTH_TOKEN"
QUIET = {"DISABLE_AUTOUPDATER": "1", "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1", "DISABLE_TELEMETRY": "1"}
SHOWN = 200


def name_of(upstream, path, headers, body):
    kept = sorted((k.lower(), v) for k, v in headers if k.lower() not in LEFT_OUT)
    text = "".join(f"{k}: {v}\n" for k, v in kept)
    digest = hashlib.sha256()
    for part in (upstream.encode(), b"\n", path.encode(), b"\n", text.encode(), b"\n", body):
        digest.update(part)
    return digest.hexdigest()


def holds(key, data):
    return bool(key) and key.encode() in data


def judged(status, data):
    if status == 200:
        try:
            body = json.loads(data)
        except Exception:
            return True
        error = body.get("error") if isinstance(body, dict) else None
        failed = isinstance(body, dict) and body.get("status") == "failed"
        return not (failed and isinstance(error, dict) and error.get("code") in FAILED)
    return 400 <= status < 500 and status not in REJECTED_NOT


def told(code, out):
    try:
        reply = json.loads(out)
    except ValueError:
        return False
    if not isinstance(reply, dict):
        return False
    if reply.get("stop_reason") == "refusal":
        return True
    return code == 0 and reply.get("is_error") is not True and isinstance(reply.get("stop_reason"), str)


def status_of(code, out):
    """A reply `told` does not take has the status the API gave the tool, so `judged` keeps a rejection of the request."""
    if told(code, out):
        return 200
    try:
        status = json.loads(out).get("api_error_status")
    except (ValueError, AttributeError):
        return 500
    return status if isinstance(status, int) and 400 <= status < 600 else 500


def said(out, key):
    try:
        reply = json.loads(out)
        words = ": ".join(reply[field][:SHOWN] for field in ("subtype", "result") if isinstance(reply.get(field), str))
    except (ValueError, AttributeError):
        return "a reply that is no JSON object"
    words = words.encode(errors="replace")
    if holds(key, out) or holds(key, words):
        return "a reply that holds the key"
    return words.decode() or "a reply that says nothing of it"


def sync_file(handle):
    handle.flush()
    os.fsync(handle.fileno())


def sync_folder(folder):
    descriptor = os.open(folder, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def replace(source, target):
    os.replace(source, target)


def asks_in(folder):
    spent = Path(folder) / "spent"
    return sum(line.startswith("ask ") for line in spent.read_text().splitlines()) if spent.exists() else 0


class Cache:
    def __init__(self, folder, upstream, write, record, ceiling, timeout, key=None, clock=time.monotonic, tool=None):
        self.folder, self.upstream, self.write = Path(folder), upstream.rstrip("/"), write
        self.record, self.ceiling, self.timeout, self.key, self.clock = Path(record), ceiling, timeout, key, clock
        self.tool, self.home = tool, None
        self.given, self.locks, self.busy = {}, {}, set()
        self.guard, self.spend = threading.Lock(), threading.Lock()
        self.asks = asks_in(self.folder)

    def lock_of(self, name):
        with self.guard:
            return self.locks.setdefault(name, threading.Lock())

    def note(self, line):
        with self.spend, self.record.open("a") as handle:
            handle.write(line + "\n")

    def spare(self, name):
        with self.spend:
            if self.ceiling is not None and self.asks >= self.ceiling:
                return False
            self.asks += 1
            if not self.folder.exists():
                self.folder.mkdir(parents=True)
                sync_folder(self.folder.parent)
            spent = self.folder / "spent"
            new = not spent.exists()
            with spent.open("a") as handle:
                handle.write(f"ask {name}\n")
                sync_file(handle)
            if new:
                sync_folder(self.folder)
        return True

    def numbers(self, name):
        folder = self.folder / name
        return sorted(int(p.name) for p in folder.iterdir() if p.name.isdigit()) if folder.is_dir() else []

    def answer(self, path, headers, body):
        name = name_of(self.upstream, path, () if self.tool else headers, body)
        with self.lock_of(name):
            k, numbers = self.given.get(name, 0), self.numbers(name)
            if k < len(numbers):
                text = (self.folder / name / str(numbers[k])).read_bytes()
                status, _, data = text.partition(b"\n")
                if holds(self.key, data):
                    self.note(f"asked {name} key")
                    return 502, UNREACHED, {}
                self.given[name] = k + 1
                self.note(f"given {name} {numbers[k]}")
                return int(status), data, {}
            if not self.write:
                self.note(f"missed {name}")
                return 404, MISS, {}
            with self.guard:
                running = name in self.busy
            if running:
                self.note(f"busy {name}")
                return 408, TIMED_OUT, {}
            sent = self.with_key(path, headers, body)
            if sent is None:
                self.note(f"unkeyed {name}")
                return 502, UNREACHED, {}
            if not self.spare(name):
                self.note(f"missed {name}")
                return 404, MISS, {}
            status, data, relayed = self.run(name, path, sent, body)
            if status is None:
                self.note(f"asked {name} none")
                return relayed, data, {}
            if holds(self.key, data) or any(holds(self.key, value.encode()) for value in relayed.values()):
                self.note(f"asked {name} key")
                return 502, UNREACHED, {}
            if judged(status, data):
                number = numbers[-1] + 1 if numbers else 0
                self.keep(name, number, status, data, new=not numbers)
                self.given[name] = k + 1
                self.note(f"asked {name} kept {number}")
                self.note(f"given {name} {number}")
            else:
                self.note(f"asked {name} status {status}")
            return status, data, relayed

    def with_key(self, path, headers, body):
        if self.tool:
            carried = holds(self.key, body) or any(holds(self.key, value.encode()) for _, value in headers)
            return headers if self.key and not carried else None
        sent, replaced = [], 0
        for name, value in headers:
            if name.lower() == "x-api-key" and value == STAND_IN:
                value, replaced = self.key, replaced + 1
            elif name.lower() == "authorization" and value.split(None, 1) == ["Bearer", STAND_IN]:
                value, replaced = f"Bearer {self.key}", replaced + 1
            sent.append((name, value))
        left = STAND_IN in path or STAND_IN.encode() in body or any(STAND_IN in name or STAND_IN in value for name, value in sent)
        return sent if replaced == 1 and not left and self.key else None

    def run(self, name, path, headers, body):
        deadline = self.clock() + self.timeout
        ask = {"state": "running", "lock": threading.Lock(), "done": threading.Event(), "result": None, "stamp": None}

        def work():
            result = (None, UNREACHED, 502)
            try:
                result = self.fetch(path, headers, body)
            except TimeoutError as error:
                print(f"cache: the upstream call raised {type(error).__name__}", file=sys.stderr, flush=True)
                result = (None, TIMED_OUT, 408)
            except Exception as error:
                print(f"cache: the upstream call raised {type(error).__name__}", file=sys.stderr, flush=True)
            finally:
                with ask["lock"]:
                    ask["stamp"] = self.clock()
                    if ask["state"] == "abandoned":
                        with self.guard:
                            self.busy.discard(name)
                    else:
                        ask["state"], ask["result"] = "delivered", result
                ask["done"].set()

        threading.Thread(target=work, daemon=True).start()
        ask["done"].wait(self.timeout)
        with ask["lock"]:
            if ask["stamp"] is None or ask["stamp"] > deadline:
                if ask["stamp"] is None:
                    ask["state"] = "abandoned"
                    with self.guard:
                        self.busy.add(name)
                return None, TIMED_OUT, 408
        return ask["result"]

    def start(self, headers, body):
        asked = json.loads(body)
        run = json.loads(next(value for name, value in headers if name.lower() == "x-run"))
        with self.guard:
            if self.home is None:
                self.home = tempfile.mkdtemp()
        environment = run["env"] | asked["env"] | QUIET | {"HOME": self.home, TOKEN_VARIABLE: self.key}
        # Files, not pipes: a process the tool leaves behind then holds nothing the cache waits for.
        with tempfile.TemporaryFile() as given, tempfile.TemporaryFile() as printed:
            given.write(asked["stdin"].encode())
            given.seek(0)
            tool = subprocess.Popen([self.tool, *asked["argv"]], stdin=given, stdout=printed, stderr=subprocess.DEVNULL,
                                    env=environment, cwd=run["folder"], start_new_session=True)
            try:
                code = tool.wait(self.timeout)
            except subprocess.TimeoutExpired:
                raise TimeoutError from None
            finally:
                # As bilbo's own start of a tool, the group is stopped on every end: what is left of it holds the token.
                with contextlib.suppress(ProcessLookupError):
                    os.killpg(tool.pid, signal.SIGKILL)
                tool.wait()
            printed.seek(0)
            out = printed.read()
        status = status_of(code, out)
        if status != 200:
            print(f"cache: the tool ended with exit {code}: {said(out, self.key)}", file=sys.stderr, flush=True)
        return status, json.dumps({"exit": code, "stdout": out.decode(errors="surrogateescape")}).encode(), {}

    def fetch(self, path, headers, body):
        if self.tool:
            return self.start(headers, body)
        sent = {k: v for k, v in headers if k.lower() not in ("host", "content-length", "connection", "accept-encoding")}
        target = urlsplit(self.upstream + path)
        kind = http.client.HTTPSConnection if target.scheme == "https" else http.client.HTTPConnection
        connection = kind(target.hostname, target.port, timeout=self.timeout)
        try:
            connection.request("POST", target.path + (f"?{target.query}" if target.query else ""), body=body, headers=sent)
            response = connection.getresponse()
            whole = response.chunked or response.length is not None
            data = response.read()
            if not whole:
                raise ValueError("a body with no length and no chunks")
            return response.status, data, {k: response.getheader(k) for k in RELAYED if response.getheader(k)}
        finally:
            connection.close()

    def keep(self, name, number, status, data, new):
        folder = self.folder / name
        folder.mkdir(parents=True, exist_ok=True)
        part = folder / f"{number}.part"
        with part.open("wb") as handle:
            handle.write(f"{status}\n".encode() + data)
            sync_file(handle)
        replace(part, folder / str(number))
        sync_folder(folder)
        if new:
            sync_folder(self.folder)


def serve(cache, port=0):
    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            body = self.rfile.read(int(self.headers.get("content-length") or 0))
            status, data, relayed = cache.answer(self.path, list(self.headers.items()), body)
            self.send_response(status)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(data)))
            for name, value in relayed.items():
                self.send_header(name, value)
            self.send_header("connection", "close")
            self.end_headers()
            self.wfile.write(data)

        def log_message(self, *args):
            pass

    return ThreadingHTTPServer(("127.0.0.1", port), Handler)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", required=True)
    ap.add_argument("--upstream", required=True)
    ap.add_argument("--record", required=True)
    ap.add_argument("--write", action="store_true")
    ap.add_argument("--ceiling", type=int)
    ap.add_argument("--timeout", type=int, required=True)
    ap.add_argument("--port", type=int, default=0)
    ap.add_argument("--tool")
    args = ap.parse_args()
    cache = Cache(args.dir, args.upstream, args.write, args.record, args.ceiling, args.timeout, os.environ.get("BILBO_EVAL_CACHE_KEY"),
                  tool=args.tool)
    server = serve(cache, args.port)
    print(f"port {server.server_address[1]}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
