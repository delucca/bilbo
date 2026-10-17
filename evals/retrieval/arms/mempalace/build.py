"""Build the MemPalace store: replay every Claude Code session through MemPalace's own Claude Code plugin hooks, hook
by hook and in one timeline across all sessions, into one palace, each hook run at the time its event happened. Then
save the palace directory with the files MemPalace writes beside it and a manifest in /work/store. Whenever no session
is open, the same files go to /work/checkpoint, and a run that finds them resumes from there."""

import heapq
import http.client
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import threading
import time
import urllib.request
from collections import Counter
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from importlib.metadata import version
from pathlib import Path
from urllib.parse import urlparse

NOTICES = ("<local-command-stdout>", "<local-command-stderr>", "<local-command-caveat>", "<system-reminder>")
# The hooks the marketplace plugin registers (.claude-plugin/hooks/hooks.json), as `mempalace hook run --hook` names
# them, and the Claude Code event each stands for.
HOOKS = {"stop": "Stop", "precompact": "PreCompact", "session-end": "SessionEnd"}
API_MODEL = "embedder"
EF_NAME = f"openai_compat_emb_{API_MODEL}"  # mempalace/embedding.py:632-636
UPSERT_BATCH, EF_BATCH = 1000, 64  # mempalace/convo_miner.py:167 (DRAWER_UPSERT_BATCH_SIZE), embedding.py:588 (_EF_API_BATCH)
MINE_TIMEOUT = 3600  # a detached mine of one transcript; the largest takes minutes on a CPU
SWEEP_TIMEOUT = 24 * 3600  # one transcript's sweep embeds every message whole; the largest takes about 5 hours on a CPU
# The cut (see Cut): the most content tokens one input may hold when it reaches the pinned embedder (the server adds its
# end token, and a request of 8,192 tokens is refused), and how a cut input is sent. Both measured on the pinned image
# with the shared service's settings: token ids get no end token from the server, so their vectors differ from the
# text's (cosine 0.886), and a cut is sent as the detokenized text.
CUT_LIMIT = 8190
CUT_AS = "text"

CORPUS = Path("/corpus")
WORK = Path("/work")
CHECKPOINT = WORK / "checkpoint"
LIVE = Path("/tmp/live")
# The hooks' home: ~/.mempalace/hook_state and ~/.mempalace/locks (flock, so not on the bind mount).
HOME = LIVE / "home"
STORE = LIVE / "store"  # the palace directory and what MemPalace writes beside it
STATE = HOME / ".mempalace" / "hook_state"  # mempalace/hooks_cli.py:39
HOOK_LOG = STATE / "hook.log"
EMBEDDER_URL = os.environ["EMBEDDER_URL"]
FAKETIME_LIB = next(Path("/usr/lib").glob("*/faketime/libfaketime.so.1"))

