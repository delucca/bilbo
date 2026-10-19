"""Build the MemPalace store: replay every Claude Code session through MemPalace's own Claude Code plugin hooks, hook
by hook and in one timeline across the sessions it replays, into one palace, each hook run at the time its event
happened. Then save the palace directory with the files MemPalace writes beside it and a manifest in /work/store.
Whenever no session is open, the same files go to /work/checkpoint, and a run that finds them resumes from there.
The store is built in lanes (x-lanes in compose.yaml), one per repository: the replay models an independent MemPalace
installation for each repository. With BILBO_EVAL_LANE set, this replays only that repository's sessions, in a build container and a palace
of their own, into /work/lanes/<lane>, and stops at its last checkpoint; `--merge` then combines the lanes' palaces into
one store, and `--lanes` names the lanes. BILBO_EVAL_SESSIONS is a trial: N for the first N sessions, or `repo:N,repo:N`
for the first N sessions of each repository named. `--merge-palace <spec>` and `--merge-verify <spec>` are the merge's
steps that run with MemPalace's settings, each in a process of its own."""

import fcntl
import heapq
import http.client
import json
import os
import re
import shutil
import signal
import sqlite3
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
ROOT = Path("/work")
LANE = os.environ.get("BILBO_EVAL_LANE", "")  # the repository this container replays, when the store is built in lanes
LANES = ROOT / "lanes"
WORK = LANES / LANE if LANE else ROOT
MERGE = sys.argv[1:2] == ["--merge"]
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
    "prefix_reuse": "MemPalace sends 64 texts a request with no sentinel, so llama.cpp's prefix reuse applies to every request as the decision note's gotcha says. Each lane's events, and so its own requests, run in a fixed order; the lanes' requests reach the shared embedder interleaved in an order that varies from run to run, so a vector can differ by that noise from the same lane replayed alone, and a store built in lanes does not reproduce its vectors bit for bit. The merge copies the vectors the lanes got",
    "lanes": "the replay models an independent MemPalace installation for each repository: one palace per repository, each replayed on its own timeline with its own quiet points, sweeps and checkpoints, against the one shared embedder; then merged into one store (`merge`). One palace for every repository differs only in what it shares across them: hallways counted over every repository's transcript drawers, the mine's check for a conversation filed under another path (it matches nothing in this corpus) and quiet points when no session of any repository is open",
}
BASE_ENV = {"HOME": str(HOME), "PATH": "/app/.venv/bin:/usr/local/bin:/usr/bin:/bin", "TZ": "UTC", "PYTHONUNBUFFERED": "1"} | {k: v for k, (v, _) in CONFIG.items()}


def log(msg):
    print(f"mempalace: {msg}", file=sys.stderr, flush=True)


def stamp(entry):
    return datetime.fromisoformat(entry["timestamp"]) if entry.get("timestamp") else None


def session_id(path):
    """The session a corpus file records: Claude Code names its transcript after it, and the corpus keeps the name."""
    return path.name.split("--", 1)[1].removesuffix(".jsonl")


def own_start(entries, sid):
    """The index of the session's first own record. A cloud session continued in a terminal (`claude --teleport`)
    begins with the cloud session's history under that session's id: turns that ran on the other machine, which the
    terminal loaded at once and fired no hook for."""
    return next((i for i, e in enumerate(entries) if e.get("sessionId") == sid), 0)


