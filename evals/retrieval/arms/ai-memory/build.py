"""Build the ai-memory store: replay every session through ai-memory's own Claude Code hook client, hook by hook and in
one timeline across all sessions, into one ai-memory daemon on loopback, set up the way its docs set it up for the best
recall. Then save ai-memory's backup (its wiki, SQLite database and config.toml) with a manifest in /work/store.
Whenever no session is open, the backup goes to /work/checkpoint, and a run that finds it resumes from there."""

import hashlib
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
import tarfile
import threading
import time
import urllib.error
import urllib.request
import uuid
from collections import Counter
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

AGENT = "claude-code"
WORKSPACE = "default"
NOTICES = ("<local-command-stdout>", "<local-command-stderr>", "<local-command-caveat>", "<system-reminder>")
# The events install-hooks registers for Claude Code (crates/ai-memory-cli/src/commands/render_shared.rs:38-50), and
# the observation kind each is stored as (crates/ai-memory-hooks/src/payload.rs:456-497).
KINDS = {
    "session-start": "session-start", "user-prompt-submit": "user-prompt", "pre-tool-use": "pre-tool-use", "post-tool-use": "post-tool-use",
    "pre-compact": "pre-compact", "stop": "stop", "session-end": "session-end", "subagent-start": "other", "subagent-stop": "other",
}
HOOKS = tuple(KINDS)
SUBAGENT_TOOLS = ("Agent", "Task")
SYNTH_FOOTER = "_Synthesised by ai-memory (M3, no-LLM heuristic)._"
FALLBACK_WARNING = "LLM consolidation unavailable; falling back to rule-based checkpoint"
PROFILE_PASS = "profile pass updated the cross-project profile"  # logged only by a pass that wrote something
IGNORED_ENDS = ("ignoring SessionEnd naming a different scope than its session", "ignoring missing SessionEnd")
SPOOL_PACE = 1000  # ai-memory prunes a spool past 10,000 events: a replay that outruns its drain waits for it here

CORPUS = Path("/corpus")
WORK = Path("/work")
CHECKPOINT = WORK / "checkpoint"
LIVE = Path("/tmp/live")
DATA = LIVE / "data"
HOME = Path("/tmp/home")
# ai-memory keeps a session's events in the project it started in, unless it started in the server's home
# (crates/ai-memory-hooks/src/router.rs:3076-3118). One session starts in /home/noah, the only home any session starts
# in, so the daemon runs with that home, as on that developer's machine. The hooks keep HOME, where no marker file is.
SERVER_HOME = Path("/home/noah")
BIN = "/usr/local/bin/ai-memory"
SERVER = "127.0.0.1:7801"
GATEWAY = ("127.0.0.1", 7802)
EMBEDDER_URL = os.environ["EMBEDDER_URL"]
LLM_URL = os.environ["BILBO_EVAL_LLM_URL"].rstrip("/")
LLM_KEY = os.environ["BILBO_EVAL_LLM_KEY"]
LLM_MODEL = os.environ["BILBO_EVAL_LLM_MODEL"]
LLM_HEADERS = json.loads(os.environ.get("BILBO_EVAL_LLM_HEADERS") or "{}")
QUERY_PREFIX = "Instruct: Given a question about a software project's past coding sessions, retrieve the memory pages that answer it\nQuery:"