# What differs from a fresh install, each with where MemPalace 3.10.0's docs or source ask for it (paths in its repo).
# Everything reaches MemPalace through the environment of each hook; config.json in the store repeats the two
# embedding keys the README names, so a store says what embedded it.
CONFIG = {
    "MEMPALACE_EMBEDDING_MODEL": ("openai-compat", "README.md:335, mempalace/config.py:1304-1326: embeddings from an OpenAI-compatible /v1/embeddings server instead of the local ONNX model"),
    "MEMPALACE_EMBEDDING_API_URL": (EMBEDDER_URL, "mempalace/config.py:1489-1498: the eval's shared Qwen3-Embedding-0.6B on llama.cpp"),
    "MEMPALACE_EMBEDDING_API_MODEL": (API_MODEL, "mempalace/config.py:1500-1507: the model id each request names; the embedding function's name is derived from it (embedding.py:632-636)"),
    "MEMPALACE_CONFIG_DIR": (str(STORE / "config"), "mempalace/config.py:255-262: the config directory, inside the store so config.json and the write log travel with the palace"),
    "MEMPALACE_PALACE_PATH": (str(STORE / "palace"), "mempalace/config.py:919-935: the palace directory; hallways.json and tunnels.json are written beside it (config.py:938-952)"),
    "MEMPALACE_PYTHON": ("/app/.venv/bin/python", "mempalace/hooks_cli.py:100-113: the interpreter the hook spawns its detached mine with"),
    "ANONYMIZED_TELEMETRY": ("False", "chromadb's PostHog telemetry is on by default (backends/chroma.py:3088 builds a bare PersistentClient)"),
}
CONFIG_JSON = {"embedding_model": "openai-compat", "embedding_api_model": API_MODEL}  # README.md:335
STEPS = {
    "sweep": ("mempalace sweep <transcript>, at each quiet point, over every transcript that ended since the last one", "README.md:324-326: \"For per-message recall on top of the file-level chunks the hooks produce, run `mempalace sweep <transcript-dir>` periodically\"; one drawer per user and assistant message (sweeper.py:89-150)"),
    "sweep embedding timeout": ("3600 s for the sweep's embeddings requests (arms/mempalace/sweep.py), MemPalace's 120 s for the hooks, their mines and the diary", "mempalace/embedding.py:589, 667: the client waits 120 s per request of 64 texts and does not retry; a CPU embedder is slower than MemPalace's local ones and the sweep's cut inputs are whole contexts, so a request can take minutes (a GPU build never needs it)"),
    "palace set-embedder": ("mempalace palace set-embedder --model openai-compat, once, after the first mine", "mempalace/palace/collection.py:120-130: the hooks create the collection through the MCP session's own client (mcp_server/_session.py:200-260), which records no embedder identity, and every later open warns and names this command"),
}
# What else the manifest should say about the setup.
NOTES = {
    "time": "each hook runs under libfaketime (LD_PRELOAD, FAKETIME at the event's time, UTC), so filed_at, the diary checkpoints' dates and hallways.json's created_at are the session's time, in UTC; the sweep runs at the quiet point's time; authored_at is MemPalace's own, the transcript's latest timestamp (the maximum over every line, convo_miner.py:595-625)",
    "mine": "the plugin's detached mine (hooks_cli.py:1185-1197) runs in the background on a developer's machine; the build waits for it by pid before the next event, so no two mines overlap and no hook skips one (hooks_cli.py:402-442)",
    "sweep": "on (README.md:324-326, 'periodically'): at each quiet point, one `mempalace sweep <transcript>` per transcript that ended since the last one, in the order they ended, under libfaketime at the quiet point's time. Each session is swept whole once, so the drawers equal what `mempalace sweep <projects dir>` gives. One drawer per user and assistant record, the message whole, no wing and no room (sweeper.py:89-150), so a wing-filtered search does not see them",
    "cut": f"an input longer than the embedder's context is cut to its first {CUT_LIMIT} tokens before it is embedded, sent as {CUT_AS}; the drawer keeps the whole message, so BM25 still sees it. MemPalace's own embedders cut the same way, silently: embeddinggemma at 2,048 tokens (embedding.py:271, 454) and MiniLM at 256. Only the sweep goes through the cut (a shim inside the build, the sweep's embedding URL); the hooks, their mines and the diary reach the embedder directly, and their drawers are far under the context",
    "general_extract": "off: `mempalace mine --extract general` (palace/mined.py:53-55) adds drawers beside the exchange ones instead of replacing them, the hooks never run it (hooks_cli.py:1185-1197) and nothing recommends it on top of them",
    "closet_llm": "off: python -m mempalace.closet_llm is documented only in its module docstring and the 3.3 changelog, no Claude Code path runs it, and its client sends no custom headers",
    "init": "the plugin's /mempalace:init is not run: it writes project entity files and config defaults that neither conversation mining nor search reads",
    "mcp": "the diary and knowledge-graph writes a live agent makes through the MCP tools (mcp_server/tools_read.py:370-377) are not in a transcript and are not reproduced: the corpus agents never had MemPalace, so the knowledge graph stays empty",
    "resumes": "each session ends once, at its last line; a session resumed after an exit does not get the SessionEnd that exit fired. The final transcript drawers are unchanged; a diary checkpoint and a mine per resume are missing",
    "search": "for the run step: mempalace.searcher.search_memories (query.py:129), not the CLI or the MCP tool; candidate_strategy 'union' adds BM25 candidates (candidates.py:261-264); MEMPALACE_HYBRID_VECTOR_WEIGHT / _BM25_WEIGHT default 0.6/0.4 (config.py:1385-1418); max_distance 0.0; no query instruction is possible for openai-compat (embedding.py:638-641); wing 'sessions' holds the transcript drawers, the wing_<cwd leaf> wings of each session's first cwd hold the diary (not one per repository), the sweep drawers have no wing; since/before filter on filed_at, which is the session time here (UTC)",
    "embedder_requests": "derived, not observed: MemPalace's embedding function sends 64 texts a request and the miner hands it 1,000 drawers at a time; a diary write is one request; the sweep's are not counted",
    "prefix_reuse": "MemPalace sends 64 texts a request with no sentinel, so llama.cpp's prefix reuse applies to every request as the decision note's gotcha says; the order is fixed, so a fresh embedder and the same session order reproduce the vectors",
}
BASE_ENV = {"HOME": str(HOME), "PATH": "/app/.venv/bin:/usr/local/bin:/usr/bin:/bin", "TZ": "UTC", "PYTHONUNBUFFERED": "1"} | {k: v for k, (v, _) in CONFIG.items()}