def sessions():
    """Every Claude Code session as (start, repo, path), in the order they started across all repos, each at its first
    own record (own_start). A transcript with no timestamp is not Claude Code's and is left out; the hooks never see it."""
    found, other = [], []
    for path in sorted(CORPUS.glob("*/*.jsonl")):
        entries = [json.loads(line) for line in path.read_text().split("\n") if line.strip()]
        start = next((t for t in map(stamp, entries[own_start(entries, session_id(path)):]) if t), None)
        (found if start else other).append((start, path.parent.name, path))
    found.sort()
    limit = os.environ.get("BILBO_EVAL_SESSIONS") or ""
    if ":" in limit:  # repo:N,repo:N: the first N sessions of each repository named
        per = {repo: int(n) for repo, n in (part.split(":") for part in limit.split(","))}
        if unknown := set(per) - {repo for _, repo, _ in found}:
            sys.exit(f"mempalace: BILBO_EVAL_SESSIONS names repositories the corpus lacks: {sorted(unknown)}")
        taken, kept = Counter(), []
        for start, repo, path in found:
            if taken[repo] < per.get(repo, 0):
                taken[repo] += 1
                kept.append((start, repo, path))
        found = kept
    elif limit:
        found = found[: int(limit)]
    return found, [f"{repo}/{path.name}" for _, repo, path in other]


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
        self.sid = session_id(path)
        own = own_start(self.entries, self.sid)
        first = next((e["cwd"] for e in self.entries[own:] if e.get("cwd")), "/")  # the first entries often carry none
        self.cwd = posix_cwd(first)
        home = "/".join(self.cwd.split("/")[:3])
        self.transcript = Path(f"{home}/.claude/projects/{re.sub('[^A-Za-z0-9]', '-', first)}/{self.sid}.jsonl")
        self.transcript.parent.mkdir(parents=True, exist_ok=True)
        self.out = self.transcript.open("w")
        self.written, self.mtime = 0, 0.0
        # The lines before own fire nothing; they reach the transcript with the first hook, as they were there before it.
        self.turn, self.index, self.ended, self.pending = None, own, False, []

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


# The merge (merge()): what a lane's store may hold, the files MemPalace writes in and beside the palace (config.py:
# 255-262, 919-952) and Chroma's database with its one vector segment. Anything else, such as tunnels.json or a
# knowledge graph, fails the merge, which does not know how to combine it.
LANE_FILES = {"config/config.json", "config/wal/write_log.jsonl", "hallways.json", "palace/chroma.sqlite3",
              "palace/mempalace_embedder.json", "palace/.collection_type_fixed", "palace/.blob_seq_ids_migrated"}
SEGMENT_FILES = {"data_level0.bin", "header.bin", "index_metadata.pickle", "length.bin", "link_lists.bin"}
# A copied vector comes back within float32 rounding, not bit for bit: Chroma's hnswlib normalizes a cosine vector as
# it adds it, and the lane's had been normalized already.
VECTOR_TOLERANCE = 1e-6
REACH_K = (3,)  # see reach()
MERGE_TMP = ROOT / "merge.tmp"  # the lanes' palaces copied, beside them on the work dir's disk
MERGE_TIMEOUT = 12 * 3600


def collection_rows(store):
    """What a palace's Chroma database says about its one collection, read without opening it. The schema's keys are
    the metadata keys its drawers ever carried, in the order they first came; only the set is compared."""
    db = sqlite3.connect(f"file:{store / 'palace' / 'chroma.sqlite3'}?mode=ro&immutable=1", uri=True)
    try:
        (name, dimension, config, schema), *more = db.execute("SELECT name, dimension, config_json_str, schema_str FROM collections").fetchall()
        if more:
            sys.exit(f"mempalace: {store} holds more than one collection")
        schema = json.loads(schema)
        return {
            "collection": [name, dimension, config, schema.get("defaults")],
            "metadata": sorted(db.execute("SELECT key, str_value, int_value, float_value, bool_value FROM collection_metadata").fetchall()),
            "segments": sorted(db.execute("SELECT type, scope FROM segments").fetchall()),
            "migrations": db.execute("SELECT dir, version, hash FROM migrations ORDER BY dir, version").fetchall(),
            "queue": db.execute("SELECT config_json_str FROM embeddings_queue_config").fetchall(),
        }, set(schema.get("keys", {}))
    finally:
        db.close()