# What differs from a fresh install, each with where ai-memory 2.6.3's docs or source ask for it (paths in its repo).
# config.toml travels with the store; the URLs and the model stay in the environment.
CONFIG = {
    "capture_assistant": (True, "docs/install.md:623-656: assistant capture's server half, the stop hook's --capture-assistant is the other"),
    "consolidate_on_session_end": (True, "crates/ai-memory-cli/src/config.rs:1088, docs/usage.md:151: LLM consolidation at SessionEnd, off by default"),
    "backfill_on_start": (False, "crates/ai-memory-cli/src/config.rs:1090: imports this machine's harness history, which a replay has none of"),
    "embedding_provider": ("openai-compat", "docs/install.md tier table: hybrid retrieval, better recall on paraphrased queries"),
    "embedding_model": ("embedder", "the eval's shared Qwen3-Embedding-0.6B on llama.cpp"),
    "embedding_dim": (1024, "Qwen3-Embedding-0.6B"),
    "embedding_query_prefix": (QUERY_PREFIX, "docs/llm-providers.md:409-449: the Qwen3 recipe, an instructed query with no trailing space and plain documents"),
    "maintenance.forget_sweep_interval_secs": (0, "docs/ARCHITECTURE.md:830, crates/ai-memory-cli/src/commands/serve.rs:1941: 0 disables the forget sweep, which a default install runs daily and which evicts a session page about 80 days after its last write unless searches keep it"),
    "maintenance.embedding_backfill_interval_secs": (3600, "docs/ARCHITECTURE.md:833, docs/install.md:2457-2463: off by default only because it may call a paid provider; each tick embeds the latest pages that lack a vector, nothing else and no LLM (crates/ai-memory-consolidate/src/embed.rs:1-9, 84-130). Approved auto-improve pages are written without one (crates/ai-memory-wiki/src/wiki.rs:1710-1848), so hourly, the scheduler's cadence (docs/auto-improvement-loop.md:375); the build's embed at each quiet point stands in for the tick"),
    "auto_improve.scheduler.enabled": (False, "crates/ai-memory-cli/src/config.rs:1398-1407: one session per project an hour; each ended session is reviewed at the next quiet point instead"),
}
DAEMON_ENV = {
    "AI_MEMORY_LLM_PROVIDER": ("openai-compat", "docs/llm-providers.md"),
    "AI_MEMORY_LLM_BASE_URL": (f"http://{GATEWAY[0]}:{GATEWAY[1]}/v1", "the gateway in this file, in front of BILBO_EVAL_LLM_URL"),
    "AI_MEMORY_LLM_MODEL": (LLM_MODEL, "BILBO_EVAL_LLM_MODEL"),
    "AI_MEMORY_LLM_TIMEOUT_SECS": ("86400", "crates/ai-memory-cli/src/config.rs llm_timeout_secs: the gateway holds a call through a usage limit, past the 300 s default"),
    "AI_MEMORY_EMBEDDING_BASE_URL": (EMBEDDER_URL, "the shared embedder"),
}
HOOK_ENV = {
    "AI_MEMORY_HOOK_DRAIN_TIMEOUT_MINUTES": ("60", "docs/install.md:702-724, the maximum: a drain posts the PreCompact whose LLM call runs in the request"),
    "AI_MEMORY_HOOK_HANDOFF_TIMEOUT_MINUTES": ("60", "docs/install.md:702-724, the maximum"),
    "AI_MEMORY_HOOK_START_BUDGET_MINUTES": ("60", "docs/install.md:702-724, the maximum"),
    "AI_MEMORY_HOOK_BACKGROUND_DRAIN_BUDGET_MINUTES": ("60", "docs/install.md:702-724, the maximum"),
    "AI_MEMORY_HOOK_SPOOL_MAX_ATTEMPTS": ("0", "docs/install.md:693-699: never drop an event after failed drain passes"),
}
# What else the manifest should say about the setup.
NOTES = {
    "server_home": f"the daemon runs with HOME={SERVER_HOME}, the only home a session starts in, since ai-memory keeps a session's events in its project unless it started in the server's home (crates/ai-memory-hooks/src/router.rs:3076-3118); the hooks run with HOME={HOME}",
    "experience_pass": "off, the default: it runs only in the auto-improve scheduler and counts sessions that ended after the scope was first seen by the wall clock, which no replayed session did (docs/experience.md; crates/ai-memory-store/src/auto_improve.rs:559-589)",
    "reranker": "AI_MEMORY_RERANKER=llm (docs/install.md:1899) reranks at query time with one LLM call per query: the run step's decision",
    "embedding_backfill": "the build runs `ai-memory embed` per project at each quiet point where latest pages still lack a vector, standing in for the hourly backfill in config.toml as the manual reviews stand in for the scheduler; pages.embedded_by_build counts them. A daemon start also backfills once (crates/ai-memory-cli/src/commands/serve.rs:2110-2139)",
    "llm": "the calls behind this store: the calls of work an interrupt threw away are not counted",
}
BASE_ENV = {"HOME": str(HOME), "PATH": "/usr/local/bin:/usr/bin:/bin", "NO_COLOR": "1"}


def log(msg):
    print(f"ai-memory: {msg}", file=sys.stderr, flush=True)


def stamp(entry):
    return datetime.fromisoformat(entry["timestamp"]) if entry.get("timestamp") else None


def sessions():
    """Every Claude Code session as (start, repo, path), in the order they started across all repos. A transcript
    with no timestamp is not Claude Code's and is left out; the hooks never see it."""
    found, other = [], []
    for path in sorted(CORPUS.glob("*/*.jsonl")):
        start = next((t for t in map(stamp, map(json.loads, path.open())) if t), None)
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


def config_toml():
    top, sections = [], {}
    for key, (value, _) in CONFIG.items():
        *section, name = key.split(".")
        (sections.setdefault(".".join(section), []) if section else top).append(f"{name} = {json.dumps(value)}")
    return "\n".join(top + [f"\n[{name}]\n" + "\n".join(lines) for name, lines in sections.items()]) + "\n"


def retry_after(headers):
    try:
        return int(float(headers.get("retry-after", 0)))
    except ValueError:  # an HTTP date: back off as usual
        return 0


def kind_of(body):
    """Which of ai-memory's jobs made an LLM call, from its system prompt."""
    try:
        content = next(m["content"] for m in json.loads(body)["messages"] if m["role"] in ("system", "developer"))
    except (ValueError, KeyError, StopIteration, TypeError):
        return "other", ""
    head = (content if isinstance(content, str) else text_of(content))[:80]
    for kind, marker in (("consolidation", "Karpathy-style"), ("auto_improve", "auto-improvement"), ("profile", "cross-project profile")):
        if marker in head:
            return kind, head
    return "other", head


def fetch(request):
    """The upstream's answer, an error status included."""
    try:
        with urllib.request.urlopen(request, timeout=3600) as r:
            return r.status, r.headers, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.headers, e.read()