def log(msg):
    print(f"mempalace: {msg}", file=sys.stderr, flush=True)


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


def text_of(content):
    if isinstance(content, str):
        return content.strip()
    return "\n".join(b.get("text", "") for b in content or [] if isinstance(b, dict) and b.get("type") == "text").strip()


def posix_cwd(cwd):
    """A Windows working directory at the POSIX form of its path: C:\\Users\\a\\b as /Users/a/b."""
    return "/" + cwd[3:].replace("\\", "/") if re.match(r"[A-Za-z]:\\", cwd) else cwd


def tool_results(content):
    return [b for b in content if isinstance(b, dict) and b.get("type") == "tool_result"] if isinstance(content, list) else []


def hook_env(when):
    """The hook's environment: MemPalace's settings, and libfaketime starting the clock at the event's time. The
    monotonic clock stays real, so timeouts and waits inside MemPalace are untouched."""
    at = when.astimezone(timezone.utc).strftime("@%Y-%m-%d %H:%M:%S")
    return BASE_ENV | {"LD_PRELOAD": str(FAKETIME_LIB), "FAKETIME": at, "FAKETIME_DONT_FAKE_MONOTONIC": "1"}


def fatal(msg):
    """Exit with a message, after saving MemPalace's log beside the store: the container and its /tmp go with the run."""
    if HOOK_LOG.exists():
        shutil.copy(HOOK_LOG, WORK / "hook.log")
    sys.exit(f"mempalace: {msg}")