def merge(plan, skipped):
    """The lanes' palaces as one store, as one palace would hold every repository's drawers: every drawer of every lane
    with its id, document, metadata and vector, written into a fresh palace in interleave()'s order; each lane's
    hallways as its palace computed them, lane after lane; the write logs in interleave()'s order; the config and the
    embedder identity, which every lane holds alike. Every lane must have replayed all its
    sessions, no drawer id may be in two lanes, and the merged palace must hold every lane's drawers equal to its own,
    each vector within VECTOR_TOLERANCE in every component, or the build stops."""
    started = time.monotonic()
    lanes = list(dict.fromkeys(repo for _, repo, _ in plan))  # the order `--lanes` names them: by first session
    saved, progresses = {}, {}
    for lane in lanes:
        cp = LANES / lane / "checkpoint"
        progress = json.loads((cp / "progress.json").read_text()) if (cp / "progress.json").exists() else {"done": []}
        want = [f"{repo}/{path.name}" for _, repo, path in plan if repo == lane]
        if sorted(progress["done"]) != sorted(want):
            sys.exit(f"mempalace: the lane {lane} has replayed {len(progress['done'])} of its {len(want)} sessions; run again to finish it")
        files = {str(f.relative_to(cp / "store")) for f in (cp / "store").rglob("*") if f.is_file()}
        segment = lambda f: f.startswith("palace/") and f.count("/") == 2 and f.rsplit("/", 1)[1] in SEGMENT_FILES  # noqa: E731
        unknown = sorted(f for f in files if f not in LANE_FILES and not segment(f))
        missing = {"config/config.json", "palace/chroma.sqlite3", "palace/mempalace_embedder.json"} - files
        if unknown or missing or len({f.split("/")[1] for f in files if segment(f)}) > 1:
            sys.exit(f"mempalace: the lane {lane}'s store holds files the merge does not know {unknown}, lacks {sorted(missing)} or has two vector segments")
        saved[lane], progresses[lane] = cp / "store", progress

    def alike(what, read):
        found = {lane: read(saved[lane]) for lane in lanes}
        if len({json.dumps(v, sort_keys=True, default=str) for v in found.values()}) != 1:
            sys.exit(f"mempalace: the lanes' {what} differ: {json.dumps(found, default=str)[:2000]}")
        return found[lanes[0]]

    alike("config.json", lambda s: (s / "config" / "config.json").read_text())
    identity = alike("embedder identity", lambda s: (s / "palace" / "mempalace_embedder.json").read_text())
    markers = alike("marker files", lambda s: sorted(p.name for p in (s / "palace").iterdir() if p.is_file() and p.name.startswith(".")))
    alike("hallways schema", lambda s: json.loads((s / "hallways.json").read_text()).get("schema_version") if (s / "hallways.json").exists() else 1)
    collection = alike("collections", lambda s: collection_rows(s)[0])
    keys = set().union(*(collection_rows(saved[lane])[1] for lane in lanes))

    # The Chroma part runs with MemPalace's settings in processes of their own, on copies: opening a palace writes to it.
    shutil.rmtree(MERGE_TMP, ignore_errors=True)
    MERGE_TMP.mkdir(parents=True)
    spec = {"out": str(STORE / "palace"), "written": str(MERGE_TMP / "written.json"), "verified": str(MERGE_TMP / "verified.json"),
            "order": str(MERGE_TMP / "order.json"), "lanes": []}
    for lane in lanes:
        shutil.copytree(saved[lane] / "palace", MERGE_TMP / lane)
        hallways = saved[lane] / "hallways.json"
        spec["lanes"].append({"lane": lane, "palace": str(MERGE_TMP / lane), "hallways": str(hallways) if hallways.exists() else None})
    (MERGE_TMP / "spec.json").write_text(json.dumps(spec, indent=2))
    (STORE / "config").mkdir(parents=True)
    shutil.copy2(saved[lanes[0]] / "config" / "config.json", STORE / "config" / "config.json")
    log(f"merging {len(lanes)} lanes: the palace")
    run(["python", "/app/build.py", "--merge-palace", str(MERGE_TMP / "spec.json")], BASE_ENV | {"EMBEDDER_URL": EMBEDDER_URL}, timeout=MERGE_TIMEOUT)
    written = json.loads(Path(spec["written"]).read_text())
    # Opened once as MemPalace's hooks open a palace, through the MCP session's collection, as a Stop's diary write
    # does: that open pins Chroma's insert threads, which Chroma records in the collection's configuration.
    run(["python", "-c", "import mempalace.mcp_server as m; m._get_collection(create=False)"], BASE_ENV)
    # The embedder identity, recorded as each lane recorded it after its first mine (MemPalace's get_collection, which
    # created the palace, records the model with no dimension); it must be every lane's.
    record_embedder()
    if (STORE / "palace" / "mempalace_embedder.json").read_text() != identity:
        sys.exit("mempalace: the merged palace's embedder identity differs from the lanes'")
    log(f"merging {len(lanes)} lanes: checking the palace against every lane")
    run(["python", "/app/build.py", "--merge-verify", str(MERGE_TMP / "spec.json")], BASE_ENV | {"EMBEDDER_URL": EMBEDDER_URL}, timeout=MERGE_TIMEOUT)
    verified = json.loads(Path(spec["verified"]).read_text())
    got, got_keys = collection_rows(STORE)
    if got != collection or got_keys != keys:
        sys.exit(f"mempalace: the merged collection is not configured as the lanes' are: {got} {sorted(got_keys ^ keys)} against {collection}")
    for name in markers:  # MemPalace writes them on opening an existing palace; one the merge never reopened lacks them
        if not (STORE / "palace" / name).exists():
            shutil.copy2(saved[lanes[0]] / "palace" / name, STORE / "palace" / name)
    if extra := {p.name for p in (STORE / "palace").iterdir() if p.is_file() and p.name.startswith(".")} - set(markers):
        sys.exit(f"mempalace: the merged palace holds marker files the lanes do not: {sorted(extra)}")

    # The write log: every lane's lines, byte for byte, each lane's in the order MemPalace appended them (interleave).
    logs = []
    for lane in lanes:
        f = saved[lane] / "config" / "wal" / "write_log.jsonl"
        lines = f.read_text().splitlines(keepends=True) if f.exists() else []
        logs.append([(json.loads(line)["timestamp"], line) for line in lines])
    merged_log = interleave(logs)
    if merged_log:
        (STORE / "config" / "wal").mkdir(mode=0o700)
        with os.fdopen(os.open(STORE / "config" / "wal" / "write_log.jsonl", os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w") as f:
            f.write("".join(merged_log))
    if sorted(merged_log) != sorted(line for part in logs for _, line in part):
        sys.exit("mempalace: the merged write log is not every lane's lines")

    # One progress for the manifest: the sessions lane after lane, the counts added up.
    progress = {"done": [], "counts": Counter(), "cut": Counter(), "ingest_seconds": 0}
    for lane in lanes:
        p = progresses[lane]
        progress["done"] += p["done"]
        progress["counts"].update(p["counts"])
        cut = p.get("cut", {})
        progress["cut"].update({k: v for k, v in cut.items() if k != "max_tokens"})
        progress["cut"]["max_tokens"] = max(progress["cut"]["max_tokens"], cut.get("max_tokens", 0))
        progress["ingest_seconds"] += p["ingest_seconds"]
    out = ROOT / "store"
    shutil.rmtree(out, ignore_errors=True)
    shutil.copytree(STORE, out)
    found = inventory(out)
    if lost := incomplete(found, progress):
        fatal(f"the merged store is incomplete ({', '.join(lost)})")
    result = manifest(progress, skipped, found)
    hallways = json.loads((out / "hallways.json").read_text())["hallways"] if (out / "hallways.json").exists() else []
    owners = Counter(lane for lane, ids in written["hallway_ids"].items() for _ in ids)
    shared = Counter(i for ids in written["hallway_ids"].values() for i in set(ids))
    result["lanes"] = {
        lane: {
            "sessions": len(progresses[lane]["done"]),
            "ingest_seconds": round(progresses[lane]["ingest_seconds"]),
            **{k: round(progresses[lane]["counts"].get(k, 0)) for k in ("quiet_points", "mines", "sweeps", "diary_writes", "stop", "precompact", "session-end")},
            "drawers": written["lanes"][lane]["drawers"],
            "hallways": {"first": written["hallways"][lane][0], "count": written["hallways"][lane][1]},
            "write_log": len(logs[k]),
            "reach": written["lanes"][lane]["reach"],
        }
        for k, lane in enumerate(lanes)}
    result["merge"] = {
        "order": "each lane's drawers in the order they entered its palace, and its write-log lines in the order MemPalace appended them; next, always the lane head with the smallest filed_at (timestamp), a tie going to the earlier lane in `lanes`. Deterministic for identical lanes; not sorted by time, since a lane's own order is not (each hook, mine and sweep runs its own faked clock)",
        "drawers": verified["drawers"], "vector_tolerance": VECTOR_TOLERANCE,
        "vector_max_difference": verified["vector_max_difference"], "vector_min_cosine": verified["vector_min_cosine"],
        "hallways": {"records": len(hallways), "ids": len(shared), "ids_in_more_than_one_lane": sum(1 for n in shared.values() if n > 1)},
        "write_log": len(merged_log),
        "reach": verified["reach"],
        "seconds": round(time.monotonic() - started),
        "checked": "every lane replayed all its sessions; no drawer id is in two lanes; the merged palace holds every drawer of every lane and no other, its document and metadata equal to the lane's and its vector within vector_tolerance in every component, inserted in the order above; it passes MemPalace's own SQLite quick_check, and its collection is configured as every lane's; hallways.json is every lane's hallways as its palace computed them, lane after lane, and the write log every lane's lines; config.json and the embedder identity are every lane's",
    }
    if sum(owners.values()) != len(hallways):
        sys.exit("mempalace: the merged hallways are not every lane's")
    (out / "manifest.json").write_text(json.dumps(result, indent=2) + "\n")
    log(f"merged {len(lanes)} lanes in {round(time.monotonic() - started)} s: {verified['drawers']} drawers, vectors within "
        f"{verified['vector_max_difference']:.3g} of the lanes' in every component; {len(hallways)} hallways")
    log(f"done: {out}")


def interleave(streams):
    """The merge's order over the lanes. streams[k] is lane k's items as (key, item) in that lane's own order: its
    drawers in the order they entered its palace, keyed by filed_at, or its write-log lines in the order MemPalace
    appended them, keyed by timestamp. Each lane's order is kept; the next item is always the lane head with the
    smallest key, a tie going to the earlier lane. A lane's own keys are not sorted (each hook, mine and sweep runs its
    own faked clock, which advances with the time it takes), so neither is the result: it is deterministic for
    identical lanes, not sorted by time."""
    tagged = ([(key, k, n, item) for n, (key, item) in enumerate(stream)] for k, stream in enumerate(streams))
    return [item for _, _, _, item in heapq.merge(*tagged)]


def reach(col, ids, vectors):
    """How much of a palace MemPalace's vector index reaches: each drawer's own stored vector as the query of an
    unfiltered search, at k=3 and at k=1000, which widens Chroma's search (its ef is the larger of ef_search, 100, and
    k). A drawer is reached when it comes back, or a vector equal to its own does (distance under 1e-4: a text the
    palace holds twice). DIAG-mempalace.md measured the single palace this way."""
    missed = Counter()
    for b in range(0, len(ids), 100):
        part = [v.tolist() for v in vectors[b : b + 100]]
        for k in REACH_K:
            res = col.query(query_embeddings=part, n_results=min(k, len(ids)), include=["distances"])
            for i, hits, d in zip(ids[b : b + 100], res["ids"], res["distances"]):
                missed[f"missed_k{k}"] += i not in hits and not (d and d[0] < 1e-4)
    return {"drawers": len(ids), **{f"missed_k{k}": missed[f"missed_k{k}"] for k in REACH_K}}


def read_palace(path):
    """Every drawer of a palace through MemPalace's own collection, in the order the drawers entered it (Chroma's rows):
    ids, documents, metadata and float32 vectors."""
    import numpy as np
    from mempalace.palace import get_collection

    col = get_collection(path, create=False)
    got = {}
    for offset in range(0, col.count(), 1000):  # chromadb 1.5.9 fails an unbounded get past 32,766 rows
        page = col.get(limit=1000, offset=offset, include=["documents", "metadatas", "embeddings"])
        for i, d, m, e in zip(page["ids"], page["documents"], page["metadatas"], page["embeddings"]):
            got[i] = (d, m, np.asarray(e, dtype=np.float32))
    db = sqlite3.connect(f"file:{Path(path) / 'chroma.sqlite3'}?mode=ro&immutable=1", uri=True)
    order = [i for (i,) in db.execute("SELECT embedding_id FROM embeddings ORDER BY id")]
    db.close()
    if len(order) != len(got) or set(order) != set(got):
        sys.exit(f"mempalace: {path}'s collection and its rows disagree: {len(got)} drawers, {len(order)} rows")
    ids = order
    return col, ids, [got[i][0] for i in ids], [got[i][1] for i in ids], np.stack([got[i][2] for i in ids]) if ids else np.zeros((0, 1024), np.float32)


def kind(m):
    return m.get("ingest_mode") or ("diary" if m.get("type") == "diary_entry" else "other")


def merge_palace(spec):
    """The merge's palace, run with MemPalace's settings (BASE_ENV): every lane's drawers read through MemPalace's own
    collection, then added to a fresh palace that MemPalace's get_collection creates, 1,000 at a time as its miner
    upserts, with their vectors (never embedded again), in interleave()'s order by filed_at, each lane's drawers in the
    order they entered its palace. Then each lane's hallways, lane after
    lane, through MemPalace's own writer. Writes what it did to spec["written"]. Chroma's index of the same drawers in
    the same order is not the same graph twice: its inserts do not repeat bit for bit."""
    import gc

    from mempalace.config import MempalaceConfig
    from mempalace.hallways import _save_hallways
    from mempalace.palace import get_collection
    from mempalace.repair import _close_chroma_handles

    lanes, owner, streams, written = [], {}, [], {"lanes": {}, "hallways": {}, "hallway_ids": {}}
    for k, lane in enumerate(spec["lanes"]):
        col, ids, docs, metas, vectors = read_palace(lane["palace"])
        for i in ids:
            if i in owner:
                sys.exit(f"mempalace: the drawer {i} is in the lanes {owner[i]} and {lane['lane']}")
            owner[i] = lane["lane"]
        written["lanes"][lane["lane"]] = {
            "drawers": {"total": len(ids), "by_kind": dict(Counter(kind(m) for m in metas))}, "reach": reach(col, ids, vectors)}
        del col
        _close_chroma_handles(lane["palace"])
        gc.collect()
        lanes.append((ids, docs, metas, vectors))
        streams.append([(m.get("filed_at") or "", (k, j)) for j, m in enumerate(metas)])
    order = interleave(streams)
    Path(spec["order"]).write_text(json.dumps([lanes[k][0][j] for k, j in order]))
    col = get_collection(spec["out"], create=True)
    for b in range(0, len(order), UPSERT_BATCH):
        part = order[b : b + UPSERT_BATCH]
        col.add(ids=[lanes[k][0][j] for k, j in part], documents=[lanes[k][1][j] for k, j in part],
                metadatas=[lanes[k][2][j] for k, j in part], embeddings=[lanes[k][3][j].tolist() for k, j in part])
    if col.count() != len(order):
        sys.exit(f"mempalace: the merged palace holds {col.count()} drawers, not {len(order)}")
    del col
    _close_chroma_handles(spec["out"])
    records = []
    for lane in spec["lanes"]:
        found = json.loads(Path(lane["hallways"]).read_text())["hallways"] if lane["hallways"] else []
        written["hallways"][lane["lane"]] = [len(records), len(found)]
        written["hallway_ids"][lane["lane"]] = [h["id"] for h in found]
        records += found
    if records:
        _save_hallways(records, MempalaceConfig())
    written["drawers"] = len(order)
    Path(spec["written"]).write_text(json.dumps(written))


def merge_verify(spec):
    """The merge's check, in a process of its own, with MemPalace's settings: every drawer of every lane read back from
    the merged palace: its document and metadata equal, its vector within VECTOR_TOLERANCE in every component; no
    other drawer; inserted in the merge's order; MemPalace's own SQLite quick_check clean; the hallways every lane's.
    And the merged index's reach. Writes the result to spec["verified"]."""
    import numpy as np

    from mempalace.hallways import list_hallways
    from mempalace.repair import sqlite_integrity_errors

    col, ids, docs, metas, vectors = read_palace(spec["out"])
    if ids != json.loads(Path(spec["order"]).read_text()):
        sys.exit("mempalace: the merged palace's drawers are not in the merge's order")
    reached = reach(col, ids, vectors)  # before the lanes' palaces open, whose open can reset Chroma's clients
    at = {i: n for n, i in enumerate(ids)}
    worst, lowest, total = 0.0, 1.0, 0
    for lane in spec["lanes"]:
        _, lids, ldocs, lmetas, lvectors = read_palace(lane["palace"])
        total += len(lids)
        for i, d, m, v in zip(lids, ldocs, lmetas, lvectors):
            n = at.get(i)
            if n is None or docs[n] != d or metas[n] != m:
                sys.exit(f"mempalace: the merged palace's {i} differs from lane {lane['lane']}'s")
            got, want = vectors[n].astype(np.float64), v.astype(np.float64)
            off = float(np.abs(got - want).max())
            if not off <= VECTOR_TOLERANCE:  # a NaN fails too
                sys.exit(f"mempalace: the merged vector of {i} is off lane {lane['lane']}'s by {off:.3g}")
            worst = max(worst, off)
            lowest = min(lowest, float(got @ want / (np.linalg.norm(got) * np.linalg.norm(want))))
    if total != len(ids):
        sys.exit(f"mempalace: the merged palace holds {len(ids)} drawers, the lanes {total}")
    if errors := sqlite_integrity_errors(spec["out"]):
        sys.exit(f"mempalace: the merged palace fails MemPalace's quick_check: {errors[:5]}")
    want = []
    for lane in spec["lanes"]:
        want += json.loads(Path(lane["hallways"]).read_text())["hallways"] if lane["hallways"] else []
    if list_hallways() != want:
        sys.exit("mempalace: the merged hallways.json is not every lane's hallways, lane after lane")
    Path(spec["verified"]).write_text(json.dumps({
        "drawers": len(ids), "vector_max_difference": worst, "vector_min_cosine": lowest, "reach": reached}))


def interrupted(signum, frame):
    sys.exit("mempalace: stopped; run again to resume from the last checkpoint")


def main():
    if sys.argv[1:2] == ["--lanes"]:
        for repo in dict.fromkeys(repo for _, repo, _ in sessions()[0]):
            print(repo)
        return
    if sys.argv[1:2] == ["--merge-palace"]:
        return merge_palace(json.loads(Path(sys.argv[2]).read_text()))
    if sys.argv[1:2] == ["--merge-verify"]:
        return merge_verify(json.loads(Path(sys.argv[2]).read_text()))
    signal.signal(signal.SIGINT, interrupted)
    signal.signal(signal.SIGTERM, interrupted)
    try:
        urllib.request.urlopen(EMBEDDER_URL.removesuffix("/v1") + "/health", timeout=30).close()
    except OSError as e:
        sys.exit(f"mempalace: the embedder {EMBEDDER_URL} does not answer: {e}")

    shutil.rmtree(LIVE, ignore_errors=True)
    (HOME / ".mempalace").mkdir(parents=True)  # the hooks' kill switch: hooks_cli.py:71-97
    plan, skipped = sessions()
    if MERGE:
        return merge(plan, skipped)
    if LANE:
        plan = [s for s in plan if s[1] == LANE]
        if not plan:
            sys.exit(f"mempalace: the lane {LANE} has no session")
    # One build at a time in a work dir: the lanes share the mount, so this holds across containers.
    WORK.mkdir(parents=True, exist_ok=True)
    lock = (WORK / ".lock").open("w")
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        sys.exit(f"mempalace: another build is working in {WORK}")
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
            counts["quiet_points"] += 1
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
    if LANE:
        if lost := incomplete(progress["palace"], progress):
            fatal(f"the lane {LANE} is incomplete ({', '.join(lost)}); see {CHECKPOINT / 'progress.json'}")
        log(f"the lane {LANE} is done: its {len(plan)} sessions are in {CHECKPOINT}, for the merge")
        return

    out = WORK / "store"
    shutil.rmtree(out, ignore_errors=True)
    shutil.copytree(CHECKPOINT / "store", out)
    shutil.copy(CHECKPOINT / "hook.log", WORK / "hook.log")
    found = inventory(out)
    (out / "manifest.json").write_text(json.dumps(manifest(progress, skipped, found), indent=2) + "\n")
    if lost := incomplete(found, progress):
        fatal(f"the store is incomplete ({', '.join(lost)}); see {out / 'manifest.json'}")
    log(f"done: {out}")


def incomplete(found, progress):
    """What a palace lost against its replay: a store that lost a diary checkpoint or a transcript is not packed, as it
    would be compared with the other arms as it stands. (The faults themselves stop the build where they happen; this
    is the backstop.)"""
    counts, done, lost = Counter(progress["counts"]), len(progress["done"]), []
    if found["sources"] != done:
        lost.append(f"{found['sources']} sessions with drawers of {done}")
    if found["by_kind"].get("sweep", 0) != counts["sweep_drawers"] or counts["sweeps"] != done:
        lost.append(f"{found['by_kind'].get('sweep', 0)} sweep drawers of {counts['sweep_drawers']} filed, {counts['sweeps']} sweeps of {done} sessions")
    return lost


if __name__ == "__main__":
    main()