class Gateway(ThreadingHTTPServer):
    """ai-memory's LLM calls go through here to BILBO_EVAL_LLM_URL. ai-memory tries a call 3 times 2 s apart and ignores
    Retry-After, then fails the consolidation or falls back to a rule-based page; here a failed call is sent again the
    way mem0's build retries its add(), so ai-memory only sees a slow provider. Every call to the LLM is counted."""

    daemon_threads = True

    def __init__(self, counts):
        super().__init__(GATEWAY, Forward)
        self.counts, self.lock, self.fatal = Counter(counts), threading.Lock(), threading.Event()
        self.in_flight, self.active = 0, time.monotonic()
        self.calls = (WORK / "llm-calls.jsonl").open("a")

    def send(self, method, path, headers, body):
        url = LLM_URL + path.removeprefix("/v1")
        skip = ("host", "content-length", "transfer-encoding", "authorization", "connection", "accept-encoding")
        headers = {k: v for k, v in headers.items() if k.lower() not in skip} | {"Authorization": f"Bearer {LLM_KEY}"} | LLM_HEADERS
        kind, system = kind_of(body) if body else ("other", "")
        for attempt in range(1, 11):
            if self.fatal.is_set():
                break
            started, error = time.monotonic(), None
            try:
                status, answer, data = fetch(urllib.request.Request(url, body, headers, method=method))
            except (OSError, http.client.HTTPException) as e:  # refused, reset, timed out or cut short
                status, answer, data, error = None, {}, b"", e
            self.record(kind, system, attempt, status, time.monotonic() - started, data)
            if status is not None and status != 429 and status < 500:
                return status, answer, data
            wait = min(60 * attempt, 600)
            if status == 429:
                wait = max(wait, retry_after(answer))
            log(f"LLM call failed ({status or error}); retrying in {wait} s")
            with self.lock:
                self.counts["retries"] += 1
            time.sleep(wait)
        self.fatal.set()
        return 503, {}, b""

    def record(self, kind, system, attempt, status, seconds, data):
        usage, empty = {}, False
        if status == 200 and data:
            try:
                answer = json.loads(data)
                usage = answer.get("usage") or {}
                empty = not ((answer.get("choices") or [{}])[0].get("message") or {}).get("content")
            except ValueError:
                pass
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
            line = {"time": datetime.now(timezone.utc).isoformat(timespec="seconds"), "kind": kind, "status": status, "attempt": attempt,
                    "ms": round(seconds * 1000), "prompt_tokens": usage.get("prompt_tokens"), "completion_tokens": usage.get("completion_tokens"),
                    "empty": empty, "system": system}
            self.calls.write(json.dumps(line) + "\n")
            self.calls.flush()

    def check(self):
        if self.fatal.is_set():
            sys.exit("ai-memory: the LLM failed 10 times in a row; run again to resume from the last checkpoint")