def run(args, env, stdin=b"", timeout=MINE_TIMEOUT):
    try:
        r = subprocess.run(args, input=stdin, env=env, cwd=HOME, capture_output=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        fatal(f"`{' '.join(args[:3])}` ran for over {timeout} s; see {WORK / 'hook.log'}, then run again to resume from the last checkpoint")
    if r.returncode:
        fatal(f"`{' '.join(args[:3])}` exited with {r.returncode}: {r.stderr.decode(errors='replace').strip()[-2000:]}")
    return r.stdout.decode(errors="replace")


class HookLog:
    """MemPalace's own record of what each hook and mine did, ~/.mempalace/hook_state/hook.log: the hooks print
    nothing else, and the detached mine writes its report there."""

    def __init__(self):
        self.read = HOOK_LOG.stat().st_size if HOOK_LOG.exists() else 0

    def new_lines(self):
        if not HOOK_LOG.exists():
            return []
        with HOOK_LOG.open("rb") as f:
            f.seek(self.read)
            data = f.read()
        self.read += len(data)
        return data.decode(errors="replace").splitlines()


def mine_pids():
    """The pid of every mine the hooks recorded (hooks_cli.py:352-362, 499-545). The convo miner never clears its
    slot, so a dead pid is the normal state."""
    pids = []
    for f in (STATE / "mine_pids").glob("*.pid"):
        head = f.read_text().split()
        if head and head[0].isdigit():
            pids.append(int(head[0]))
    return pids


def alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except OSError:
        return False


def wait_mines():
    started = time.monotonic()
    while any(alive(p) for p in mine_pids()):
        if time.monotonic() - started > MINE_TIMEOUT:
            fatal(f"a mine ran for over {MINE_TIMEOUT} s; see {WORK / 'hook.log'}, then run again to resume from the last checkpoint")
        time.sleep(0.05)
    return time.monotonic() - started


def requests_for(drawers):
    """The embeddings requests behind a mine of `drawers` drawers: the miner upserts 1,000 at a time and the
    embedding function sends 64 texts a request."""
    full, rest = divmod(drawers, UPSERT_BATCH)
    return full * -(-UPSERT_BATCH // EF_BATCH) + -(-rest // EF_BATCH)


class Session:
    """One session through MemPalace's hooks, called as Claude Code calls the ones the plugin registers: Stop when a
    reply ends, PreCompact before a compaction, SessionEnd at the end. The transcript grows as Claude Code had written
    it by each hook, at the path Claude Code would use, and each step is one entry, so sessions open at the same time
    interleave by time. The turn rules are ai-memory's (arms/ai-memory/build.py, Session)."""

    def __init__(self, repo, path, counts, hook_log):
        self.name, self.counts, self.log = f"{repo}/{path.name}", counts, hook_log
        self.lines = [line for line in path.read_text().split("\n") if line.strip()]
        self.entries = [json.loads(line) for line in self.lines]  # so an entry's index is its line's
        self.times, when = [], None
        for entry in self.entries:
            when = stamp(entry) or when
            self.times.append(when)
        first_time = next(t for t in self.times if t)
        self.times = [t or first_time for t in self.times]
        self.sid = next(e["sessionId"] for e in self.entries if e.get("sessionId"))
        first = next((e["cwd"] for e in self.entries if e.get("cwd")), "/")  # the first entries often carry none
        self.cwd = posix_cwd(first)
        home = "/".join(self.cwd.split("/")[:3])
        self.transcript = Path(f"{home}/.claude/projects/{re.sub('[^A-Za-z0-9]', '-', first)}/{self.sid}.jsonl")
        self.transcript.parent.mkdir(parents=True, exist_ok=True)
        self.out = self.transcript.open("w")
        self.written, self.mtime = 0, 0.0
        self.turn, self.index, self.ended, self.pending = None, 0, False, []

    def due(self):
        return self.times[min(self.index, len(self.entries) - 1)]

    def stop(self, index, when):
        turn = self.turn
        if not turn or turn["stopped"] or not turn["reply"] or turn["interrupted"]:
            return
        turn["stopped"] = True
        self.pending.append(("stop", index, when, {"stop_hook_active": False, "last_assistant_message": turn["reply"]}))

    def turn_goes_on(self, index):
        """Whether the model goes on after this compaction before anything is typed: an automatic compaction can come
        in the middle of a turn, whose Stop Claude Code fires only when the turn ends."""
        for entry in self.entries[index + 1:]:
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
        """The hooks the next entry fires, in order, into self.pending."""
        if self.index >= len(self.entries):
            self.stop(len(self.entries), self.times[-1])
            self.pending.append(("session-end", len(self.entries), self.times[-1], {"reason": "other"}))
            self.ended = True
            return
        index, entry, when = self.index, self.entries[self.index], self.times[self.index]
        kind, subtype = entry.get("type"), entry.get("subtype")
        content = (entry.get("message") or {}).get("content")
        results = tool_results(content)
        if not entry.get("isSidechain"):
            typed = kind == "user" and not (results or entry.get("isMeta") or entry.get("isCompactSummary"))
            text = text_of(content) if typed else ""
            new_turn = text and not text.startswith((*NOTICES, "[Request interrupted"))
            if new_turn:
                self.stop(index, when)  # the turn before ends where it ran
            self.cwd = posix_cwd(entry["cwd"]) if entry.get("cwd") else self.cwd
            if kind == "system" and subtype in ("stop_hook_summary", "turn_duration"):
                self.stop(index, when)
            elif kind == "system" and subtype == "compact_boundary":
                trigger = (entry.get("compactMetadata") or {}).get("trigger") or "auto"
                if trigger == "manual" or not self.turn_goes_on(index):  # a typed /compact comes between turns
                    self.stop(index, when)
                self.pending.append(("precompact", index, when, {"trigger": trigger, "custom_instructions": ""}))
            elif text.startswith("[Request interrupted"):
                if self.turn:
                    self.turn["interrupted"] = True
            elif new_turn:
                self.turn = {"reply": "", "interrupted": False, "stopped": False}
            elif kind == "assistant":
                reply = text_of(content)
                api_error = bool(entry.get("isApiErrorMessage"))
                if reply and (api_error or (entry.get("message") or {}).get("model") == "<synthetic>"):
                    # Claude Code's own text (an API error, a usage limit, "No response requested."), not the model's:
                    # never a reply. A turn an API error ends fires StopFailure, which the plugin does not register.
                    self.counts["skipped_api_error_replies" if api_error else "skipped_synthetic_replies"] += 1
                    if self.turn and api_error:
                        self.turn["interrupted"] = True
                elif self.turn and reply:
                    # Text after a Stop means Claude went on (a Stop hook blocked it, a wakeup fired): Stop fires again.
                    self.turn["reply"], self.turn["interrupted"], self.turn["stopped"] = reply, False, False
        self.index += 1

    def grow(self, upto, when):
        """The transcript as Claude Code had written it when the hook fired: the lines before the entry that fires
        it, all of them at SessionEnd. Its mtime is the event's time, always later than the last one: a mine skips a
        transcript whose mtime it already holds (convo_miner.py:828-847)."""
        if upto > self.written:
            self.out.write("".join(line + "\n" for line in self.lines[self.written:upto]))
            self.out.flush()
            self.written = upto
        self.mtime = max(when.timestamp(), self.mtime + 0.01)
        os.utime(self.transcript, (self.mtime, self.mtime))

    def hook(self, event):
        """One hook through `mempalace hook run`, as the plugin's wrapper runs it, reading Claude Code's JSON on stdin
        (hooks_cli.py:1683-1703 reads session_id, stop_hook_active and transcript_path). Then the detached mine it may
        have spawned is waited for, and MemPalace's log says what both did."""
        slug, upto, when, fields = event
        self.grow(upto, when)
        body = {"session_id": self.sid, "transcript_path": str(self.transcript), "cwd": self.cwd, "hook_event_name": HOOKS[slug], **fields}
        started = time.monotonic()
        out = run(["mempalace", "hook", "run", "--hook", slug, "--harness", "claude-code"], hook_env(when), stdin=json.dumps(body, ensure_ascii=False).encode(), timeout=1800)
        self.counts["hook_seconds"] += time.monotonic() - started
        self.counts[slug] += 1
        try:
            answer = json.loads(out or "{}")
        except ValueError:
            fatal(f"the {slug} hook printed no JSON: {out.strip()[:500]}")
        if isinstance(answer, dict) and answer.get("decision"):
            fatal(f"the {slug} hook asked the model to save ({out.strip()}): silent_save is off")
        self.counts["mine_seconds"] += wait_mines()
        self.tally(slug, self.log.new_lines())

    def tally(self, slug, lines):
        """What MemPalace's log says the hook and its mine did; a mine that started must have finished, and a fault
        stops the build here, where the last checkpoint is the place to resume from."""
        text = "\n".join(lines)
        resume = "see " + str(WORK / "hook.log") + ", then run again to resume from the last checkpoint"
        for bad in ("Traceback", "EmbeddingAPIError", "transcript ingest hook failed", "PRAGMA quick_check after this mine", "Diary checkpoint failed", "Diary checkpoint error", "hallways skipped"):
            if bad in text:
                fatal(f"a {slug} hook of {self.name} logged `{bad}`; {resume}")
        if "Skipping mine" in text or re.search(r"Files skipped \(already filed\): [1-9]", text):
            fatal(f"a {slug} hook of {self.name} skipped its mine, so one was still running or the transcript looked unchanged; {resume}")
        fired = "TRIGGERING SAVE" in text
        self.counts["stops_fired"] += fired
        self.counts["diary_writes"] += text.count("Diary checkpoint saved")
        self.counts["diary_skipped"] += text.count("No recent messages to save")
        started = text.count("Transcript ingest started")
        self.counts["mines"] += started
        if started != text.count("Files processed:"):
            fatal(f"a {slug} hook of {self.name} started a mine that did not finish; {resume}")
        if (slug != "stop" or fired) and started != 1:
            fatal(f"the {slug} hook of {self.name} started {started} mines, not one; {resume}")
        for n in re.findall(r"Drawers filed: (\d+)", text):
            self.counts["drawers_filed"] += int(n)
            self.counts["embed_requests"] += requests_for(int(n))
        self.counts["embed_requests"] += text.count("Diary checkpoint saved")


class Cut(ThreadingHTTPServer):
    """A small OpenAI-compatible /v1/embeddings endpoint for the sweep alone: it forwards each request to the embedder
    and returns the answer unchanged, except that an input longer than CUT_LIMIT tokens is cut to its first CUT_LIMIT,
    as MemPalace's own embedders cut (embeddinggemma at 2,048 tokens, MiniLM at 256). The drawer keeps the whole
    message; only its vector sees the head. An error of the embedder goes back as its own status, so MemPalace's error
    path stops the sweep. Qwen3's tokenizer is byte-level BPE: a text of at most CUT_LIMIT bytes has at most that many
    tokens, so only longer ones are tokenized."""

    daemon_threads = True

    def __init__(self):
        super().__init__(("127.0.0.1", 0), CutHandler)
        self.stats, self.lock = Counter(), threading.Lock()
        self.base = EMBEDDER_URL.removesuffix("/v1")

    @property
    def url(self):
        return f"http://127.0.0.1:{self.server_address[1]}/v1"

    def post(self, path, body):
        conn = http.client.HTTPConnection(urlparse(self.base).netloc, timeout=3600)
        try:
            conn.request("POST", path, json.dumps(body), {"Content-Type": "application/json"})
            r = conn.getresponse()
            return r.status, r.read()
        finally:
            conn.close()

    def tokens(self, text):
        status, data = self.post("/tokenize", {"content": text, "add_special": False})
        if status != 200:
            raise RuntimeError(f"/tokenize answered {status}: {data[:300]!r}")
        return json.loads(data)["tokens"]

    def cut(self, text):
        """The input to send for a text: the text itself, or its first CUT_LIMIT tokens."""
        with self.lock:
            self.stats["forwarded"] += 1
        if len(text.encode()) <= CUT_LIMIT:
            return text
        ids = self.tokens(text)
        with self.lock:
            self.stats["max_tokens"] = max(self.stats["max_tokens"], len(ids))
        if len(ids) <= CUT_LIMIT:
            return text
        with self.lock:
            self.stats["cut"] += 1
            self.stats["dropped_tokens"] += len(ids) - CUT_LIMIT
        if CUT_AS == "token ids":
            return ids[:CUT_LIMIT]
        head = ids[:CUT_LIMIT]
        while True:
            status, data = self.post("/detokenize", {"tokens": head})
            if status != 200:
                raise RuntimeError(f"/detokenize answered {status}: {data[:300]!r}")
            text = json.loads(data)["content"]
            if len(self.tokens(text)) <= CUT_LIMIT:
                return text
            head = head[:-1]


class CutHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def reply(self, status, data):
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        try:
            if not self.path.endswith("/embeddings"):
                return self.reply(404, b'{"error": "only /embeddings"}')
            inputs = body["input"]
            body["input"] = [self.server.cut(x) if isinstance(x, str) else x for x in inputs] if isinstance(inputs, list) else self.server.cut(inputs)
            self.reply(*self.server.post("/v1/embeddings", body))
        except Exception as e:  # the sweep's own error path reports it
            log(f"the cut failed: {type(e).__name__}: {e}")
            self.reply(502, json.dumps({"error": f"cut: {e}"}).encode())


def sweep(cut, transcript, when, counts):
    """`mempalace sweep` of one ended transcript, at `when`, through the cut. Its summary line is the proof it ran to
    the end, and a first sweep of a session skips nothing (sweeper.py:196-330)."""
    started = time.monotonic()
    env = hook_env(when) | {"MEMPALACE_EMBEDDING_API_URL": cut.url}
    out = run(["python", "/app/sweep.py", str(transcript)], env, timeout=SWEEP_TIMEOUT)
    found = re.search(r"Swept .*: \+(\d+) new, (\d+) already present, (\d+) skipped", out)
    if not found:
        fatal(f"the sweep of {transcript} printed no summary: {out.strip()[-500:]}")
    added, present, skipped = map(int, found.groups())
    if skipped:
        fatal(f"the first sweep of {transcript} skipped {skipped} messages as older than a cursor")
    counts["sweeps"] += 1
    counts["sweep_drawers"] += added
    counts["sweep_present"] += present  # a message uuid that a transcript repeats
    counts["sweep_seconds"] += time.monotonic() - started


def record_embedder():
    """Record the palace's embedder identity, as MemPalace's own warning on every open asks (collection.py:120-130).
    The hooks create the collection through the MCP session's client, which never records it."""
    run(["mempalace", "palace", "set-embedder", "--model", "openai-compat"], BASE_ENV)
    if not (STORE / "palace" / "mempalace_embedder.json").exists():
        sys.exit("mempalace: `palace set-embedder` recorded nothing")


def inventory(store):
    """What a palace holds, read in a fresh process with the store's settings (a chromadb client is not reusable
    across copies of a palace)."""
    code = r"""
import json, os
from collections import Counter
from mempalace.palace import get_collection
col = get_collection(os.environ["MEMPALACE_PALACE_PATH"], create=False, read_only=True)
metas = []
for offset in range(0, col.count(), 1000):  # chromadb 1.5.9 fails an unbounded get past 32,766 rows; MemPalace pages by 1,000 (palace/mined.py:169-171)
    metas += col.get(limit=1000, offset=offset, include=["metadatas"])["metadatas"]
kinds = Counter((m.get("ingest_mode") or ("diary" if m.get("type") == "diary_entry" else "other")) for m in metas)
none = "(none)"  # a sweep drawer has no wing and no room
# A diary entry over chunk_size is several drawers with a chunk_index each (mcp_server/tools_diary.py:88-125).
print(json.dumps({"total": len(metas), "by_kind": dict(kinds), "by_wing": dict(Counter(m.get("wing") or none for m in metas)),
                  "by_room": dict(Counter(m.get("room") or none for m in metas)),
                  "diary_entries": sum(1 for m in metas if m.get("type") == "diary_entry" and m.get("chunk_index") == 0),
                  "sources": len({m.get("source_file") for m in metas if m.get("ingest_mode") == "convos"})}))
"""
    env = BASE_ENV | {"MEMPALACE_PALACE_PATH": str(store / "palace"), "MEMPALACE_CONFIG_DIR": str(store / "config")}
    return json.loads(run(["python", "-c", code], env))


def save(progress):
    """Write the palace to the checkpoint while no process holds it: the swap is two renames, and a run that finds
    only checkpoint.old uses it."""
    assert not any(alive(p) for p in mine_pids())
    progress["palace"] = inventory(STORE)
    tmp, old = WORK / "checkpoint.tmp", WORK / "checkpoint.old"
    shutil.rmtree(tmp, ignore_errors=True)
    tmp.mkdir(parents=True)
    shutil.copytree(STORE, tmp / "store")
    shutil.copy(HOOK_LOG, tmp / "hook.log")
    (tmp / "progress.json").write_text(json.dumps(progress, indent=2) + "\n")
    shutil.rmtree(old, ignore_errors=True)
    if CHECKPOINT.exists():
        CHECKPOINT.rename(old)
    tmp.rename(CHECKPOINT)
    shutil.rmtree(old, ignore_errors=True)


def restore():
    shutil.copytree(CHECKPOINT / "store", STORE)
    STATE.mkdir(parents=True)
    shutil.copy(CHECKPOINT / "hook.log", HOOK_LOG)
    progress = json.loads((CHECKPOINT / "progress.json").read_text())
    found = inventory(STORE)
    if found != progress["palace"]:
        sys.exit(f"mempalace: the restored palace does not match its checkpoint: {found} against {progress['palace']}")
    return progress


def manifest(progress, skipped, found):
    cut = Counter(progress.get("cut", {}))
    inputs = (WORK / "key.txt").read_text().splitlines()
    counts = Counter(progress["counts"])
    hallways = json.loads((WORK / "store" / "hallways.json").read_text()) if (WORK / "store" / "hallways.json").exists() else {}
    props = json.loads(urllib.request.urlopen(EMBEDDER_URL.removesuffix("/v1") + "/props", timeout=30).read())
    settings = [{"key": k, "value": v, "cite": cite} for k, (v, cite) in CONFIG.items()]
    settings += [{"key": f"config.json {k}", "value": v, "cite": "README.md:335"} for k, v in CONFIG_JSON.items()]
    settings += [{"key": k, "value": v, "cite": cite} for k, (v, cite) in STEPS.items()]
    return {
        "arm": "mempalace",
        "key": inputs[0],
        "inputs": inputs[1:],
        "device": os.environ["BILBO_EVAL_DEVICE"],
        "versions": {"mempalace": version("mempalace"), "chromadb": version("chromadb"), "python": sys.version.split()[0], "llama_cpp": props["build_info"]},
        "config": settings,
        "embedder": {"provider": "openai-compat", "model": "Qwen3-Embedding-0.6B Q8_0", "api_model": API_MODEL, "ef_name": EF_NAME, "dim": 1024, "query_prefix": None,
                     "requests": counts["embed_requests"], "identity_recorded": (WORK / "store" / "palace" / "mempalace_embedder.json").exists(),
                     "cut": {"limit_tokens": CUT_LIMIT, "sent_as": CUT_AS, "forwarded": cut["forwarded"], "cut": cut["cut"], "max_tokens": cut["max_tokens"],
                             "dropped_tokens": cut["dropped_tokens"]}},
        "notes": NOTES,
        "time": {"filed_at": "event time (libfaketime, UTC)", "authored_at": "the transcript's latest timestamp (MemPalace)"},
        "sessions": len(progress["done"]),
        "skipped": skipped,
        "by_repo": dict(sorted(Counter(name.split("/")[0] for name in progress["done"]).items())),
        "events": {"stop": counts["stop"], "precompact": counts["precompact"], "session_end": counts["session-end"]},
        "replay": {
            "stops_fired": counts["stops_fired"],
            "diary_writes": counts["diary_writes"],
            "diary_skipped_no_recent_messages": counts["diary_skipped"],
            "mines": counts["mines"],
            "drawers_filed_over_all_mines": counts["drawers_filed"],
            "sweeps": {"runs": counts["sweeps"], "drawers_added": counts["sweep_drawers"], "already_present": counts["sweep_present"],
                       "seconds": round(counts["sweep_seconds"])},
            "skipped_replies": {"api_error": counts["skipped_api_error_replies"], "synthetic": counts["skipped_synthetic_replies"]},
            "hook_seconds": round(counts["hook_seconds"]),
            "mine_seconds": round(counts["mine_seconds"]),
        },
        "drawers": {"total": found["total"], "by_kind": found["by_kind"], "by_wing": found["by_wing"], "by_room": found["by_room"],
                    "diary_entries": found["diary_entries"], "sessions_with_drawers": found["sources"]},
        "hallways": len(hallways.get("hallways", [])),
        "ingest_seconds": round(progress["ingest_seconds"]),
        "finished": datetime.now(timezone.utc).isoformat(timespec="seconds"),
    }


def interrupted(signum, frame):
    sys.exit("mempalace: stopped; run again to resume from the last checkpoint")


def main():
    signal.signal(signal.SIGINT, interrupted)
    signal.signal(signal.SIGTERM, interrupted)
    try:
        urllib.request.urlopen(EMBEDDER_URL.removesuffix("/v1") + "/health", timeout=30).close()
    except OSError as e:
        sys.exit(f"mempalace: the embedder {EMBEDDER_URL} does not answer: {e}")

    shutil.rmtree(LIVE, ignore_errors=True)
    (HOME / ".mempalace").mkdir(parents=True)  # the hooks' kill switch: hooks_cli.py:71-97
    if not CHECKPOINT.exists() and (WORK / "checkpoint.old").exists():
        (WORK / "checkpoint.old").rename(CHECKPOINT)
    if (CHECKPOINT / "progress.json").exists():
        progress = restore()
        log(f"resuming after {len(progress['done'])} sessions")
    else:
        (STORE / "config").mkdir(parents=True)
        (STORE / "palace").mkdir()
        (STORE / "config" / "config.json").write_text(json.dumps(CONFIG_JSON, indent=2) + "\n")
        STATE.mkdir(parents=True)
        progress = {"done": [], "counts": {}, "palace": None, "ingest_seconds": 0}
    counts, hook_log = Counter(progress["counts"]), HookLog()
    cut = Cut()
    cut.stats.update(progress.get("cut", {}))
    threading.Thread(target=cut.serve_forever, daemon=True).start()
    ended = []  # the transcripts to sweep at the next quiet point, in the order their sessions ended

    plan, skipped = sessions()
    done = set(progress["done"])
    queue = [(start, n, None, repo, path) for n, (start, repo, path) in enumerate(plan) if f"{repo}/{path.name}" not in done]
    heapq.heapify(queue)
    open_now, started = 0, time.monotonic()
    while queue:
        _, n, session, repo, path = heapq.heappop(queue)
        if session is None:
            session, open_now = Session(repo, path, counts, hook_log), open_now + 1
        session.step()
        for event in session.pending:
            session.hook(event)
            if counts["mines"] == 1 and not (STORE / "palace" / "mempalace_embedder.json").exists():
                record_embedder()
        session.pending.clear()
        if not session.ended:
            heapq.heappush(queue, (session.due(), n, session, repo, path))
            continue
        session.out.close()
        open_now -= 1
        progress["done"].append(session.name)
        ended.append(session.transcript)
        if open_now == 0:
            for transcript in ended:
                sweep(cut, transcript, session.times[-1], counts)
            ended.clear()
            progress["cut"] = dict(cut.stats)
            progress["counts"] = dict(counts)
            progress["ingest_seconds"] += time.monotonic() - started
            save(progress)
            started = time.monotonic()
            log(f"{len(progress['done'])}/{len(plan)} sessions: {counts['mines']} mines, {progress['palace']['total']} drawers so far")
    if not progress["palace"] or progress["done"] and progress["counts"] != dict(counts):
        progress["counts"] = dict(counts)
        progress["ingest_seconds"] += time.monotonic() - started
        save(progress)

    out = WORK / "store"
    shutil.rmtree(out, ignore_errors=True)
    shutil.copytree(CHECKPOINT / "store", out)
    shutil.copy(CHECKPOINT / "hook.log", WORK / "hook.log")
    found = inventory(out)
    (out / "manifest.json").write_text(json.dumps(manifest(progress, skipped, found), indent=2) + "\n")
    # A store that lost a diary checkpoint or a transcript is not packed: it would be compared with the other arms
    # as it stands.
    # (the faults themselves stop the build where they happen; these are the backstop)
    lost = []
    if found["sources"] != len(progress["done"]):
        lost.append(f"{found['sources']} sessions with drawers of {len(progress['done'])}")
    if found["by_kind"].get("sweep", 0) != counts["sweep_drawers"] or counts["sweeps"] != len(progress["done"]):
        lost.append(f"{found['by_kind'].get('sweep', 0)} sweep drawers of {counts['sweep_drawers']} filed, {counts['sweeps']} sweeps of {len(progress['done'])} sessions")
    if lost:
        fatal(f"the store is incomplete ({', '.join(lost)}); see {out / 'manifest.json'}")
    log(f"done: {out}")


if __name__ == "__main__":
    main()
