"""Build the claude-mem store: replay every session through claude-mem's own Claude Code hook command, hook by hook and
in one timeline across all sessions, into one claude-mem worker on loopback, set up with its docs' custom
OpenAI-compatible observer and Chroma embedding through the shared embedder. A Bun preload (clock.js) gives the worker
and every hook the time of the event being replayed, which claude-mem would otherwise stamp with the build's clock. Then
save the worker's data directory (SQLite, Chroma and its sync state) with a manifest in /work/store. Whenever no
session is open, the worker is stopped and its data goes to /work/checkpoint, and a run that finds it resumes from
there. `--count-chroma <dir>` is a mode of this file for the chroma-mcp environment: it counts a Chroma directory."""

import heapq
import http.client
import json
import os
import platform
import re
import shutil
import signal
import sqlite3
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
import uuid
from collections import Counter
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

NOTICES = ("<local-command-stdout>", "<local-command-stderr>", "<local-command-caveat>", "<system-reminder>")
CORPUS = Path("/corpus")
WORK = Path("/work")
CHECKPOINT = WORK / "checkpoint"
LIVE = Path("/tmp/live")
DATA = LIVE / "data"
CLOCK = LIVE / "clock"
HOME = Path("/tmp/home")
PLUGIN = Path("/opt/claude-mem/plugin")
WORKER = PLUGIN / "scripts" / "worker-service.cjs"
PORT = 37777
GATEWAY = ("127.0.0.1", 37800)
# How the observer's 3-minute idle wait (SessionMessageBuffer.ts:5) is honoured: "virtual" lets clock.js end it when the
# replay's clock passes 3 minutes; "real" leaves claude-mem's own timer and waits it out in real time wherever the
# replay predicts an exit.
IDLE = "virtual"
IDLE_MS = 180_000
SLICE = 240  # seconds the gateway holds one request before it answers with a rate limit
CONDENSE_SLICE = 170  # claude-mem gives up on a field condense at 180 s (field-optimizer.ts:72)
MIN_ATTEMPT = (
    60  # an upstream attempt starts only with a slice's length minus this left, so a slow upstream is not cut short
)
SKIP_TOOLS = (
    "listmcpresourcestool", "slashcommand", "skill", "todowrite", "askuserquestion")  # CLAUDE_MEM_SKIP_TOOLS default
EMBED_CTX = 8192
LAUNCHER = [
    "--python", "3.13", "--with", "onnxruntime>=1.20", "--with", "protobuf<7", "--with", "chromadb==1.5.9", "--with",
    "pydantic<2.14", "--from", "chroma-mcp==0.2.6"]
BASE_IMAGE = "ghcr.io/astral-sh/uv:0.12.19-python3.12-trixie-slim@sha256:10b94fe3550b7a96e3982860bb82cdefba6300264a7a40bb9334acf21a450974"
NPM_SHA256 = "a5d7b646541e1a839572095a31682b0e121929c1b3c6e80e29b27eb0a702a67a"
BUN_SHA256 = {
    "x86_64": "36368faef7527875d5ffa52e53cd48021741f2a83eb6208a8dd64068d422a913",
    "aarch64": "54328bbc2d9c8e0c9f892c544d66c57a83b84139e34909e5ee81758f1ac8fda7"}
EMBEDDER_URL = os.environ.get("EMBEDDER_URL", "")
LLM_URL = os.environ.get("BILBO_EVAL_LLM_URL", "").rstrip("/")
LLM_KEY = os.environ.get("BILBO_EVAL_LLM_KEY", "")
LLM_MODEL = os.environ.get("BILBO_EVAL_LLM_MODEL", "")
LLM_HEADERS = json.loads(os.environ.get("BILBO_EVAL_LLM_HEADERS") or "{}")

# What differs from a fresh install, each with where claude-mem 13.35.0's docs or source ask for it (paths in its repo).
# settings.json travels with the store.
SETTINGS = {
    "CLAUDE_MEM_PROVIDER": (
        "openai-compatible",
        "docs/public/configuration.mdx:17,125-134: a custom OpenAI-compatible observer, the eval's LLM"),
    "CLAUDE_MEM_OPENAI_COMPAT_BASE_URL": (
        f"http://{GATEWAY[0]}:{GATEWAY[1]}/v1",
        "docs/public/configuration.mdx:130: the gateway in this file, in front of BILBO_EVAL_LLM_URL"),
    "CLAUDE_MEM_OPENAI_COMPAT_MODEL": (LLM_MODEL, "docs/public/configuration.mdx:131: BILBO_EVAL_LLM_MODEL"),
    "CLAUDE_MEM_OBSERVER_CONTEXT_WINDOW": (
        "200000",
        "docs/public/configuration.mdx:167: a custom endpoint falls back to 131,072, so it is set to the doc's Claude value, 200,000, which is what claude-mem resolves for its default Claude observer (src/services/worker/context-window.ts), so the budgets equal the default provider's"),
    "CLAUDE_MEM_CHROMA_EMBEDDING_FUNCTION": (
        "openai",
        "src/shared/SettingsDefaultsManager.ts:504-510: the shared embedder is served on an OpenAI-compatible endpoint, which chroma-mcp's openai function reads from OPENAI_BASE_URL"),
    "CLAUDE_MEM_WORKER_AUTOSTART": (
        "false",
        "docs/public/configuration.mdx:29: false when you run the worker yourself (a container, `worker-service.cjs --daemon`)"),
    "CLAUDE_MEM_WORKER_PORT": (str(PORT), "docs/public/configuration.mdx:27: fixed instead of 37700 + uid % 100"),
}
# What else is set around claude-mem, outside settings.json.
ENV = {
    "DO_NOT_TRACK": ("1", "docs/public/telemetry.mdx:235: overrides every other telemetry switch"),
    "OPENAI_BASE_URL": (
        EMBEDDER_URL, "read by chroma-mcp's openai embedding function; claude-mem's own LLM path never reads it"),
    "OPENAI_API_KEY": ("local", "the embedder needs none; the openai function needs a value"),
    "BUN_OPTIONS": (
        "--preload /app/clock.js",
        "the user's decision (LEAD-LOG 2026-10-11): Date.now() and new Date() return the replayed event's time in the worker and every hook"),
    "BILBO_VIRTUAL_IDLE": (
        "1" if IDLE == "virtual" else "0",
        "the user's decision (LEAD-LOG 2026-10-11): the observer's 3-minute idle timer follows the same clock"),
}
# Log lines that mean claude-mem lost work (copied from the source, SessionManager.ts:88,713, ResponseProcessor.ts:564,
# GeneratorExitHandler.ts:129, hook-spool.ts:422): any fails the build.
LOSS_LINES = (
    "Batch parked: its paid-send budget is spent", "Session ended with parked batches",
    "Dropping the buffered summarize after its one rescue pass",
    "Hook spool entry was never accepted within its retry window")
# claude-mem drops a batch after a second reply that is not XML (ResponseProcessor.ts:564): the tool and the model
# behaving as they would live, so it is listed in the manifest, not fatal.
DROPPED_LINE = "response again for the same queued batch — dropping it"
# Counted in the manifest, not fatal.
COUNTED_LINES = {
    "rate_limit_retries": "{kind=rate_limit, outcome=", "compression_unusable": "Oversized field compression unusable",
    "compression_failed": "Oversized field compression failed",
    "cut_off": "reply was cut off at the output-token limit",
    "oversized_field": "Oversized field too large to condense; falling back to truncation",
    "non_xml": "returned non-XML"}
NOTES = {
    "hooks_not_replayed": "SessionStart and PreToolUse only read what claude-mem would show the live agent (GET /api/context/inject, /api/observations/by-file), and Setup installs dependencies, which the image does; none reaches the store",
    "serial": "each event waits until claude-mem has stored what it gave the observer before the clock moves, so no LLM call is in flight across a clock jump",
    "search": "/api/search drops Chroma hits older than 90 days unless dateStart is sent (SearchManager.ts:121,431, ChromaSearchStrategy.ts:196) and falls back to SQLite FTS5 when Chroma finds nothing (SearchOrchestrator.ts:135-174); this store's rows are dated by the replayed sessions, so the run step should send dateStart",
    "serve": "copy the store to a writable directory, set CLAUDE_MEM_DATA_DIR, OPENAI_BASE_URL and OPENAI_API_KEY, and start `bun worker-service.cjs --daemon`; no LLM is needed to search",
    "quiet_points": "the worker is stopped at each one, which finalizes open generators",
    "llm": "the calls behind this store: the calls of work an interrupt threw away are not counted",
    "projects": "a worktree's sessions are filed under <repo>/<worktree> keys (drain-cycle/ABA-301, ...) and stay there, since claude-mem adopts a worktree's rows into its repo only when the checkout is gone or merged (WorktreeAdoption.ts:470-560): a per-repo search must pass every key `projects` lists for the repo",
    "queries": "a search embeds the query like a document: the openai embedding function takes no query instruction, which Qwen3 would take (ai-memory sends one)",
    "image_only_prompts": "sent with an empty prompt as Claude Code sends them, and stored by claude-mem as [media prompt]; the mem0 and ai-memory arms skip them"}