class Forward(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_GET(self):
        self.forward(None)

    def do_POST(self):
        self.forward(self.rfile.read(int(self.headers.get("Content-Length") or 0)))

    def forward(self, body):
        gateway = self.server
        with gateway.lock:
            gateway.in_flight += 1
            gateway.active = time.monotonic()
        try:
            status, headers, data = gateway.send(self.command, self.path, self.headers, body)
        finally:
            with gateway.lock:
                gateway.in_flight -= 1
                gateway.active = time.monotonic()
        self.send_response(status)
        self.send_header("Content-Type", headers.get("Content-Type") or "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        pass


class Checkouts:
    """The sessions' directories, under the paths they had. Each repository's main checkout holds a minimal git
    repository with its GitHub remote, which is all ai-memory's hook reads to name the project; the worktrees and
    subdirectories the sessions ran in sit inside it, as they did on the developer's machine."""

    def __init__(self, roots):
        self.made = set()
        for repo, root in roots.items():
            git = Path(root) / ".git"
            (git / "objects").mkdir(parents=True, exist_ok=True)
            (git / "refs" / "heads").mkdir(parents=True, exist_ok=True)
            (git / "HEAD").write_text("ref: refs/heads/main\n")
            remote = f"https://github.com/{repo.replace('__', '/', 1)}.git"
            (git / "config").write_text(f'[core]\n\trepositoryformatversion = 0\n\tbare = false\n[remote "origin"]\n\turl = {remote}\n')

    def ensure(self, cwd):
        if cwd not in self.made:
            Path(cwd).mkdir(parents=True, exist_ok=True)
            self.made.add(cwd)


class Daemon:
    """`ai-memory serve` on loopback in this container, with the CLI that talks to it."""

    def __init__(self):
        self.env = BASE_ENV | {"LLM_API_KEY": "local", "AI_MEMORY_SERVER_URL": f"http://{SERVER}"} | {k: v for k, (v, _) in DAEMON_ENV.items()}
        self.hook_env = BASE_ENV | {k: v for k, (v, _) in HOOK_ENV.items()}
        self.process, self.started, self.read = None, None, 0

    def start(self):
        (DATA / "config.toml").write_text(config_toml())
        server_log = WORK / "server.log"
        self.read = server_log.stat().st_size if server_log.exists() else 0  # earlier runs' lines are not this store's
        self.process = subprocess.Popen([BIN, "--data-dir", str(DATA), "serve", "--transport", "http", "--bind", SERVER], env=self.env | {"HOME": str(SERVER_HOME)},
                                        cwd=HOME, stdin=subprocess.DEVNULL, stdout=server_log.open("a"), stderr=subprocess.STDOUT)
        self.started = time.monotonic()
        for _ in range(600):
            self.check()
            try:
                urllib.request.urlopen(f"http://{SERVER}/healthz", timeout=2).close()
                return
            except OSError:
                time.sleep(0.5)
        sys.exit("ai-memory: the daemon did not answer /healthz in 5 minutes")

    def check(self):
        if self.process.poll() is not None:
            sys.exit(f"ai-memory: the daemon exited with {self.process.returncode}; see {WORK / 'server.log'}, then run again to resume from the last checkpoint")

    def stop(self):
        if self.process and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(60)
            except subprocess.TimeoutExpired:
                self.process.kill()

    def logged(self):
        """The server log's whole lines written since the last call."""
        with (WORK / "server.log").open("rb") as f:
            f.seek(self.read)
            data = f.read()
        data = data[: data.rfind(b"\n") + 1]
        self.read += len(data)
        return data.decode(errors="replace").splitlines()

    def cli(self, *args, env=None):
        r = subprocess.run([BIN, "--data-dir", str(DATA), *args], env=env or self.env, cwd=HOME, stdin=subprocess.DEVNULL, capture_output=True, text=True)
        if r.returncode:
            sys.exit(f"ai-memory: `ai-memory {args[0]}` exited with {r.returncode}: {r.stderr.strip()[-2000:]}")
        return r.stdout

    def status(self):
        return json.loads(self.cli("status", "--json"))

    def hook(self, slug, body):
        """One event through the native hook, as the command install-hooks registers runs it, reading Claude Code's
        JSON on stdin. It spools the event; stop, pre-compact and session-end also start ai-memory's background
        drain, and a growing backlog makes post-tool-use drain too."""
        args = [BIN, "--data-dir", str(DATA), "hook", "--event", slug, "--agent", AGENT, "--server-url", f"http://{SERVER}"]
        if slug == "stop":
            args.append("--capture-assistant")  # install-hooks --capture-assistant bakes it on the stop command only
        payload = json.dumps(body, ensure_ascii=False, separators=(",", ":")).encode()
        r = subprocess.run(args, input=payload, env=self.hook_env, cwd=body["cwd"], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        lost = ("ai-memory hook warning", "hook-spool at capacity")
        warning = next((line for line in r.stderr.decode(errors="replace").splitlines() if any(x in line for x in lost)), None)
        if r.returncode or warning:
            sys.exit(f"ai-memory: the {slug} hook did not spool its event ({warning or f'exit {r.returncode}'})")

    def spooled(self):
        spool = DATA / "hook-spool"
        return sum(1 for f in os.scandir(spool) if f.name.endswith(".json")) if spool.exists() else 0


def text_of(content):
    if isinstance(content, str):
        return content.strip()
    return "\n".join(b.get("text", "") for b in content or [] if isinstance(b, dict) and b.get("type") == "text").strip()


def slash_command(text):
    """A slash command as the user typed it, from Claude Code's <command-name> record."""
    name = re.search(r"<command-name>(.*?)</command-name>", text, re.DOTALL)
    args = re.search(r"<command-args>(.*?)</command-args>", text, re.DOTALL)
    return " ".join(x for x in (name.group(1).strip() if name else "", args.group(1).strip() if args else "") if x)


def iso(when):
    return when.astimezone(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")


def tool_results(content):
    return [b for b in content if isinstance(b, dict) and b.get("type") == "tool_result"] if isinstance(content, list) else []


class Session:
    """One session through ai-memory's hook client, called as Claude Code calls the hooks install-hooks registers:
    SessionStart, again after a compaction; UserPromptSubmit for each prompt typed; PreToolUse and PostToolUse around
    each tool call, with SubagentStart and SubagentStop around a subagent; Stop when a reply ends; PreCompact before a
    compaction; SessionEnd at the end. Each event carries the time it happened as occurred_at, and each step is one
    event, so sessions open at the same time interleave by time."""

    def __init__(self, repo, path, daemon, gateway, checkouts, counts):
        self.name, self.daemon, self.gateway, self.checkouts, self.counts = f"{repo}/{path.name}", daemon, gateway, checkouts, counts
        self.entries = [json.loads(line) for line in path.read_text().splitlines() if line.strip()]
        self.times, when = [], None
        for entry in self.entries:
            when = stamp(entry) or when
            self.times.append(when)
        first_time = next(t for t in self.times if t)
        self.times = [t or first_time for t in self.times]
        self.sid = next(e["sessionId"] for e in self.entries if e.get("sessionId"))
        self.cwd = next((e["cwd"] for e in self.entries if e.get("cwd")), "/")  # the first entries often carry none
        home = "/".join(self.cwd.split("/")[:3])
        self.transcript = f"{home}/.claude/projects/{re.sub('[^A-Za-z0-9]', '-', self.cwd)}/{self.sid}.jsonl"
        # A subagent's id and last reply arrive with its result; SubagentStart already knows the id.
        self.subagents = {}
        for entry in self.entries:
            result = entry.get("toolUseResult")
            if isinstance(result, dict) and result.get("agentId"):
                for b in tool_results((entry.get("message") or {}).get("content")):
                    self.subagents[b.get("tool_use_id")] = (result["agentId"], text_of(result.get("content")))
        self.turn, self.tools, self.index, self.ended = None, {}, -1, False
        self.prompt_id, self.mode = None, "default"

    def due(self):
        return self.times[min(max(self.index, 0), len(self.entries) - 1)]

    def next_time(self):
        return self.due()

    def send(self, slug, event, when, turn=False, **fields):
        body = {"session_id": self.sid, "transcript_path": self.transcript, "cwd": self.cwd}
        if turn and self.prompt_id:
            body |= {"prompt_id": self.prompt_id, "permission_mode": self.mode}
        body |= {"hook_event_name": event, **fields, "occurred_at": iso(when)}
        self.checkouts.ensure(self.cwd)
        started = time.monotonic()
        self.daemon.hook(slug, body)
        self.counts[slug] += 1
        self.counts["hook_seconds"] += time.monotonic() - started
        if self.daemon.spooled() >= SPOOL_PACE:
            self.counts["spool_paced"] += 1
            drain_all(self.daemon, self.gateway)

    def prompt(self, text, when):
        self.send("user-prompt-submit", "UserPromptSubmit", when, turn=True, prompt=text)

    def stop(self, when):
        turn = self.turn
        if not turn or turn["stopped"] or not turn["reply"] or turn["interrupted"]:
            return
        turn["stopped"] = True
        self.send("stop", "Stop", when, turn=True, stop_hook_active=False, last_assistant_message=turn["reply"], background_tasks=[], session_crons=[])

    def turn_goes_on(self):
        """Whether the model goes on after this compaction before anything is typed: an automatic compaction can come
        in the middle of a turn, whose Stop Claude Code fires only when the turn ends."""
        for entry in self.entries[self.index + 1:]:
            if entry.get("type") not in ("user", "assistant") or entry.get("isSidechain") or entry.get("isMeta") or entry.get("isCompactSummary"):
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
            self.send("session-start", "SessionStart", self.times[0], source="startup")
        elif self.index < len(self.entries):
            self.event(self.entries[self.index], self.times[self.index])
            self.index += 1
        else:
            self.stop(self.times[-1])
            self.send("session-end", "SessionEnd", self.times[-1], reason="other")
            self.ended = True

    def event(self, entry, when):
        kind, subtype, sidechain = entry.get("type"), entry.get("subtype"), entry.get("isSidechain")
        content = (entry.get("message") or {}).get("content")
        blocks = content if isinstance(content, list) else []
        results = tool_results(content)
        if sidechain:
            return
        typed = kind == "user" and not (results or entry.get("isMeta") or entry.get("isCompactSummary"))
        text = text_of(content) if typed else ""
        new_turn = text and not text.startswith((*NOTICES, "[Request interrupted"))
        if new_turn:
            self.stop(when)  # the turn before ends where it ran
        self.cwd = entry.get("cwd") or self.cwd  # Claude Code sends the cwd the session is in at each hook

        if kind == "system" and subtype in ("stop_hook_summary", "turn_duration"):
            self.stop(when)
        elif kind == "system" and subtype == "compact_boundary":
            trigger = (entry.get("compactMetadata") or {}).get("trigger") or "auto"
            if trigger == "manual" or not self.turn_goes_on():  # a typed /compact comes between turns
                self.stop(when)
            self.send("pre-compact", "PreCompact", when, trigger=trigger, custom_instructions="")
            self.send("session-start", "SessionStart", when, source="compact")
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
                self.prompt(text, when)
        elif kind == "assistant":
            if self.turn and self.turn["prompt"]:
                self.prompt(self.turn["prompt"], when)
                self.turn["prompt"] = None
            for b in blocks:
                if isinstance(b, dict) and b.get("type") == "tool_use":
                    name, tool_input = b.get("name"), b.get("input") or {}
                    self.tools[b.get("id")] = (name, tool_input, when)
                    self.send("pre-tool-use", "PreToolUse", when, turn=True, tool_name=name, tool_input=tool_input, tool_use_id=b.get("id"))
                    if name in SUBAGENT_TOOLS:
                        agent_id, _ = self.subagents.get(b.get("id"), (b.get("id"), ""))
                        self.send("subagent-start", "SubagentStart", when, turn=True, agent_id=agent_id, agent_type=tool_input.get("subagent_type") or "general-purpose")
            reply = text_of(content)
            api_error = bool(entry.get("isApiErrorMessage"))
            if reply and (api_error or (entry.get("message") or {}).get("model") == "<synthetic>"):
                # Claude Code's own text (an API error, a usage limit, "No response requested."), not the model's: never a
                # reply. A turn an API error ends fires StopFailure, which ai-memory does not install, instead of Stop.
                self.counts["skipped_api_error_replies" if api_error else "skipped_synthetic_replies"] += 1
                if self.turn and api_error:
                    self.turn["interrupted"] = True
            elif self.turn and reply:
                # Text after a Stop means Claude went on (a Stop hook blocked it, a wakeup fired): Stop fires again.
                self.turn["reply"], self.turn["interrupted"], self.turn["stopped"] = reply, False, False
        elif kind == "user" and results:
            # A failed tool fires PostToolUseFailure, which ai-memory does not install for Claude Code.
            for b in (b for b in results if not b.get("is_error")):
                name, tool_input, called = self.tools.get(b.get("tool_use_id"), ("unknown", {}, when))
                if name in SUBAGENT_TOOLS:
                    agent_id, reply = self.subagents.get(b.get("tool_use_id"), (b.get("tool_use_id"), ""))
                    self.send("subagent-stop", "SubagentStop", when, turn=True, agent_id=agent_id, agent_type=tool_input.get("subagent_type") or "general-purpose",
                              agent_transcript_path=f"{self.transcript.removesuffix('.jsonl')}/subagents/agent-{agent_id}.jsonl",
                              stop_hook_active=False, last_assistant_message=reply, background_tasks=[], session_crons=[])
                response = entry.get("toolUseResult")
                self.send("post-tool-use", "PostToolUse", when, turn=True, tool_name=name, tool_input=tool_input, tool_use_id=b.get("tool_use_id"),
                          tool_response=text_of(b.get("content")) if response is None else response,
                          duration_ms=max(0, round((when - called).total_seconds() * 1000)))


def query(sql, *args, path=DATA / "db" / "memory.sqlite", immutable=False):
    """Read ai-memory's database without writing to it: the live one beside the daemon, or the saved one."""
    c = sqlite3.connect(f"file:{path}?mode=ro{'&immutable=1' if immutable else ''}", uri=True)
    try:
        return c.execute(sql, args).fetchall()
    finally:
        c.close()


def drain_all(daemon, gateway):
    """Deliver every spooled event. A drain holds the lock through a PreCompact's LLM call, which the gateway holds
    through a usage limit, so only the time the LLM is idle counts toward the two hours a stuck spool gets."""
    stuck, told = 0, time.monotonic()
    while daemon.spooled():
        daemon.check()
        gateway.check()
        if stuck > 7200:
            drain_log = DATA / "logs" / "hook-drain.log"
            if drain_log.exists():
                shutil.copy(drain_log, WORK / drain_log.name)
            sys.exit(f"ai-memory: {daemon.spooled()} hook events stayed in the spool for two hours; see {WORK / drain_log.name}")
        started = time.monotonic()
        daemon.cli("hook-drain", env=daemon.hook_env)
        if daemon.spooled():
            time.sleep(2)
        if not gateway.in_flight and gateway.active < started:
            stuck += time.monotonic() - started
        elif time.monotonic() - told > 300:
            log(f"{daemon.spooled()} hook events wait in the spool for the LLM")
            told = time.monotonic()


def wait_queue(daemon, gateway):
    """Wait for ai-memory's SessionEnd consolidation queue to empty. A failed job means a session kept only its
    rule-based page, which the gateway exists to prevent."""
    calm = 0
    while calm < 3:
        daemon.check()
        gateway.check()
        states = dict(query("SELECT state, COUNT(*) FROM session_consolidation_jobs GROUP BY state"))
        failed = query("SELECT lower(hex(session_id)), last_error FROM session_consolidation_jobs WHERE state = 'failed'")
        if failed:
            sys.exit(f"ai-memory: SessionEnd consolidation failed for {failed}")
        calm = calm + 1 if not states.get("pending") and not states.get("running") else 0
        time.sleep(2)


def auto_improve(daemon, progress):
    """ai-memory's scheduler reviews each completed session once, at most one per project an hour. Each session that
    has ended is reviewed here instead, once, in the order the sessions ended, with the same settings."""
    ended = query("SELECT s.id, p.name FROM sessions s JOIN projects p ON p.id = s.project_id WHERE s.ended_at IS NOT NULL ORDER BY s.ended_at")
    for sid, project in ended:
        sid = str(uuid.UUID(bytes=sid))
        if sid in progress["auto_improve"]:
            continue
        report = json.loads(daemon.cli("auto-improve", "--session-id", sid, "--workspace", WORKSPACE, "--project", project, "--json"))
        progress["auto_improve"][sid] = {
            "project": project,
            "skipped_preflight": report["summary"] == "session skipped by preflight filters",
            "proposals": dict(Counter(p["status"] for p in report["proposals"])),
            "skipped": len(report["skipped"]),
            "rejected": report["rejected_candidates_count"],
            "warnings": report["warnings"],
        }


def missing_embeddings(daemon):
    """Wait until every latest page has its embedding, or the count stops moving for 10 s."""
    seen = [daemon.status()["derived"]["latest_pages_missing_embeddings"]]
    while seen[-1] and seen[-3:] != [seen[-1]] * 3:
        time.sleep(5)
        seen.append(daemon.status()["derived"]["latest_pages_missing_embeddings"])
    return seen[-1]


def wait_embeddings(daemon, counts):
    """Wait for new pages to be embedded; embed any left over, project by project, as the hourly backfill set in
    config.toml would. Returns how many still have none."""
    missing = missing_embeddings(daemon)
    if not missing:
        return 0
    for (name,) in query("SELECT name FROM projects"):
        daemon.cli("embed", "--workspace", WORKSPACE, "--project", name)
    left = missing_embeddings(daemon)
    counts["embedded_by_build"] += max(0, missing - left)
    return left


def observation_counts(names, progress):
    sql = "SELECT COUNT(*) FROM observations WHERE session_id = ?"
    return {name: query(sql, uuid.UUID(progress["sessions"][name]["sid"]).bytes)[0][0] for name in names}


def field(line, name):
    found = re.search(rf"\b{name}=(\S+)", line)
    return found.group(1) if found else None


def tally(lines, found):
    """What only the server log tells: rule-based fallbacks, profile passes and the SessionEnds ai-memory ignored."""
    for line in lines:
        if FALLBACK_WARNING in line:
            found["fallback_warnings"] += 1
        elif PROFILE_PASS in line:
            found["profile_passes"] += 1
            found["profile_llm_fallbacks"] += int(field(line, "llm_fallbacks") or 0)
        elif any(x in line for x in IGNORED_ENDS):
            found["ignored_ends"].append(field(line.split("SessionEnd", 1)[1], "session"))


def save(daemon, gateway, progress):
    """Write the store to the checkpoint: the swap is two renames, and a run that finds only checkpoint.old uses it."""
    with gateway.lock:
        progress["llm"] = dict(gateway.counts)
    ingest = {k: v for k, v in (daemon.status().get("ingest") or {}).items() if k != "last_persisted_ms"}
    progress["ingest"] = {k: progress["ingest_before"].get(k, 0) + v for k, v in ingest.items()}
    tally(daemon.logged(), progress["log"])
    for name, n in observation_counts([n for n, s in progress["sessions"].items() if "observations" not in s], progress).items():
        progress["sessions"][name]["observations"] = n
    tmp, old = WORK / "checkpoint.tmp", WORK / "checkpoint.old"
    shutil.rmtree(tmp, ignore_errors=True)
    tmp.mkdir(parents=True)
    daemon.cli("backup", "--to", str(tmp / "backup.tar.gz"))
    (tmp / "progress.json").write_text(json.dumps(progress, indent=2) + "\n")
    shutil.rmtree(old, ignore_errors=True)
    if CHECKPOINT.exists():
        CHECKPOINT.rename(old)
    tmp.rename(CHECKPOINT)
    shutil.rmtree(old, ignore_errors=True)


def restore(daemon):
    DATA.mkdir(parents=True)
    daemon.cli("restore", "--from", str(CHECKPOINT / "backup.tar.gz"), "--force")
    progress = json.loads((CHECKPOINT / "progress.json").read_text())
    progress["ingest_before"] = progress["ingest"]  # the daemon's counters start again from zero
    return progress


def verify_counts(progress):
    """A resumed store must hold exactly what the checkpoint recorded for each finished session: nothing re-sent."""
    recorded = {name: s["observations"] for name, s in progress["sessions"].items()}
    found = observation_counts(recorded, progress)
    if found != recorded:
        sys.exit(f"ai-memory: the restored store does not match its checkpoint: {[n for n in recorded if found[n] != recorded[n]]}")


def settle(daemon, gateway, last_end):
    """ai-memory's cross-project profile pass runs 30 s after a session ends, and 2 minutes after the daemon starts at
    the earliest, with nothing to show it is coming: wait for its time, then for the LLM to go quiet for a minute."""
    time.sleep(max(0, daemon.started + 150 - time.monotonic(), last_end + 45 - time.monotonic()))
    while gateway.in_flight or time.monotonic() - gateway.active < 60:
        daemon.check()
        gateway.check()
        time.sleep(1)


def manifest(progress, skipped, status, missing):
    inputs = (WORK / "key.txt").read_text().splitlines()
    saved = {"path": WORK / "store" / "db" / "memory.sqlite", "immutable": True}
    rows = {str(uuid.UUID(bytes=sid)): (sid, name, ended) for sid, name, ended in query("SELECT s.id, p.name, s.ended_at FROM sessions s JOIN projects p ON p.id = s.project_id", **saved)}
    observations = dict(query("SELECT kind, COUNT(*) FROM observations GROUP BY kind ORDER BY kind", **saved))
    sent = Counter()
    for slug, kind in KINDS.items():
        sent[kind] += progress["counts"].get(slug, 0)
    latest = query("SELECT path, body, frontmatter_json FROM pages WHERE is_latest = 1", **saved)
    session_pages = {path: (body, json.loads(meta or "{}")) for path, body, meta in latest if path.startswith("sessions/")}
    paged = sum(1 for s in progress["sessions"].values() if f"sessions/{s['sid']}.md" in session_pages)
    # Every session's SessionEnd was sent: a session still open had it ignored, and one that ended without a page was
    # too short for ai-memory to write one.
    last_project = "SELECT p.name FROM observations o JOIN projects p ON p.id = o.project_id WHERE o.session_id = ? ORDER BY o.created_at DESC, o.rowid DESC LIMIT 1"
    ignored, ephemeral = [], []
    for name, s in progress["sessions"].items():
        sid, project, ended = rows.get(s["sid"], (None, "?", None))
        if ended is None:
            last = query(last_project, sid, **saved) if sid else []
            ignored.append({"session": name, "project": project, "last_observation_project": last[0][0] if last else None, "logged": s["sid"] in progress["log"]["ignored_ends"]})
        elif f"sessions/{s['sid']}.md" not in session_pages:
            ephemeral.append(name)
    projects = {}
    for name, s in progress["sessions"].items():
        projects.setdefault(name.split("/")[0], Counter())[rows.get(s["sid"], (None, "?"))[1]] += 1
    reviews = progress["auto_improve"]
    settings = [{"key": k, "value": v, "cite": cite} for k, (v, cite) in (CONFIG | DAEMON_ENV | HOOK_ENV).items() if k != "AI_MEMORY_LLM_MODEL"]
    settings.append({"key": "hook --capture-assistant", "value": "on the stop hook", "cite": "docs/install.md:623-656: install-hooks --capture-assistant"})
    return {
        "arm": "ai-memory",
        "key": inputs[0],
        "inputs": inputs[1:],
        "device": os.environ["BILBO_EVAL_DEVICE"],
        "versions": {
            "ai-memory": subprocess.run([BIN, "--version"], env=BASE_ENV, capture_output=True, text=True).stdout.split()[-1],
            "binary_sha256": hashlib.sha256(Path(BIN).read_bytes()).hexdigest(),
            "arch": platform.machine(),
            "python": platform.python_version(),
        },
        "config": settings,
        "embedder": {"provider": "openai-compat", "model": "Qwen3-Embedding-0.6B Q8_0", "dim": 1024, "query_prefix": QUERY_PREFIX},
        "notes": NOTES,
        "time": {"sessions_and_observations": "occurred_at (session time)", "pages": "wall clock at build"},
        "projects": {repo: dict(found) for repo, found in sorted(projects.items())},
        "sessions": {"replayed": len(progress["done"]), "skipped": skipped, "with_page": paged, "ignored_end": ignored, "ephemeral": ephemeral},
        "events": {k: progress["counts"].get(k, 0) for k in HOOKS},
        "replay": {
            "hook_seconds": round(progress["counts"].get("hook_seconds", 0)),
            "spool_paced": progress["counts"].get("spool_paced", 0),
            "skipped_replies": {"api_error": progress["counts"].get("skipped_api_error_replies", 0), "synthetic": progress["counts"].get("skipped_synthetic_replies", 0)},
        },
        "ingest": progress["ingest"],
        "observations": {"total": sum(observations.values()), "by_kind": observations, "unstored": {k: n - observations.get(k, 0) for k, n in sorted(sent.items())}},
        "pages": {
            "latest": len(latest),
            "all": query("SELECT COUNT(*) FROM pages", **saved)[0][0],
            "by_folder": dict(sorted(Counter(path.split("/")[0] for path, _, _ in latest).items())),
            "missing_embeddings": missing,
            "unembeddable": status["derived"].get("latest_pages_unembeddable", 0),
            "embedded_by_build": progress["counts"].get("embedded_by_build", 0),
        },
        "consolidation": {
            "jobs": dict(query("SELECT state, COUNT(*) FROM session_consolidation_jobs GROUP BY state", **saved)),
            "fallback_warnings": progress["log"]["fallback_warnings"],
            "rule_based_latest_session_pages": sum(body.rstrip().endswith(SYNTH_FOOTER) for body, _ in session_pages.values()),
            "consolidated_latest_session_pages": sum(meta.get("consolidated") is True for _, meta in session_pages.values()),
        },
        "auto_improve": {
            "reviewed": sum(not r["skipped_preflight"] for r in reviews.values()),
            "skipped_preflight": sum(r["skipped_preflight"] for r in reviews.values()),
            "proposals": dict(sum((Counter(r["proposals"]) for r in reviews.values()), Counter())),
            "sessions": reviews,
        },
        "profile": {"llm_calls": progress["llm"].get("calls_profile", 0), "passes_that_wrote": progress["log"]["profile_passes"], "llm_fallbacks": progress["log"]["profile_llm_fallbacks"]},
        "llm": {"model": LLM_MODEL} | {k: round(v) if isinstance(v, float) else v for k, v in sorted(progress["llm"].items())},
        "ingest_seconds": round(progress["ingest_seconds"]),
        "finished": datetime.now(timezone.utc).isoformat(timespec="seconds"),
    }


def interrupted(signum, frame):
    sys.exit("ai-memory: stopped; run again to resume from the last checkpoint")


def main():
    signal.signal(signal.SIGINT, interrupted)
    signal.signal(signal.SIGTERM, interrupted)
    try:
        urllib.request.urlopen(urllib.request.Request(f"{LLM_URL}/models", headers={"Authorization": f"Bearer {LLM_KEY}"} | LLM_HEADERS), timeout=30).close()
    except OSError as e:
        sys.exit(f"ai-memory: the LLM endpoint {LLM_URL} does not answer: {e}")

    HOME.mkdir(parents=True, exist_ok=True)
    SERVER_HOME.mkdir(parents=True, exist_ok=True)
    if not CHECKPOINT.exists() and (WORK / "checkpoint.old").exists():
        (WORK / "checkpoint.old").rename(CHECKPOINT)
    daemon = Daemon()
    if (CHECKPOINT / "progress.json").exists():
        progress = restore(daemon)
        log(f"resuming after {len(progress['done'])} sessions")
    else:
        DATA.mkdir(parents=True)
        progress = {"done": [], "sessions": {}, "counts": {}, "llm": {}, "ingest": {}, "ingest_before": {}, "auto_improve": {}, "ingest_seconds": 0,
                    "log": {"fallback_warnings": 0, "profile_passes": 0, "profile_llm_fallbacks": 0, "ignored_ends": []}}
    counts = Counter(progress["counts"])
    gateway = Gateway(progress["llm"])
    threading.Thread(target=gateway.serve_forever, daemon=True).start()
    try:
        daemon.start()
        verify_counts(progress)
        checkouts = Checkouts(checkout_roots())
        plan, skipped = sessions()
        done = set(progress["done"])
        queue = [(start, n, None, repo, path) for n, (start, repo, path) in enumerate(plan) if f"{repo}/{path.name}" not in done]
        heapq.heapify(queue)
        open_now, started, last_end = 0, time.monotonic(), time.monotonic()
        while queue:
            daemon.check()
            gateway.check()
            _, n, session, repo, path = heapq.heappop(queue)
            if session is None:
                session, open_now = Session(repo, path, daemon, gateway, checkouts, counts), open_now + 1
            session.step()
            if not session.ended:
                heapq.heappush(queue, (session.next_time(), n, session, repo, path))
                continue
            open_now, last_end = open_now - 1, time.monotonic()
            progress["done"].append(session.name)
            progress["sessions"][session.name] = {"sid": session.sid}
            if open_now == 0:
                drain_all(daemon, gateway)
                wait_queue(daemon, gateway)
                auto_improve(daemon, progress)
                wait_embeddings(daemon, counts)
                gateway.check()
                progress["counts"] = dict(counts)
                progress["ingest_seconds"] += time.monotonic() - started
                save(daemon, gateway, progress)
                started = time.monotonic()
                log(f"{len(progress['done'])}/{len(plan)} sessions: {sum(counts[k] for k in HOOKS)} hook events, {gateway.counts['calls']} LLM calls so far")

        settle(daemon, gateway, last_end)
        drain_all(daemon, gateway)
        wait_queue(daemon, gateway)
        missing = wait_embeddings(daemon, counts)
        gateway.check()
        progress["counts"] = dict(counts)
        status = daemon.status()
        save(daemon, gateway, progress)
    finally:
        daemon.stop()
        gateway.shutdown()

    out = WORK / "store"
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir()
    with tarfile.open(CHECKPOINT / "backup.tar.gz") as t:
        t.extractall(out, filter="data")
    (out / "manifest.json").write_text(json.dumps(manifest(progress, skipped, status, missing), indent=2) + "\n")
    # A store that lost events or vectors is not packed: it would be compared with the other arms as it stands.
    lost = [f"{k} {progress['ingest'][k]}" for k in ("failed", "dropped_invalid", "dropped_unauthorized") if progress["ingest"].get(k)]
    if missing:
        lost.append(f"{missing} latest pages without an embedding")
    if lost:
        sys.exit(f"ai-memory: the store lost events or vectors ({', '.join(lost)}); see {out / 'manifest.json'}")
    log(f"done: {out}")


if __name__ == "__main__":
    main()