HOOK_ENV = {
    "HOME": str(HOME), "PATH": "/usr/local/bin:/usr/bin:/bin", "CLAUDE_MEM_DATA_DIR": str(DATA),
    "CLAUDE_MEM_WORKER_PORT": str(PORT), "BUN_OPTIONS": ENV["BUN_OPTIONS"][0], "BILBO_CLOCK_FILE": str(CLOCK),
    "DO_NOT_TRACK": "1"}
WORKER_ENV = (
    HOOK_ENV
    | {"OPENAI_BASE_URL": EMBEDDER_URL, "OPENAI_API_KEY": "local"}
    | {k: os.environ[k] for k in os.environ if k.startswith("UV_")}
    | ({"BILBO_VIRTUAL_IDLE": "1"} if IDLE == "virtual" else {})
)
MARKERS = (".cleanup-v12.4.3-applied", ".fts-bloat-reclaim-started")


def log(msg):
    print(f"claude-mem: {msg}", file=sys.stderr, flush=True)


def stamp(entry):
    return datetime.fromisoformat(entry["timestamp"]) if entry.get("timestamp") else None


def millis(when):
    return int(when.timestamp() * 1000)


def iso(ms):
    return datetime.fromtimestamp(ms / 1000, timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")


def sessions():
    """Every Claude Code session as (start, repo, path), in the order they started across all repos. A transcript
    with no timestamp is not Claude Code's and is left out; the hooks never see it."""
    found, other = [], []
    for path in sorted(CORPUS.glob("*/*.jsonl")):
        start, seen = None, set()
        for entry in map(json.loads, path.open()):
            if entry.get("uuid") in seen:  # a line Claude Code wrote again fires no hook
                continue
            if entry.get("uuid"):
                seen.add(entry["uuid"])
            t = stamp(entry)
            start = t if t and (start is None or t < start) else start
        (found if start else other).append((start, path.parent.name, path))
    found.sort()
    limit = os.environ.get("BILBO_EVAL_SESSIONS")
    return found[: int(limit) if limit else None], [f"{repo}/{path.name}" for _, repo, path in other]


def checkout_roots():
    """Each repository's main checkout: the directory most of its sessions start in."""
    starts = {}
    for path in sorted(CORPUS.glob("*/*.jsonl")):
        cwd = next((e["cwd"] for e in map(json.loads, path.open()) if e.get("cwd")), None)
        if cwd:
            starts.setdefault(path.parent.name, Counter())[cwd] += 1
    return {repo: cwds.most_common(1)[0][0] for repo, cwds in starts.items()}


def text_of(content):
    if isinstance(content, str):
        return content.strip()
    return "\n".join(
        b.get("text", "") for b in content or [] if isinstance(b, dict) and b.get("type") == "text").strip()


def slash_command(text):
    """A slash command as the user typed it, from Claude Code's <command-name> record."""
    name = re.search(r"<command-name>(.*?)</command-name>", text, re.DOTALL)
    args = re.search(r"<command-args>(.*?)</command-args>", text, re.DOTALL)
    return " ".join(x for x in (name.group(1).strip() if name else "", args.group(1).strip() if args else "") if x)


def tool_results(content):
    return (
        [b for b in content if isinstance(b, dict) and b.get("type") == "tool_result"]
        if isinstance(content, list)
        else []
    )


def retry_after(headers):
    try:
        return int(float(headers.get("retry-after", 0)))
    except (ValueError, TypeError):  # an HTTP date: back off as usual
        return 0


def fetch(request, timeout):
    """The upstream's answer, an error status included."""
    try:
        with urllib.request.urlopen(request, timeout=timeout) as r:
            return r.status, r.headers, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.headers, e.read()


def kind_of(request):
    """Which of claude-mem's jobs made an LLM call, and the head of the user request its conversation is about."""
    messages = request.get("messages") or []
    texts = [m["content"] if isinstance(m.get("content"), str) else json.dumps(m.get("content")) for m in messages]
    last = texts[-1] if texts else ""
    if any("Reply with the condensed payload only" in t for t in texts):
        kind = "condense"  # field-optimizer.ts:109
    elif "MODE SWITCH: PROGRESS SUMMARY" in last:  # prompts.ts:6
        kind = "summary"
    elif "<occurred_at>" in last:
        kind = "observation"
    else:
        kind = "other"
    found = re.search(
        r"<user_request>(.*?)</user_request>", next((t for t in texts if "<user_request>" in t), ""), re.DOTALL)
    return kind, (found.group(1).strip() if found else "")[:80]


class Gateway(ThreadingHTTPServer):
    """claude-mem's LLM calls go through here to BILBO_EVAL_LLM_URL. claude-mem streams every call and gives up after
    90 s without a byte, and a failure after work may have started spends one of the two paid sends of a batch, after
    which the batch is parked and never resent; a refusal before work (a rate limit) spends nothing and is retried. So
    each call is answered at once as a stream with keep-alives, the upstream is called without streaming, a failed call
    is sent again the way mem0's build retries its add(), and a call that cannot be answered within its slice becomes a
    rate limit claude-mem retries. Every call to the LLM is counted."""

    daemon_threads = True

    def __init__(self, counts, models=()):
        super().__init__(GATEWAY, Forward)
        self.counts, self.lock, self.fatal = Counter(counts), threading.Lock(), threading.Event()
        self.in_flight, self.active = 0, time.monotonic()
        self.usable_at, self.attempt, self.hard = 0.0, 0, 0
        self.models = set(models)  # the model ids the upstream answered with
        self.calls = (WORK / "llm-calls.jsonl").open("a")

    def headers(self, request_id=None):
        return (
            {"Content-Type": "application/json", "Authorization": f"Bearer {LLM_KEY}"}
            | ({"x-client-request-id": request_id} if request_id else {})
            | LLM_HEADERS
        )

    def unusable(self):
        """Whether a wait the upstream asked for is pending. The failures behind it only scale the next wait."""
        with self.lock:
            return self.usable_at > time.monotonic()

    def complete(self, request, deadline, kind, head, request_id=None):
        """The upstream's answer to a chat completion as (status, json), or (429, None) when the slice has too little
        time left to wait and to call: claude-mem retries a rate limit a second later with a slice of its own."""
        data = json.dumps({k: v for k, v in request.items() if k not in ("stream", "stream_options")}).encode()
        slice_ = CONDENSE_SLICE if kind == "condense" else SLICE
        while True:
            if self.fatal.is_set():
                return 503, {"error": {"message": "the LLM failed 10 times in a row"}}
            with self.lock:
                wait = max(0.0, self.usable_at - time.monotonic())
            if deadline - time.monotonic() - wait < slice_ - MIN_ATTEMPT:
                return 429, None
            if wait > 0:
                time.sleep(wait)
                continue
            started, error = time.monotonic(), None
            try:
                status, answer, raw = fetch(
                    urllib.request.Request(
                        f"{LLM_URL}/chat/completions", data, self.headers(request_id), method="POST"),
                    max(1, deadline - time.monotonic()),
                )
            except (OSError, http.client.HTTPException) as e:  # refused, reset, timed out or cut short
                status, answer, raw, error = None, {}, b"", e
            try:
                parsed = json.loads(raw) if raw else {}
            except ValueError:
                parsed = {}
            self.record(kind, head, len(request.get("messages") or []), status, time.monotonic() - started, parsed)
            if status is not None and status != 429 and status < 500:
                with self.lock:
                    if status == 200:
                        self.attempt = self.hard = 0
                return status, parsed
            with self.lock:
                self.attempt += 1
                self.counts["retries"] += 1
                wait = min(60 * self.attempt, 600)
                if status == 429:
                    wait = max(wait, retry_after(answer))
                else:
                    self.hard += 1
                    if self.hard >= 10:
                        self.fatal.set()
                self.usable_at = time.monotonic() + wait
            log(f"LLM call failed ({status or error}); waiting {wait} s")

    def record(self, kind, head, messages, status, seconds, answer):
        usage = (answer.get("usage") or {}) if status == 200 else {}
        choice = ((answer.get("choices") or [{}])[0]) if status == 200 else {}
        empty = status == 200 and not (choice.get("message") or {}).get("content")
        with self.lock:
            self.counts[f"status_{status}"] += 1
            self.counts["seconds"] += seconds
            if status == 200:
                self.counts["calls"] += 1
                self.counts[f"calls_{kind}"] += 1
                self.counts["prompt_tokens"] += usage.get("prompt_tokens") or 0
                self.counts["completion_tokens"] += usage.get("completion_tokens") or 0
                self.counts["cached_tokens"] += (usage.get("prompt_tokens_details") or {}).get("cached_tokens") or 0
                self.counts["empty_answers"] += empty
                self.counts["cut_off"] += choice.get("finish_reason") == "length"
                if answer.get("model"):
                    self.models.add(answer["model"])
                if messages == 2 and kind in ("observation", "summary"):
                    self.counts["generations"] += 1
            line = {
                "time": datetime.now(timezone.utc).isoformat(timespec="seconds"), "kind": kind, "status": status,
                "attempt": self.attempt, "ms": round(seconds * 1000), "prompt_tokens": usage.get("prompt_tokens"),
                "completion_tokens": usage.get("completion_tokens"),
                "cached_tokens": (usage.get("prompt_tokens_details") or {}).get("cached_tokens"),
                "finish_reason": choice.get("finish_reason"), "messages": messages, "user_request": head}
            self.calls.write(json.dumps(line) + "\n")
            self.calls.flush()

    def check(self):
        if self.fatal.is_set():
            sys.exit("claude-mem: the LLM failed 10 times in a row; run again to resume from the last checkpoint")


class Forward(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        """GET /v1/models, forwarded as is."""
        try:
            status, headers, data = fetch(
                urllib.request.Request(LLM_URL + self.path.removeprefix("/v1"), headers=self.server.headers()), 60)
        except (OSError, http.client.HTTPException):
            status, headers, data = 502, {}, b"{}"
        self.send_response(status)
        self.send_header("Content-Type", headers.get("Content-Type") or "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        gateway = self.server
        arrived = time.monotonic()
        request = json.loads(self.rfile.read(int(self.headers.get("Content-Length") or 0)))
        kind, head = kind_of(request)
        with gateway.lock:
            gateway.in_flight += 1
            gateway.active = arrived
            gateway.counts["requests"] += 1
            gateway.counts["requests_observer"] += kind in ("observation", "summary")
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Retry-After", "1")
        self.send_header("Connection", "close")
        self.end_headers()
        wire, done = threading.Lock(), threading.Event()

        def emit(payload):
            with wire:
                try:
                    self.wfile.write(payload)
                    self.wfile.flush()
                except OSError:  # claude-mem gave up on this call
                    pass

        def keepalive():
            while not done.wait(15):
                emit(b": keepalive\n\n")

        threading.Thread(target=keepalive, daemon=True).start()
        try:
            status, answer = gateway.complete(
                request, arrived + (CONDENSE_SLICE if kind == "condense" else SLICE), kind, head,
                self.headers.get("x-client-request-id"))
        finally:
            done.set()
            with gateway.lock:
                gateway.in_flight -= 1
                gateway.active = time.monotonic()
        choice = ((answer or {}).get("choices") or [None])[0]
        if status == 200 and choice:
            chunks = [
                {
                    "id": answer.get("id"),
                    "model": answer.get("model"),
                    "choices": [
                        {
                            "index": 0,
                            "delta": {
                                "role": "assistant", "content": (choice.get("message") or {}).get("content") or ""},
                            "finish_reason": None,
                        }
                    ],
                },
                {
                    "id": answer.get("id"), "model": answer.get("model"),
                    "choices": [{"index": 0, "delta": {}, "finish_reason": choice.get("finish_reason") or "stop"}],
                    "usage": answer.get("usage")},
            ]
        elif status == 200 and isinstance((answer or {}).get("error"), dict):
            chunks = [{"error": answer["error"]}]
        elif answer is None:
            with gateway.lock:
                gateway.counts["rate_limit_relays"] += 1
            chunks = [
                {"error": {"code": 429, "type": "rate_limit_exceeded", "message": "the gateway is waiting for the LLM"}}]
        else:
            error = (
                answer.get("error")
                if isinstance(answer.get("error"), dict)
                else {"message": str(answer.get("error") or answer)[:300]}
            )
            chunks = [
                {
                    "error": {
                        "code": status if status != 200 else 502, "type": error.get("type"),
                        "message": error.get("message") or str(answer)[:300]}
                }
            ]
        emit(b"".join(f"data: {json.dumps(c)}\n\n".encode() for c in chunks) + b"data: [DONE]\n\n")
        self.close_connection = True


class Clock:
    """The time of the event being replayed, in the file clock.js reads. It never moves back."""

    def __init__(self, value=0):
        self.value = 0
        CLOCK.parent.mkdir(parents=True, exist_ok=True)
        self.set(value)

    def set(self, ms):
        self.value = max(self.value, ms)
        tmp = CLOCK.with_suffix(".tmp")
        tmp.write_text(str(self.value))
        os.replace(tmp, CLOCK)


class Checkouts:
    """The sessions' directories, under the paths they had. A repository's main checkout holds a minimal git
    repository with its GitHub remote, and a worktree a linked one, which is all claude-mem reads to name a project
    (git rev-parse --show-toplevel, the worktree's .git file); the subdirectories the sessions ran in sit inside."""

    WORKTREE = re.compile(r"^(.*?)/(?:\.worktrees|\.claude/worktrees)/([^/]+)$")

    def __init__(self, roots, launches):
        self.made = set()
        for repo, root in roots.items():
            self.main(root, repo)
        for repo, launch in launches:
            found = self.WORKTREE.match(launch)
            if found:
                self.main(found.group(1), repo)
                self.worktree(found.group(1), found.group(2), launch)

    def main(self, root, repo):
        git = Path(root) / ".git"
        (git / "objects").mkdir(parents=True, exist_ok=True)
        (git / "refs" / "heads").mkdir(parents=True, exist_ok=True)
        (git / "HEAD").write_text("ref: refs/heads/main\n")
        remote = f"https://github.com/{repo.replace('__', '/', 1)}.git"
        (git / "config").write_text(
            f'[core]\n\trepositoryformatversion = 0\n\tbare = false\n[remote "origin"]\n\turl = {remote}\n')

    def worktree(self, root, name, path):
        admin = Path(root) / ".git" / "worktrees" / name
        admin.mkdir(parents=True, exist_ok=True)
        (admin / "HEAD").write_text(f"ref: refs/heads/{name}\n")
        (admin / "commondir").write_text("../..\n")
        (admin / "gitdir").write_text(f"{path}/.git\n")
        Path(path).mkdir(parents=True, exist_ok=True)
        (Path(path) / ".git").write_text(f"gitdir: {admin}\n")

    def ensure(self, cwd):
        if cwd not in self.made:
            Path(cwd).mkdir(parents=True, exist_ok=True)
            self.made.add(cwd)


def query(sql, *args, path=None, immutable=False):
    """Read claude-mem's database without writing to it: the live one beside the worker, or a saved one."""
    c = sqlite3.connect(
        f"file:{path or DATA / 'claude-mem.db'}?mode=ro{'&immutable=1' if immutable else ''}", uri=True, timeout=30)
    try:
        return c.execute(sql, args).fetchall()
    finally:
        c.close()


def api(path, method="GET", timeout=10):
    request = urllib.request.Request(
        f"http://127.0.0.1:{PORT}{path}", b"{}" if method == "POST" else None, {"Content-Type": "application/json"},
        method=method)
    with urllib.request.urlopen(request, timeout=timeout) as r:
        return json.loads(r.read() or b"{}")


class Worker:
    """`worker-service.cjs --daemon` on loopback in this container, and the lines it logs."""

    def __init__(self, counts):
        self.process, self.offsets = None, {}
        self.idle, self.lost, self.backfilled, self.gone = (
            set(), [], False, set())  # gone: db ids whose generator paused or exited
        self.counted, self.errors, self.rejected, self.dropped = Counter(counts), [], {}, []

    def start(self, strict=True):
        self.backfilled = False
        if not (DATA / "settings.json").exists():
            DATA.mkdir(parents=True, exist_ok=True)
            (DATA / "settings.json").write_text(json.dumps({k: v for k, (v, _) in SETTINGS.items()}, indent=2) + "\n")
        self.process = subprocess.Popen(
            ["bun", str(WORKER), "--daemon"], env=WORKER_ENV, cwd=HOME, stdin=subprocess.DEVNULL,
            stdout=(WORK / "worker.out").open("a"), stderr=subprocess.STDOUT,
            start_new_session=True)  # its uvx and python children share its process group
        until = time.monotonic() + 240
        while True:
            self.check()
            try:
                api("/api/readiness", timeout=2)
                break
            except OSError:
                if time.monotonic() > until:
                    sys.exit("claude-mem: the worker did not answer /api/readiness in 4 minutes")
                time.sleep(0.5)
        provider = api("/api/health")["ai"]["provider"]
        if provider != "openai-compatible":
            sys.exit(
                f"claude-mem: the worker's observer is {provider}, not openai-compatible (provider-dispatch.ts falls back when it is not fully configured)")
        until = time.monotonic() + 600
        while not self.backfilled:
            self.check()
            self.pump()
            if time.monotonic() > until:
                sys.exit("claude-mem: the worker did not end its backfill check in 10 minutes")
            time.sleep(0.5)
        # The backfill ends early while a row stays pending, as the prompt the embedder rejects does: any
        # other pending row is a loss.
        unknown = [
            (k, i)
            for marks_ in sync_state().values()
            for k, ids in (marks_.get("pending") or {}).items()
            for i in ids
            if (k, i) not in self.rejected
        ]
        if strict and unknown:
            sys.exit(f"claude-mem: Chroma has pending rows the embedder did not reject: {unknown[:10]}")

    def check(self):
        if self.process.poll() is not None:
            sys.exit(
                f"claude-mem: the worker exited with {self.process.returncode}; see {WORK / 'worker.out'}, then run again to resume from the last checkpoint")

    def stop(self):
        """Shut the worker down the way claude-mem does, which finalizes every live session, then make sure nothing of it is left."""
        if self.process and self.process.poll() is None:
            try:
                api("/api/admin/shutdown", "POST")
            except OSError:
                pass
            try:
                self.process.wait(120)
            except subprocess.TimeoutExpired:
                self.kill()
                sys.exit("claude-mem: the worker did not stop in 120 s; run again to resume from the last checkpoint")
        self.pump()
        left = subprocess.run(["pgrep", "-f", "chroma-mcp"], capture_output=True, text=True).stdout.split()
        if left:
            subprocess.run(["pkill", "-9", "-f", "chroma-mcp"])
            sys.exit(
                f"claude-mem: chroma-mcp outlived the worker ({' '.join(left)}); run again to resume from the last checkpoint")
        self.backfilled = False

    def kill(self):
        """Stop what is left of the worker's process group, without any ceremony."""
        if self.process and self.process.poll() is None:
            try:
                os.killpg(self.process.pid, signal.SIGTERM)
                self.process.wait(30)
            except (ProcessLookupError, subprocess.TimeoutExpired):
                try:
                    os.killpg(self.process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass

    def status(self):
        for attempt in range(5):
            try:
                return api("/api/processing-status", timeout=30)
            except OSError:  # a busy worker can take longer than the request waits
                self.check()
                time.sleep(2)
        sys.exit(
            "claude-mem: the worker does not answer /api/processing-status; run again to resume from the last checkpoint")

    def pump(self):
        """Read the whole lines the worker logged since the last call, in every day's file."""
        for path in sorted((DATA / "logs").glob("*.log")):
            with path.open("rb") as f:
                f.seek(self.offsets.get(path.name, 0))
                data = f.read()
            data = data[: data.rfind(b"\n") + 1]
            self.offsets[path.name] = self.offsets.get(path.name, 0) + len(data)
            for line in data.decode(errors="replace").splitlines():
                self.read(line)

    def read(self, line):
        if (
            "Backfill check complete for all projects" in line
            or "Backfill check ended before every project finished" in line
        ):
            self.backfilled = True
        gone = re.search(r"\[session-(\d+)\] Generator (?:paused for|exited)", line)
        if gone:
            self.gone.add(int(gone.group(1)))
        if DROPPED_LINE in line:
            who = re.search(r"\[session-(\d+)\]", line)
            kind = re.search(r"non-XML (\w+) response again", line)
            self.dropped.append(
                {
                    "session_db_id": int(who.group(1)) if who else None, "kind": kind.group(1) if kind else None,
                    "time": line[1:24]}
            )
        found = re.search(r"Idle timeout reached.*?sessionDbId=(\d+)", line)
        if found:
            self.idle.add(int(found.group(1)))
        exited = re.search(r"Generator exited — finalizing session \{.*?reason=([^,}]*)", line)
        if exited and exited.group(1) not in ("idle", "shutdown"):
            self.lost.append(line)
        for loss in LOSS_LINES:
            if loss in line:
                self.lost.append(line)
        for name, marker in COUNTED_LINES.items():
            if marker in line:
                self.counted[name] += 1
        if "[ERROR]" in line:
            self.counted["error_lines"] += 1
            if len(self.errors) < 20:
                self.errors.append(line[:500])
        if self.lost:
            sys.exit(f"claude-mem: the worker lost work: {self.lost[0][:600]}")

    def wait_idle(self, db_id, timeout):
        """Whether the observer of this session exited because it was idle, within the seconds given."""
        until = time.monotonic() + timeout
        while time.monotonic() < until:
            self.check()
            self.pump()
            if db_id in self.idle:
                self.idle.discard(db_id)
                # The exit line follows within moments; without it a later claim could be taken for a stale exit.
                for _ in range(30):
                    self.pump()
                    if db_id in self.gone:
                        break
                    time.sleep(0.1)
                self.gone.discard(db_id)
                return True
            time.sleep(0.1)
        return False


def hook(name, payload, cwd, launch):
    """One event through claude-mem's hook command, as hooks.json runs it, reading Claude Code's JSON on stdin."""
    started = time.monotonic()
    try:
        r = subprocess.run(
            ["bun", str(WORKER), "hook", "claude-code", name],
            input=json.dumps(payload, separators=(",", ":")).encode(), env=HOOK_ENV | {"CLAUDE_PROJECT_DIR": launch},
            cwd=cwd, capture_output=True, timeout=120)
    except subprocess.TimeoutExpired:
        sys.exit(f"claude-mem: the {name} hook took over 120 s; run again to resume from the last checkpoint")
    if r.returncode:
        sys.exit(f"claude-mem: the {name} hook exited with {r.returncode}: {r.stderr.decode(errors='replace')[-1000:]}")
    return time.monotonic() - started


def skipped(name, tool_input):
    """Whether claude-mem's ingest drops the tool call before it stores it (shared.ts, CLAUDE_MEM_SKIP_TOOLS and session-memory files)."""
    if name.lower() in SKIP_TOOLS:
        return True
    path = (
        (tool_input.get("file_path") or tool_input.get("notebook_path") or "") if isinstance(tool_input, dict) else "")
    return name in ("Edit", "Write", "Read", "NotebookEdit") and "session-memory" in path


class Session:
    """One session through claude-mem's hook command, called as Claude Code calls the hooks claude-mem registers that
    reach its store: UserPromptSubmit for each prompt typed, PostToolUse or PostToolUseFailure for each tool result,
    Stop when a reply ends, SessionEnd at the end. Each step is one event, so sessions open at the same time interleave
    by time. The transcript file grows line by line, so each hook reads what Claude Code had written by then."""

    def __init__(self, repo, path, replay):
        self.name, self.replay = f"{repo}/{path.name}", replay
        self.lines = [line + "\n" for line in path.read_text().split("\n") if line.strip()]
        self.entries = [json.loads(line) for line in self.lines]
        self.times, when = [], None
        for entry in self.entries:
            when = stamp(entry) or when
            self.times.append(when)
        first_time = next(t for t in self.times if t)
        self.times = [t or first_time for t in self.times]
        self.sid = next(e["sessionId"] for e in self.entries if e.get("sessionId"))
        self.launch = next((e["cwd"] for e in self.entries if e.get("cwd")), "/")  # the first entries often carry none
        self.cwd = self.launch
        self.transcript = HOME / ".claude" / "projects" / re.sub("[^A-Za-z0-9]", "-", self.launch) / f"{self.sid}.jsonl"
        self.transcript.parent.mkdir(parents=True, exist_ok=True)
        self.out = self.transcript.open("w")
        self.turn, self.tools, self.index, self.ended = None, {}, -1, False
        self.prompt_id, self.mode = None, "default"
        self.expected = set()  # the tool_use_ids claude-mem should keep in tool_uses
        self.sent = Counter()
        # Claude Code sometimes writes a line after lines with later timestamps, and one session re-writes its earlier
        # history. A line whose uuid appeared before is written to the transcript and fires no hook. A session is placed
        # in the timeline by the earliest timestamp among the lines still to come that replay, so a late-written line is
        # delivered at its own time, before other sessions move the clock past it.
        seen, self.copied = set(), set()
        for k, entry in enumerate(self.entries):
            if entry.get("uuid") in seen:
                self.copied.add(k)
            elif entry.get("uuid"):
                seen.add(entry["uuid"])
        self.keys, following = [None] * len(self.entries), None
        for k in range(len(self.entries) - 1, -1, -1):
            own = self.times[k] if k not in self.copied and self.entries[k].get("timestamp") else None
            self.keys[k] = min(own, following) if own and following else own or following or self.times[k]
            following = self.keys[k]

    def next_time(self):
        return self.keys[min(max(self.index, 0), len(self.entries) - 1)]

    def following_time(self):
        """The time of the next hook after the step that comes: the next line's, or the session's end step."""
        if self.index >= len(self.entries):
            return None  # the end step: the next event is another session's
        return self.keys[min(self.index + 1, len(self.entries) - 1)]

    def send(self, hook_name, event, when, more=False, **fields):
        body = {
            "session_id": self.sid, "transcript_path": str(self.transcript), "cwd": self.cwd,
            "permission_mode": self.mode,
            "prompt_id": self.prompt_id or str(uuid.uuid5(uuid.NAMESPACE_URL, f"{self.sid}/{self.index}")),
            "hook_event_name": event, **fields}
        if event == "SessionEnd":
            del body["permission_mode"]  # Claude Code 2.1.296 sends none to SessionEnd
        self.sent[event] += 1
        message = event == "Stop" or (
            event.startswith("PostToolUse") and not skipped(fields["tool_name"], fields["tool_input"]))
        self.replay.deliver(self, hook_name, body, when, message=message, more=more)

    def prompt(self, text, when):
        self.send("session-init", "UserPromptSubmit", when, prompt=text)

    def stop(self, when, proven=False, more=False):
        """Stop for the turn that ends. A stop_hook_summary line proves Claude Code ran its Stop hooks, whatever the reply
        was (a usage limit's text, an API error), so it sends one for any turn not stopped yet."""
        turn = self.turn
        if not turn or turn["stopped"] or not (proven or (turn["reply"] and not turn["interrupted"])):
            return
        turn["stopped"] = True
        self.send(
            "summarize", "Stop", when, more=more, stop_hook_active=False, last_assistant_message=turn["reply"],
            background_tasks=[], session_crons=[])

    def turn_goes_on(self):
        """Whether the model goes on after this compaction before anything is typed: an automatic compaction can come
        in the middle of a turn, whose Stop Claude Code fires only when the turn ends."""
        for entry in self.entries[self.index + 1 :]:
            if (
                entry.get("type") not in ("user", "assistant")
                or entry.get("isSidechain")
                or entry.get("isMeta")
                or entry.get("isCompactSummary")
            ):
                continue
            content = (entry.get("message") or {}).get("content")
            if entry.get("type") == "assistant" or tool_results(content):
                return True
            text = text_of(content)
            if text and not text.startswith(NOTICES):
                return text.startswith("[Request interrupted")
        return False

    def step(self):
        if self.index < 0:
            self.index = 0
            self.replay.checkouts.ensure(self.launch)
        elif self.index < len(self.entries):
            self.event(self.lines[self.index], self.entries[self.index], self.times[self.index])
            self.index += 1
        else:
            self.stop(self.times[-1])
            self.send("session-end", "SessionEnd", self.times[-1], reason="other")
            self.out.close()
            self.transcript.unlink()
            self.ended = True

    def event(self, line, entry, when):
        kind, subtype, sidechain = entry.get("type"), entry.get("subtype"), entry.get("isSidechain")
        content = (entry.get("message") or {}).get("content")
        blocks = [b for b in content if isinstance(b, dict)] if isinstance(content, list) else []
        results = tool_results(content)
        if sidechain or self.index in self.copied:
            self.replay.counts["copied_lines"] += self.index in self.copied
            self.out.write(line)
            self.out.flush()
            return
        typed = kind == "user" and not (results or entry.get("isMeta") or entry.get("isCompactSummary"))
        text = text_of(content) if typed else ""
        image_only = typed and not text and any(b.get("type") == "image" for b in blocks)
        new_turn = (text and not text.startswith((*NOTICES, "[Request interrupted"))) or image_only
        if new_turn:
            self.stop(when, more=True)  # the turn before ends where it ran; this prompt's hook follows in the same step
        self.cwd = entry.get("cwd") or self.cwd  # Claude Code sends the cwd the session is in at each hook
        self.replay.checkouts.ensure(self.cwd)

        if kind == "system" and subtype in ("stop_hook_summary", "turn_duration"):
            self.stop(when, proven=subtype == "stop_hook_summary")
        elif kind == "system" and subtype == "compact_boundary":
            trigger = (entry.get("compactMetadata") or {}).get("trigger") or "auto"
            if trigger == "manual" or not self.turn_goes_on():  # a typed /compact comes between turns
                self.stop(when)
        elif text.startswith("[Request interrupted"):
            if self.turn:
                self.turn["interrupted"] = True
        elif new_turn:
            self.turn = {"reply": "", "interrupted": False, "stopped": False, "prompt": None}
            self.prompt_id = entry.get("promptId") or str(uuid.uuid5(uuid.NAMESPACE_URL, f"{self.sid}/{self.index}"))
            self.mode = entry.get("permissionMode") or self.mode
            if "<command-name>" in text:
                self.turn["prompt"] = slash_command(text)  # sent once the model answers: a local command sends none
            else:
                self.replay.counts[
                    (
                        "image_only_prompts"
                        if image_only
                        else "task_notification_prompts" if text.startswith("<task-notification>") else "typed_prompts"
                    )
                ] += 1
                self.prompt("" if image_only else text, when)
        elif kind == "assistant" and self.turn and self.turn["prompt"]:
            self.replay.counts["slash_prompts"] += 1
            self.prompt(self.turn["prompt"], when)
            self.turn["prompt"] = None

        self.out.write(line)
        self.out.flush()
        if kind == "assistant":
            for b in blocks:
                if b.get("type") == "tool_use":
                    self.tools[b.get("id")] = (b.get("name"), b.get("input") or {}, when)
            reply = text_of(content)
            api_error = bool(entry.get("isApiErrorMessage"))
            if reply and (api_error or (entry.get("message") or {}).get("model") == "<synthetic>"):
                # Claude Code's own text (an API error, a usage limit, "No response requested."),
                # not the model's: never a
                # reply. A turn an API error ends fires StopFailure, which claude-mem does not
                # register, instead of Stop.
                self.replay.counts["skipped_api_error_replies" if api_error else "skipped_synthetic_replies"] += 1
                if self.turn and api_error:
                    self.turn["interrupted"] = True
            elif self.turn and reply:
                # Text after a Stop means Claude went on (a Stop hook blocked it, a wakeup fired): Stop fires again.
                self.turn["reply"], self.turn["interrupted"], self.turn["stopped"] = reply, False, False
        elif kind == "user" and results:
            for b in results:
                name, tool_input, called = self.tools.get(b.get("tool_use_id"), ("unknown", {}, when))
                self.replay.counts["unknown_tools"] += name == "unknown"
                common = {
                    "tool_name": name, "tool_input": tool_input, "tool_use_id": b.get("tool_use_id"),
                    "duration_ms": max(0, round((when - called).total_seconds() * 1000))}
                if b.get("tool_use_id") and not skipped(name, tool_input):
                    self.expected.add(b["tool_use_id"])
                if b.get("is_error"):
                    error = text_of(b.get("content"))
                    self.send(
                        "observation", "PostToolUseFailure", when, **common, error=error,
                        is_interrupt=error.startswith("[Request interrupted by user"))
                else:
                    response = entry.get("toolUseResult")
                    self.send(
                        "observation", "PostToolUse", when, **common,
                        tool_response=text_of(b.get("content")) if response is None else response)


def docs_of(kind, row):
    """The Chroma documents claude-mem writes for a row, as (id, text) (ChromaSync.ts formatObservationDocs,
    formatSummaryDocs and the prompt document)."""
    if kind == "prompts":
        return [(f"prompt_{row['id']}", row["prompt_text"])]
    if kind == "summaries":
        return [
            (f"summary_{row['id']}_{f}", row[f])
            for f in ("request", "investigated", "learned", "completed", "next_steps", "notes")
            if row[f]
        ]
    try:
        facts = json.loads(row["facts"]) if row["facts"] else []
        facts = (
            [f for f in facts if isinstance(f, str) and f.strip()]
            if isinstance(facts, list)
            else ([facts] if isinstance(facts, str) and facts.strip() else [])
        )
    except ValueError:
        facts = [row["facts"]] if row["facts"].strip() else []
    docs = [(f"obs_{row['id']}_{f}", row[f]) for f in ("narrative", "text") if row[f]]
    docs += [(f"obs_{row['id']}_fact_{i}", fact) for i, fact in enumerate(facts)]
    if not docs:
        title = "" if (row["title"] or "").strip() == "Untitled" else (row["title"] or "").strip()
        text = "\n".join(p for p in (title, (row["subtitle"] or "").strip()) if p)
        if text:
            docs.append((f"obs_{row['id']}_title", text))
    return docs


def rows_of(kind, ids):
    table = {"observations": "observations", "summaries": "session_summaries", "prompts": "user_prompts"}[kind]
    c = sqlite3.connect(f"file:{DATA / 'claude-mem.db'}?mode=ro", uri=True, timeout=30)
    c.row_factory = sqlite3.Row
    try:
        return [c.execute(f"SELECT * FROM {table} WHERE id = ?", (i,)).fetchone() for i in ids]
    finally:
        c.close()


def tokens_of(text):
    """How many tokens the embedder counts in a document."""
    request = urllib.request.Request(
        EMBEDDER_URL.removesuffix("/v1") + "/tokenize", json.dumps({"content": text, "add_special": True}).encode(),
        {"Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=300) as r:
        return len(json.loads(r.read())["tokens"])


def spooled():
    """Whether a hook event waits in the spool: claude-mem removes an entry once its ingest has accepted it."""
    spool = DATA / "state" / "hook-spool"
    return spool.exists() and any(f.name.endswith(".json") for f in os.scandir(spool))


def sync_state():
    path = DATA / "chroma-sync-state.json"
    return json.loads(path.read_text()) if path.exists() else {}


def marks(state):
    return {p: [m.get("observations", 0), m.get("summaries", 0), m.get("prompts", 0)] for p, m in sorted(state.items())}


class Replay:
    """The clock, the worker and the gateway, and what the replay needs of them: each event moves the clock to its time,
    runs its hook, and waits until claude-mem has stored what the event gave the observer, so every LLM call starts at
    its event's time and none is in flight across a clock jump."""

    def __init__(self, clock, worker, gateway, checkouts, counts, idle, rejected):
        self.clock, self.worker, self.gateway, self.checkouts, self.counts, self.idle = (
            clock, worker, gateway, checkouts, counts, idle)
        self.alive, self.db_ids = {}, {}  # session id → the clock when its observer last took a message
        self.uncertain = set()  # alive sessions whose generator paused or exited during the drain that set them
        self.rejected = worker.rejected = rejected  # (kind, id) → tokens: rows the embedder refuses for length
        self.horizon = float("inf")  # the time of the next event of the timeline: the clock never steps past it

    def db_id(self, sid):
        if sid not in self.db_ids:
            rows = query("SELECT id FROM sdk_sessions WHERE content_session_id = ?", sid)
            if not rows:
                sys.exit(f"claude-mem: session {sid} has no sdk_sessions row")
            self.db_ids[sid] = rows[0][0]
        return self.db_ids[sid]

    def advance(self, t):
        """Move the clock to t. The observer of a session exits when 3 minutes pass with no message: bring the clock to
        each due exit first and wait for it. A session whose generator paused or exited meanwhile has no exit to wait for.
        """
        self.worker.pump()
        for sid in [s for s in self.alive if self.db_id(s) in self.worker.gone]:
            del self.alive[sid]
            self.uncertain.discard(sid)
        for sid, due in sorted(((s, last + IDLE_MS) for s, last in self.alive.items()), key=lambda x: x[1]):
            if due > t:
                break
            self.clock.set(due)
            self.idle["predicted"] += 1
            if self.worker.wait_idle(self.db_id(sid), 15 if IDLE == "virtual" else 200):
                self.idle["observed"] += 1
            elif sid in self.uncertain:
                self.idle[
                    "uncertain_missed"] += 1  # its generator paused or exited during a drain: the exit may never come
            elif IDLE == "virtual":
                sys.exit(
                    f"claude-mem: the observer of session {sid} did not exit idle at {iso(due)}, so the replay's clock model is wrong; "
                    "run again to resume from the last checkpoint"
                )
            else:
                self.idle["missed"] += 1
            del self.alive[sid]
            self.uncertain.discard(sid)
        self.clock.set(t)

    def deliver(self, session, name, payload, when, message, more=False):
        self.advance(millis(when))
        self.checkouts.ensure(session.cwd)
        # Counted before the hook: the observer's request can reach the gateway before the hook's spool entry is gone.
        seen = self.gateway.counts["requests_observer"]
        self.counts["hook_seconds"] += hook(name, payload, session.cwd, session.launch)
        self.counts[payload["hook_event_name"]] += 1
        if name != "session-init":  # that hook has already waited for the worker to take the prompt
            self.wait_spool()
        if not message:
            return
        self.counts["messages"] += 1
        # Lines from earlier drains are stale now that the session's row exists; a pause or exit logged during this
        # drain makes its idle exit uncertain.
        db_id = self.db_id(session.sid)
        self.worker.pump()
        self.worker.idle.discard(db_id)
        self.worker.gone.discard(db_id)
        # Another hook of this step follows at the same time, so the clock may not step: it would stamp that hook late.
        if not self.wait_drain(millis(when) if more else self.horizon):
            self.alive.pop(session.sid, None)  # its generator is paused behind a rate limit: no idle exit is coming
            self.uncertain.discard(session.sid)
        elif self.gateway.counts["requests_observer"] > seen:
            self.worker.pump()
            self.alive[session.sid] = self.clock.value  # what claude-mem's claim saw, and clock.js arms its timer from
            (self.uncertain.add if db_id in self.worker.gone else self.uncertain.discard)(session.sid)

    def wait_spool(self):
        until = time.monotonic() + 120
        while spooled():
            self.worker.check()
            if time.monotonic() > until:
                sys.exit("claude-mem: a hook event stayed in the spool for 2 minutes")
            time.sleep(0.05)

    def wait_drain(self, horizon):
        """Wait until claude-mem has stored what the last event gave the observer. While the upstream refuses, the wait
        goes on, as mem0's add() blocks. Once it answers but claude-mem is still held by its rate-limit breaker, whose
        90 s window is virtual, the clock steps by at most 91 s, never past the next event of the timeline. False when it
        cannot step and nothing has moved for 120 s: the replay then goes on, and the manifest counts it."""
        told = quiet_since = time.monotonic()
        limited = False
        while True:
            self.worker.check()
            self.gateway.check()
            self.worker.pump()
            status = self.worker.status()
            if status["queueDepth"] == 0 and status.get("parkedSessions", 0) == 0 and not self.gateway.in_flight:
                return True
            quiet_since = max(quiet_since, self.gateway.active)
            if not self.gateway.in_flight:
                if self.gateway.unusable():
                    if not limited:
                        self.counts["limit_waits"] += 1
                        log("the LLM refuses calls; waiting for it")
                    limited, quiet_since = True, time.monotonic()
                elif (limited and horizon - 1 - self.clock.value < 91_000 and time.monotonic() - quiet_since > 10) or (
                        time.monotonic() - quiet_since > 120):
                    # After a limit claude-mem's breaker needs 90 virtual seconds: with less room than that before the
                    # next event no step can open it, so the replay goes on at once instead of
                    # waiting 190 s for nothing.
                    room = horizon - 1 - self.clock.value
                    if room <= 0 or (limited and room < 91_000):
                        self.counts["stall_continues"] += 1
                        log(
                            f"claude-mem holds {status['queueDepth']} messages behind its rate-limit breaker and the clock cannot step: the replay goes on without them")
                        return False
                    self.counts["breaker_steps"] += 1
                    self.advance(self.clock.value + min(91_000, room))
                    quiet_since = time.monotonic() - 50  # the breaker's sweep re-probes within 60 s
                    limited = False
            if time.monotonic() - told > 300:
                log(f"{status['queueDepth']} messages wait for the LLM")
                told = time.monotonic()
            time.sleep(0.1)

    def caught_up(self):
        """What Chroma still lacks: the (project, kind) pairs behind SQLite's newest row, and the ids it failed to write."""
        state, want = sync_state(), {}
        for kind, sql in (
            ("observations", "SELECT project, MAX(id) FROM observations GROUP BY project"),
            ("summaries", "SELECT project, MAX(id) FROM session_summaries GROUP BY project"),
            (
                "prompts",
                "SELECT s.project, MAX(p.id) FROM user_prompts p JOIN sdk_sessions s ON s.id = p.session_db_id GROUP BY s.project"),
        ):
            for project, top in query(sql):
                want[(project, kind)] = top
        behind = [(p, k) for (p, k), top in want.items() if (state.get(p) or {}).get(k, 0) < top]
        pending = [(k, i) for marks_ in state.values() for k, ids in (marks_.get("pending") or {}).items() for i in ids]
        return behind, pending

    def wait_chroma(self):
        """Wait until Chroma holds every row, but for the ones the embedder rejects for length, which claude-mem keeps
        pending: those are recorded in `self.rejected`. Pending rows that are not rejections are retried once by a worker
        restart, whose backfill resends them (writes made during chroma-mcp's 10 s reconnect backoff are skipped under
        the frozen clock); any still pending after that fail the build."""
        progress, last, restarted = None, time.monotonic(), False
        while True:
            self.worker.check()
            behind, pending = self.caught_up()
            for kind, i in pending:
                if (kind, i) in self.rejected:
                    continue
                row = rows_of(kind, [i])[0]
                big = max((tokens_of(text) for _, text in docs_of(kind, row)), default=0)
                if big > EMBED_CTX:
                    self.rejected[(kind, i)] = big
            open_ = [p for p in pending if p not in self.rejected]
            if not behind and not open_:
                return
            if (behind, open_) != progress:
                progress, last = (behind, open_), time.monotonic()
            elif open_ and not restarted and time.monotonic() - last > 60:
                log(f"Chroma still lacks {open_[:5]}: restarting the worker once, whose backfill retries them")
                restarted = True
                self.worker.stop()
                self.worker.start(strict=False)
                last = time.monotonic()
            elif time.monotonic() - last > 600:
                sys.exit(f"claude-mem: Chroma is behind SQLite: {behind} {open_}")
            time.sleep(0.5)

    def settle(self, done, progress, next_start):
        """Everything of a quiet point before the worker stops; False when the replay is stalled and cannot go on."""
        spool = DATA / "state" / "hook-spool"
        if not self.wait_drain(next_start):
            return False
        if spooled() or any((spool / d).exists() and any((spool / d).iterdir()) for d in ("corrupt", "expired")):
            sys.exit("claude-mem: the hook spool holds events that were never ingested")
        self.worker.pump()
        for session in done:
            stored = query("SELECT COUNT(*) FROM tool_uses WHERE content_session_id = ?", session.sid)[0][0]
            if stored != len(session.expected):
                sys.exit(
                    f"claude-mem: {session.name} sent {len(session.expected)} tool calls to keep and tool_uses holds {stored}")
        self.wait_chroma()
        progress["rejected"] = [[k, i, n] for (k, i), n in sorted(self.rejected.items())]
        return True

    def record(self, session, progress):
        sid = session.sid
        count = {
            "prompts": query("SELECT COUNT(*) FROM user_prompts WHERE content_session_id = ?", sid)[0][0],
            "observations": query(
                "SELECT COUNT(*) FROM observations o JOIN sdk_sessions s ON s.memory_session_id = o.memory_session_id WHERE s.content_session_id = ?",
                sid)[0][0],
            "summaries": query(
                "SELECT COUNT(*) FROM session_summaries m JOIN sdk_sessions s ON s.memory_session_id = m.memory_session_id WHERE s.content_session_id = ?",
                sid)[0][0],
            "tool_uses": query("SELECT COUNT(*) FROM tool_uses WHERE content_session_id = ?", sid)[0][0],
        }
        progress["sessions"][session.name] = {"sid": sid, "db_id": self.db_id(sid), "sent": dict(session.sent)} | count

    def quiet_point(self, done, progress, next_start, final=False):  # True when the data was saved
        """No session is open: wait until claude-mem has stored everything, stop the worker so nothing is in flight,
        and save its data. A checkpoint also starts the worker again."""
        if not self.settle(done, progress, next_start):
            self.counts["skipped_checkpoints"] += 1
            log("the LLM is unavailable and the next session starts soon: no checkpoint here")
            return False
        state = marks(sync_state())
        for session in done:
            self.record(session, progress)
        progress["done"] += [s.name for s in done]
        self.check_run()
        self.worker.stop()
        self.alive.clear()
        self.db_ids.clear()
        progress["marks"] = state
        progress["clock"] = self.clock.value
        progress["idle"] = dict(self.idle)
        progress["counts"] = dict(self.counts)
        with self.gateway.lock:
            progress["llm"] = dict(self.gateway.counts)
        progress["log"] = {
            "counted": dict(self.worker.counted), "errors": self.worker.errors, "dropped": self.worker.dropped}
        progress["models"] = sorted(self.gateway.models)
        save(progress)
        if not final:
            self.worker.start()
            self.verify_marks(progress)
        return True

    def check_run(self):
        """What would fail the build at its end, checked at each quiet point instead, before more of the LLM is spent."""
        with self.gateway.lock:
            calls = self.gateway.counts["calls"]
        if calls < self.counts["messages"]:
            sys.exit(
                f"claude-mem: the gateway answered {calls} calls for {self.counts['messages']} messages; run again to resume from the last checkpoint")
        served = {m for (m,) in query("SELECT DISTINCT generated_by_model FROM observations") if m}
        allowed = self.gateway.models or {
            LLM_MODEL}  # an upstream that names no model leaves claude-mem the configured one
        if served - allowed:
            sys.exit(
                f"claude-mem: observations were generated by {sorted(served - allowed)}, not by a model the LLM answered with {sorted(allowed)}")

    def verify_marks(self, progress):
        if marks(sync_state()) != progress["marks"]:
            sys.exit(
                f"claude-mem: the sync state moved while the worker started again: {marks(sync_state())} instead of {progress['marks']}")


def copy_store(dest):
    """The worker's data as a store: SQLite folded into one file, Chroma without its writer lock, the sync state and
    settings, and the markers that keep claude-mem's one-time maintenance from running on a copy."""
    shutil.rmtree(dest, ignore_errors=True)
    dest.mkdir(parents=True)
    for name in (
        "claude-mem.db", "claude-mem.db-wal", "claude-mem.db-shm", "chroma-sync-state.json", "settings.json", *MARKERS):
        if (DATA / name).exists():
            shutil.copy2(DATA / name, dest / name)
    if (DATA / "chroma").exists():
        shutil.copytree(
            DATA / "chroma", dest / "chroma", ignore=shutil.ignore_patterns(".claude-mem-chroma-writer.lock"))
    if (dest / "claude-mem.db-wal").exists():
        c = sqlite3.connect(dest / "claude-mem.db")
        c.execute("PRAGMA wal_checkpoint(TRUNCATE)")
        c.close()
        for name in ("claude-mem.db-wal", "claude-mem.db-shm"):
            (dest / name).unlink(missing_ok=True)


def save(progress):
    """Write the checkpoint: the swap is two renames, and a run that finds only checkpoint.old uses it."""
    tmp, old = WORK / "checkpoint.tmp", WORK / "checkpoint.old"
    copy_store(tmp)
    (tmp / "progress.json").write_text(json.dumps(progress, indent=2) + "\n")
    shutil.rmtree(old, ignore_errors=True)
    if CHECKPOINT.exists():
        CHECKPOINT.rename(old)
    tmp.rename(CHECKPOINT)
    shutil.rmtree(old, ignore_errors=True)


def restore():
    shutil.copytree(CHECKPOINT, DATA, ignore=shutil.ignore_patterns("progress.json"))
    return json.loads((CHECKPOINT / "progress.json").read_text())


def verify_counts(progress):
    """A resumed store must hold exactly what the checkpoint recorded for each finished session: nothing re-sent."""
    wrong = []
    for name, s in progress["sessions"].items():
        sid = s["sid"]
        found = {
            "prompts": query("SELECT COUNT(*) FROM user_prompts WHERE content_session_id = ?", sid)[0][0],
            "observations": query(
                "SELECT COUNT(*) FROM observations o JOIN sdk_sessions x ON x.memory_session_id = o.memory_session_id WHERE x.content_session_id = ?",
                sid)[0][0],
            "summaries": query(
                "SELECT COUNT(*) FROM session_summaries m JOIN sdk_sessions x ON x.memory_session_id = m.memory_session_id WHERE x.content_session_id = ?",
                sid)[0][0],
            "tool_uses": query("SELECT COUNT(*) FROM tool_uses WHERE content_session_id = ?", sid)[0][0],
        }
        if found != {k: s[k] for k in found}:
            wrong.append(name)
    if wrong or marks(sync_state()) != progress["marks"]:
        sys.exit(f"claude-mem: the restored store does not match its checkpoint: {wrong or 'the Chroma sync state'}")


def count_chroma(path):
    """Run in the chroma-mcp environment: how many documents a Chroma directory holds, by doc_type."""
    import chromadb

    collection = chromadb.PersistentClient(path=path).get_collection("cm__claude-mem")
    metadatas = collection.get(include=["metadatas"])["metadatas"]
    print(json.dumps({"count": len(metadatas), "by_doc_type": dict(Counter(m.get("doc_type") for m in metadatas))}))


def chroma_counts():
    """Counted on a copy: opening a Chroma directory writes to it."""
    copy = LIVE / "chroma-count"
    shutil.rmtree(copy, ignore_errors=True)
    if not (WORK / "store" / "chroma").exists():
        return {"count": 0, "by_doc_type": {}}
    shutil.copytree(WORK / "store" / "chroma", copy)
    r = subprocess.run(
        ["uvx", *LAUNCHER, "python", "/app/build.py", "--count-chroma", str(copy)], capture_output=True, text=True,
        env=os.environ | {"OPENAI_BASE_URL": EMBEDDER_URL, "OPENAI_API_KEY": "local"})
    shutil.rmtree(copy, ignore_errors=True)
    if r.returncode:
        sys.exit(f"claude-mem: counting the Chroma collection failed: {r.stderr.strip()[-2000:]}")
    return json.loads(r.stdout.strip().splitlines()[-1])


def tool_versions():
    def out(*cmd):
        return subprocess.run(cmd, capture_output=True, text=True).stdout.strip().splitlines()[0]

    packages = subprocess.run(
        [
            "uvx", *LAUNCHER, "python", "-c",
            "import importlib.metadata as m, json; print(json.dumps({d.metadata['Name']: d.version for d in m.distributions()}))"],
        capture_output=True,
        text=True,
    )
    env = json.loads(packages.stdout.strip().splitlines()[-1]) if packages.returncode == 0 else {}
    return {
        "claude-mem": "13.35.0", "npm_tarball_sha256": NPM_SHA256, "bun": out("bun", "--version"),
        "bun_zip_sha256": BUN_SHA256[platform.machine()], "python": "3.13.15", "uv": out("uv", "--version"),
        "git": out("git", "--version"), "procps": out("ps", "--version"), "chroma_env": dict(sorted(env.items())),
        "base_image": BASE_IMAGE, "arch": platform.machine(), "driver_python": platform.python_version()}


def manifest(progress, skipped_files, plan, replayed):
    inputs = (WORK / "key.txt").read_text().splitlines()
    saved = {"path": WORK / "store" / "claude-mem.db", "immutable": True}
    projects = {}
    for name, s in progress["sessions"].items():
        found = query("SELECT project, status FROM sdk_sessions WHERE content_session_id = ?", s["sid"], **saved)
        projects.setdefault(name.split("/")[0], Counter())[found[0][0] if found else "?"] += 1
    statuses = dict(query("SELECT status, COUNT(*) FROM sdk_sessions GROUP BY status", **saved))
    chroma = chroma_counts()
    counts, idle, llm = progress["counts"], progress["idle"], progress["llm"]
    events = ("UserPromptSubmit", "PostToolUse", "PostToolUseFailure", "Stop", "SessionEnd")
    config = [
        {"key": k, "value": v, "cite": cite}
        for k, (v, cite) in (SETTINGS | ENV).items()
        if k != "CLAUDE_MEM_OPENAI_COMPAT_MODEL"
    ]
    config.append(
        {
            "key": "CLAUDE_MEM_OPENAI_COMPAT_MODEL", "value": LLM_MODEL,
            "cite": SETTINGS["CLAUDE_MEM_OPENAI_COMPAT_MODEL"][1]}
    )
    times = [
        t
        for _, _, path in plan
        for t in (stamp(json.loads(line)) for line in path.read_text().split("\n") if line.strip())
        if t
    ]
    models = [m for (m,) in query("SELECT DISTINCT generated_by_model FROM observations", **saved)]
    return {
        "arm": "claude-mem",
        "key": inputs[0],
        "inputs": inputs[1:],
        "device": os.environ["BILBO_EVAL_DEVICE"],
        "versions": tool_versions(),
        "config": config,
        "embedder": {
            "function": "openai", "requested_model": "text-embedding-ada-002",
            "served_by": "Qwen3-Embedding-0.6B Q8_0 on llama.cpp b11206", "dim": 1024, "space": "cosine",
            "query_instruction": None, "env": ["OPENAI_BASE_URL", "OPENAI_API_KEY"]},
        "time": {
            "clock": "virtual: Date.now() and new Date() return the replayed event's transcript time in the worker and every hook (Bun preload clock.js), "
            "frozen between events, never backwards",
            "idle_timer": IDLE if IDLE == "virtual" else "real wait",
            "first_event": iso(millis(min(times))),
            "last_event": iso(millis(max(times))),
            "rows": "dated by the clock: sessions, prompts, observations, summaries, Chroma metadata",
        },
        "notes": NOTES,
        "projects": {repo: dict(found) for repo, found in sorted(projects.items())},
        "sessions": {
            "replayed": replayed, "skipped": skipped_files, "completed": statuses.get("completed", 0),
            "active": statuses.get("active", 0)},
        "events": {k: counts.get(k, 0) for k in events},
        "replay": {
            "messages": counts.get("messages", 0),
            "skipped_replies": {
                "api_error": counts.get("skipped_api_error_replies", 0),
                "synthetic": counts.get("skipped_synthetic_replies", 0)},
            "prompts": {
                "typed": counts.get("typed_prompts", 0), "slash": counts.get("slash_prompts", 0),
                "task_notification": counts.get("task_notification_prompts", 0),
                "image_only": counts.get("image_only_prompts", 0)},
            "unknown_tools": counts.get("unknown_tools", 0),
            "idle_exits": {k: idle.get(k, 0) for k in ("predicted", "observed", "missed")},
            "hook_seconds": round(counts.get("hook_seconds", 0)),
            "copied_lines": counts.get("copied_lines", 0),
            "idle_uncertain_missed": idle.get("uncertain_missed", 0),
            "dropped_by_claude_mem": [
                {
                    "session": (
                        query("SELECT content_session_id FROM sdk_sessions WHERE id = ?", d["session_db_id"], **saved)
                        or [[None]]
                    )[0][0]
                }
                | {k: d[k] for k in ("kind", "time")}
                for d in progress["log"].get("dropped", [])
            ],
            "limit_waits": counts.get("limit_waits", 0),
            "breaker_steps": counts.get("breaker_steps", 0),
            "stall_continues": counts.get("stall_continues", 0),
            "skipped_checkpoints": counts.get("skipped_checkpoints", 0),
        },
        "rows": {
            "sdk_sessions": query("SELECT COUNT(*) FROM sdk_sessions", **saved)[0][0],
            "user_prompts": query("SELECT COUNT(*) FROM user_prompts", **saved)[0][0],
            "observations": {
                "total": query("SELECT COUNT(*) FROM observations", **saved)[0][0],
                "by_type": dict(query("SELECT type, COUNT(*) FROM observations GROUP BY type ORDER BY type", **saved))},
            "session_summaries": query("SELECT COUNT(*) FROM session_summaries", **saved)[0][0],
            "tool_uses": query("SELECT COUNT(*) FROM tool_uses", **saved)[0][0],
            "generated_by_model": sorted(m for m in models if m),
        },
        "chroma": {
            "collection": "cm__claude-mem", "documents": chroma["count"], "by_doc_type": chroma["by_doc_type"],
            "rejected_by_embedder": [{"kind": k, "id": i, "tokens": n} for k, i, n in progress["rejected"]]},
        "worker_log": progress["log"],
        "llm": {"model": LLM_MODEL} | {k: round(v) if isinstance(v, float) else v for k, v in sorted(llm.items())},
        "ingest_seconds": round(progress["ingest_seconds"]),
        "finished": datetime.now(timezone.utc).isoformat(timespec="seconds"),
    }


def interrupted(signum, frame):
    sys.exit("claude-mem: stopped; run again to resume from the last checkpoint")


def main():
    if sys.argv[1:2] == ["--count-chroma"]:
        return count_chroma(sys.argv[2])
    signal.signal(signal.SIGINT, interrupted)
    signal.signal(signal.SIGTERM, interrupted)
    try:
        urllib.request.urlopen(
            urllib.request.Request(f"{LLM_URL}/models", headers={"Authorization": f"Bearer {LLM_KEY}"} | LLM_HEADERS),
            timeout=30).close()
    except OSError as e:
        sys.exit(f"claude-mem: the LLM endpoint {LLM_URL} does not answer: {e}")

    HOME.mkdir(parents=True, exist_ok=True)
    LIVE.mkdir(parents=True, exist_ok=True)
    if not CHECKPOINT.exists() and (WORK / "checkpoint.old").exists():
        (WORK / "checkpoint.old").rename(CHECKPOINT)
    plan, skipped_files = sessions()
    first = millis(plan[0][0])
    if (CHECKPOINT / "progress.json").exists():
        progress = restore()
        log(f"resuming after {len(progress['done'])} sessions")
    else:
        DATA.mkdir(parents=True)
        progress = {
            "done": [], "sessions": {}, "counts": {}, "llm": {}, "idle": {}, "marks": {}, "rejected": [], "models": [],
            "ingest_seconds": 0, "clock": first, "log": {"counted": {}, "errors": []}}
    counts = Counter(progress["counts"])
    clock = Clock(progress["clock"])
    gateway = Gateway(progress["llm"], progress["models"])
    threading.Thread(target=gateway.serve_forever, daemon=True).start()
    worker = Worker(progress["log"]["counted"])
    worker.errors = progress["log"]["errors"]
    worker.dropped = progress["log"].get("dropped", [])
    try:
        if len(progress["done"]) < len(plan):  # a rerun of a build that finished goes straight to the store
            launches = [
                (repo, next((e["cwd"] for e in map(json.loads, path.open()) if e.get("cwd")), "/"))
                for _, repo, path in plan
            ]
            checkouts = Checkouts(checkout_roots(), launches)
            replay = Replay(
                clock, worker, gateway, checkouts, counts, Counter(progress["idle"]),
                {(k, i): n for k, i, n in progress["rejected"]})
            worker.start()
            verify_counts(progress)
            done = set(progress["done"])
            queue = [
                (start, n, None, repo, path)
                for n, (start, repo, path) in enumerate(plan)
                if f"{repo}/{path.name}" not in done
            ]
            heapq.heapify(queue)
            open_now, started, finished = 0, time.monotonic(), []
            while queue:
                worker.check()
                gateway.check()
                _, n, session, repo, path = heapq.heappop(queue)
                if session is None:
                    session, open_now = Session(repo, path, replay), open_now + 1
                following = session.following_time() if session.index >= 0 else None
                replay.horizon = min(
                    millis(queue[0][0]) if queue else float("inf"), millis(following) if following else float("inf"))
                session.step()
                if not session.ended:
                    heapq.heappush(queue, (session.next_time(), n, session, repo, path))
                    continue
                open_now -= 1
                finished.append(session)
                if open_now == 0 and queue:  # the last one is the final quiet point's
                    progress["ingest_seconds"] += time.monotonic() - started
                    if replay.quiet_point(
                        finished, progress, millis(queue[0][0])):  # one skipped keeps its sessions for the next
                        finished = []
                        progress_line(progress, len(plan), counts, gateway)
                    started = time.monotonic()
            progress["ingest_seconds"] += time.monotonic() - started
            replay.quiet_point(finished, progress, float("inf"), final=True)
    finally:
        worker.kill()
        gateway.shutdown()

    out = WORK / "store"
    copy_store(out)
    (out / "manifest.json").write_text(
        json.dumps(manifest(progress, skipped_files, plan, len(progress["done"])), indent=2) + "\n")
    log(f"done: {out}")


def progress_line(progress, total, counts, gateway):
    log(
        f"{len(progress['done'])}/{total} sessions: {sum(counts[k] for k in ('UserPromptSubmit', 'PostToolUse', 'PostToolUseFailure', 'Stop', 'SessionEnd'))} hooks, "
        f"{gateway.counts['calls']} LLM calls so far"
    )


if __name__ == "__main__":
    main()
