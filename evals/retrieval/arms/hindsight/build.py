"""Build the Hindsight store: replay every session through Hindsight's own Claude Code integration (the coding-agents
package, its Stop hook as shipped), one timeline across all sessions, into one Hindsight server on loopback with one
memory bank per repository, set up the way the integration sets up a Claude Code user. Then save Hindsight's own
backup (`hindsight-admin backup`) with a manifest in /work/store. Whenever no session is open, the backup goes to
/work/checkpoint, and a run that finds it resumes from there.
BILBO_EVAL_DRY_RUN=1 walks the timeline on the host's Python and prints it, with no server, no LLM and no Docker."""

import hashlib
import heapq
import http.client
import json
import os
import platform
import re
import shutil
import signal
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import zipfile
from collections import Counter
from datetime import datetime, timedelta, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

DRY = os.environ.get("BILBO_EVAL_DRY_RUN") == "1"
NOTICES = ("<local-command-stdout>", "<local-command-stderr>", "<local-command-caveat>", "<system-reminder>")

CORPUS = Path(os.environ["BILBO_EVAL_CORPUS"]) if DRY else Path("/corpus")
WORK = Path("/work")
CHECKPOINT = WORK / "checkpoint"
LIVE = Path("/tmp/live")
HOME = LIVE / "home"
HOOK_TMP = LIVE / "hooks"
TRANSCRIPTS = LIVE / "transcripts"
CONFIGS = LIVE / "config"
REPOS = LIVE / "repos"
PLUGIN = "/opt/coding-agents/dist"
SERVER = "127.0.0.1:8888"
GATEWAY = ("127.0.0.1", 7802)
EMBEDDER_URL = os.environ.get("EMBEDDER_URL", "")
DATABASE_URL = os.environ.get("DATABASE_URL", "")
LLM_URL = os.environ.get("BILBO_EVAL_LLM_URL", "").rstrip("/")
LLM_KEY = os.environ.get("BILBO_EVAL_LLM_KEY", "")
LLM_MODEL = os.environ.get("BILBO_EVAL_LLM_MODEL", "")
LLM_HEADERS = json.loads(os.environ.get("BILBO_EVAL_LLM_HEADERS") or "{}")
QUIET_IDLE = 1800  # seconds a bank may stay busy with the LLM idle before the build gives up on it
HOOK_TIMEOUT = 60  # the host's timeout for the Stop hook (hindsight-integrations/coding-agents/src/harness/hook-lifecycle.ts:301)
DRY_REFRESH_SECONDS = 60  # the dry run has no server to time a refresh: it takes a nominal minute
FAR = datetime(9999, 1, 1, tzinfo=timezone.utc)

# Paths in Hindsight's repository at v0.10.3 (eb6df499), for the cites below.
CA = "hindsight-integrations/coding-agents/src/"
API = "hindsight-api-slim/hindsight_api/"
DOC = "hindsight-docs/docs/developer/"

# The decisions the user can flip, each one line; every flip changes the key.
EXTRACTION_MODE = "verbose"  # "concise" is the integration's shipped default
QUERY_PREFIX = "Instruct: Given a question about a software project's past coding sessions, retrieve the memories that answer it\nQuery:"  # "" turns it off
PAGES = "hourly"  # the knowledge pages' refresh: "hourly" (live use's cron, on the replayed clock), "final" or "off"
# The five similarity thresholds are calibrated for BAAI/bge-small-en-v1.5 (DOC configuration.mdx:1619-1623); each is
# matched to Qwen3 by quantile over memory pairs: (value, default, where it acts, cite). An empty dict keeps the defaults.
THRESHOLDS = {
    "HINDSIGHT_API_SEMANTIC_LINK_MIN_SIMILARITY": (0.58, 0.7, "build", API + "config.py:633, 1323: Qwen3 keeps bge's share of pairs (4.6%) at 0.593, 0.582 with the dates"),
    "HINDSIGHT_API_CONSOLIDATION_DEDUP_THRESHOLD": (0.97, 0.97, "build", API + "config.py:860, 1764: Qwen3's matched value is 0.967 to 0.969"),
    "HINDSIGHT_API_SEMANTIC_MIN_SIMILARITY": (0.1, 0.3, "recall", API + "config.py:630, 1320: bge's 0.3 cuts about 0.01% of pairs; Qwen3's 0.1 cuts under 0.001%"),
    "HINDSIGHT_API_GRAPH_SEED_MIN_SIMILARITY": (0.1, 0.3, "recall", API + "config.py:631, 1321: as the semantic floor"),
    "HINDSIGHT_API_TEMPORAL_SEMANTIC_MIN_SIMILARITY": (0.1, 0.1, "recall", API + "config.py:632, 1322: Qwen3 needs no change"),
}

# The server's environment: what differs from a fresh install, or what the manifest should say anyway, each with where
# Hindsight's docs or source ask for it.
SERVER_ENV = {
    "HINDSIGHT_API_DATABASE_URL": (DATABASE_URL, DOC + "storage.md:62-66: the pgvector service"),
    "HINDSIGHT_API_HOST": ("127.0.0.1", "loopback in the build container (the image defaults to 0.0.0.0)"),
    "HINDSIGHT_API_PORT": (SERVER.split(":")[1], "the image's default"),
    "HINDSIGHT_API_LLM_PROVIDER": ("openai", API + "config.py:1102: an OpenAI-compatible client"),
    "HINDSIGHT_API_LLM_BASE_URL": (f"http://{GATEWAY[0]}:{GATEWAY[1]}/v1", "the gateway in this file, in front of BILBO_EVAL_LLM_URL"),
    "HINDSIGHT_API_LLM_API_KEY": ("local", "the gateway adds the real key"),
    "HINDSIGHT_API_LLM_MODEL": (LLM_MODEL, "BILBO_EVAL_LLM_MODEL"),
    "HINDSIGHT_API_LLM_TIMEOUT": ("86400", API + "config.py:1182, 2391-2411: the gateway holds a call through a usage limit, past the 120 s default; retain, consolidation, reflect and page refreshes inherit it"),
    "HINDSIGHT_API_RETAIN_WALL_TIMEOUT": ("0", API + "config.py:1903, worker/poller.py:98-170: the worker's 3600 s ceiling on a whole retain would cut a held call; 0 lifts it"),
    "HINDSIGHT_API_CONSOLIDATION_WALL_TIMEOUT": ("0", API + "config.py:1910: the 7200 s ceiling without progress; same reason"),
    "HINDSIGHT_API_REFLECT_WALL_TIMEOUT": ("86400", API + "config.py:1927, engine/memory_engine.py:15558: the 300 s ceiling on a reflect and a page refresh; it cannot be 0"),
    "HINDSIGHT_API_SKIP_LLM_VERIFICATION": ("true", API + "config.py:904: build.py checks the endpoint itself first, as every arm does"),
    "HINDSIGHT_API_EMBEDDINGS_PROVIDER": ("openai", API + "config.py:471, " + DOC + "configuration.mdx:848-935: the shared Qwen3 embedder behind an OpenAI-compatible endpoint"),
    "HINDSIGHT_API_EMBEDDINGS_OPENAI_BASE_URL": (EMBEDDER_URL, "the shared embedder"),
    "HINDSIGHT_API_EMBEDDINGS_OPENAI_MODEL": ("embedder", "the shared embedder"),
    "HINDSIGHT_API_EMBEDDINGS_OPENAI_API_KEY": ("none", "the shared embedder takes none"),
    "HINDSIGHT_API_EMBEDDINGS_QUERY_PREFIX": (QUERY_PREFIX, DOC + "configuration.mdx:854, 924-930: the instruction an asymmetric model needs; consolidation recalls with it too (engine/consolidation/consolidator.py:3061-3112)"),
    "HINDSIGHT_API_BANK_STATS_CACHE_TTL_SECONDS": ("0", API + "config.py:1638: the 60 s cache would hide a pending operation from the quiet check"),
    "HINDSIGHT_API_MENTAL_MODEL_REFRESH_TICK_SECONDS": ("0", API + "config.py:2021, engine/maintenance.py:224-226: 0 turns the wall-clock page cron off; the build refreshes pages on the replayed clock"),
    "HINDSIGHT_API_LLM_TRACE_RETENTION_DAYS": ("3650", API + "config.py:2003, 2025: the default sweeps a day-old trace; the per-operation token counts must outlive the build"),
    "HINDSIGHT_API_LLM_TRACE_MAX_CHARS": ("100", API + "config.py:2004: keeps prompts out of the store"),
    "HF_HUB_OFFLINE": ("1", "the Dockerfile's ENV: the MiniLM cross-encoder is baked into the image and loads offline as the build user"),
    "HF_HOME": ("/home/hindsight/.cache/huggingface", "the Dockerfile's ENV: the build user's HOME is not the image's, so this points at the baked cache"),
}
# The integration's own config file (hindsight-coding-agents reads HINDSIGHT_CONFIG). Everything else stays at its default.
INTEGRATION = {
    "serverMode": ("self-hosted", CA + "core/config.ts:53-60: a Hindsight server the user runs"),
    "apiUrl": (f"http://{SERVER}", CA + "core/config.ts:60"),
    "autoSeed": (False, CA + "core/config.ts:207, core/session-start.ts:218-300: seeds a bank from git history, which is outside the corpus"),
    "codebaseSurvey": (False, CA + "core/config.ts:209: a headless claude surveys the checkout, which is outside the corpus"),
    "gitIngest": ("none", CA + "core/config.ts:227: git history is outside the corpus"),
    "autoUpdate": (False, CA + "core/config.ts:221: an npm version check"),
    "retainExtractionMode": (EXTRACTION_MODE, DOC + "configuration.mdx:1971-1973, retain.md:297: verbose, richer facts with full context and relationships; the integration syncs it onto its own strategies (" + CA + "core/missions.ts:720, 733-740)"),
}
# What the run step needs to reproduce what reaches the agent for a question asked as the first prompt of a new session: the
# hooks' sequence, in order. The faithful reproduction is the pinned tarball's own hooks, run unmodified per question.
PROMPT_TIME = {
    "faithful_reproduction": ("run the tarball's claude-sessionstart-hook.js, then claude-hook.js, unmodified, against the restored server, with this arm's per-repo "
                              "config (the integration.* entries of config, plus bankId) and a fresh HOME and TMPDIR per question; take their additionalContext",
                              CA + "harness/hook-lifecycle.ts:96-99, 226-232: both hooks hand their text to Claude Code as hookSpecificOutput.additionalContext"),
    "1_session_start": ("the <hindsight_knowledge> preamble: the tool guide plus the bank's page index, with reflectOnNewGoals false because autoInject is reflect",
                        CA + "core/session-start.ts:319-322, core/knowledge-injection.ts:119-133"),
    "2_first_prompt_reflect": ("autoInject is reflect (the default): POST .../reflect {query: buildReflectQuery(prompt), budget: low}, a 20000 ms client deadline; the "
                               "server's max_tokens default 4096 applies, its iterations are REFLECT_MAX_ITERATIONS x 0.5 = 5, steered by the bank's reflect_mission "
                               "(bank_config); the reflect API takes no query time, so its recency is the wall clock; it needs the server's LLM (recall does not); "
                               "the answer goes inside buildSystemInjection's <hindsight_memory> block, and an empty answer injects nothing",
                               CA + "core/config.ts:489, 580-587, core/hook.ts:262-285, 343, core/inject.ts:10-32, 74-112, core/hindsight.ts:722-746; " + API + "api/http.py:1498-1550, 1507, engine/memory_engine.py:15297-15303"),
    "3_fallback_on_timeout_or_5xx": ({"order": "page search, then observation recall only when the search found nothing or failed", "shared_timeout_ms": 7000,
                                      "pageSearch": {"q": "prompt[:500]", "limit": 10},
                                      "observationRecall": {"query": "prompt[:2000]", "types": ["observation"], "budget": "low", "max_tokens": 2000, "include": {"entities": None}},
                                      "formats": "PAGE_FALLBACK_LEAD then '- name (id): snippet'; RECALL_FALLBACK_LEAD then '- text'"},
                                     CA + "core/hook.ts:126, 148, 164-180, 301-305, core/hindsight.ts:250, 260-265, 761-775, 858-866, core/inject.ts:35-72, core/config.ts:493"),
    "4_every_prompt": ("the <hindsight_knowledge_refresh> block, the page index plus the tool guide, because pageRefreshEveryTurns is 1",
                       CA + "core/config.ts:656, core/hook.ts:309-355, core/knowledge-injection.ts:141-150"),
    "5_retries": ("a failed reflect is tried again on the next prompt, two attempts in all", CA + "core/hook.ts:95-108, 283-306"),
    "6_deferred_reflect": (False, CA + "core/session-start.ts:171, 218, 323, 420-421: the flag is set only for a cold repo with autoSeed on, or a bank with no page; "
                                       "this arm has autoSeed off and every bank has its five pages, so the first prompt reflects on every bank"),
}
NOTES = {
    "time": "facts and observations are dated from the session's first kept turn, which the integration takes from the transcript; created_at and page refresh times are the build's wall clock",
    "pages": "refreshed on the replayed clock by the server's cron rule: a page is due when it is stale and a cron fire has passed since its last refresh, and that refresh is stamped at its replayed time plus its measured duration, as live stamps its completion; the server's 5-minute tick is ignored, so a page can refresh up to one tick earlier than live; recency inside a refresh is measured from the wall clock",
    "vectors": "reproducible to about 1e-3 when embedded again, across devices: Hindsight embeds a fact with its date and its entities in extraction order, which text_signals keeps",
    "llm": "the calls behind this store: the calls of work an interrupt threw away are not counted",
    "skipped_hooks": "SessionStart and UserPromptSubmit are not run: neither writes a memory, the prompt hook's reflect only leaves LLM trace rows (" + CA + "core/session-start.ts, core/hook.ts); deepen.js sets up each bank once, in the foreground, at the bank's first session",
}
BASE_ENV = {"PATH": os.environ.get("PATH", "/usr/local/bin:/usr/bin:/bin"), "HOME": str(HOME), "TMPDIR": str(HOOK_TMP), "NO_COLOR": "1"}


def log(msg):
    print(f"hindsight: {msg}", file=sys.stderr, flush=True)


def stamp(entry):
    return datetime.fromisoformat(entry["timestamp"]) if entry.get("timestamp") else None


def iso(when):
    return when.astimezone(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")


def text_of(content):
    if isinstance(content, str):
        return content.strip()
    return "\n".join(b.get("text", "") for b in content or [] if isinstance(b, dict) and b.get("type") == "text").strip()


def tool_results(content):
    return [b for b in content if isinstance(b, dict) and b.get("type") == "tool_result"] if isinstance(content, list) else []


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


def bank_of(root):
    """The bank the integration's default template, coding-agent::{gitProject}, names for the main checkout (CA core/bank.ts:53, 176-196)."""
    return "coding-agent::" + root.rstrip("/").rsplit("/", 1)[-1]


def bank_path(bank):
    return f"/v1/default/banks/{urllib.parse.quote(bank, safe='')}"


def retry_after(headers):
    try:
        return int(float(headers.get("retry-after", 0)))
    except ValueError:  # an HTTP date: back off as usual
        return 0


def kind_of(body):
    """Which of Hindsight's jobs made an LLM call, from its system prompt (API engine/retain/fact_extraction.py:1058, 1219;
    engine/consolidation; engine/reflect/prompts.py:233, 1135)."""
    try:
        content = next(m["content"] for m in json.loads(body)["messages"] if m["role"] in ("system", "developer"))
    except (ValueError, KeyError, StopIteration, TypeError):
        return "other", ""
    head = (content if isinstance(content, str) else text_of(content))[:80]
    for kind, marker in (("extraction", "Extract "), ("consolidation", "You are a memory consolidation system"),
                         ("page_refresh", "CRITICAL: You MUST ONLY use information"), ("page_delta", "You are integrating *new information*")):
        if head.startswith(marker):
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
    """Hindsight's LLM calls go through here to BILBO_EVAL_LLM_URL. Hindsight tries a call 3 times 1, 2 and 4 s apart,
    reads Retry-After only to defer the whole operation and run it again, and never sleeps for it; here a failed call is
    sent again the way mem0's build retries its add() (a 429 waits out Retry-After, uncapped), so Hindsight only sees a
    slow provider. Every call to the LLM is counted."""

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
                message = (answer.get("choices") or [{}])[0].get("message") or {}
                empty = not (message.get("content") or message.get("tool_calls"))  # a reflect's answers are tool calls
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
            sys.exit("hindsight: the LLM failed 10 times in a row; run again to resume from the last checkpoint")


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


def sql(query, *args):
    """One query on the build's database (asyncpg is in Hindsight's image)."""
    import asyncio
    import asyncpg

    async def run():
        c = await asyncpg.connect(DATABASE_URL)
        try:
            return [dict(r) for r in await c.fetch(query, *args)]
        finally:
            await c.close()

    return asyncio.run(run())


def admin(*args):
    """`hindsight-admin` on the build's database; it talks to Postgres directly, so the server can stay up."""
    r = subprocess.run(["hindsight-admin", *args], env=os.environ | {"HINDSIGHT_API_DATABASE_URL": DATABASE_URL}, stdin=subprocess.DEVNULL,
                       capture_output=True, text=True, timeout=3600)
    if r.returncode:
        sys.exit(f"hindsight: `hindsight-admin {args[0]}` exited with {r.returncode}: {(r.stdout + r.stderr).strip()[-2000:]}")


def restore_database(path):
    """A logical restore into a fresh database: the migration first (with the embedder's dimension), then the rows, then
    the per-bank vector indexes a restore leaves out (DOC admin-cli.md:70-110)."""
    admin("run-db-migration", "--embedding-dimension", "1024")
    admin("restore", str(path), "--yes")
    admin("repair-bank", "--all")


class Server:
    """`hindsight-api` on loopback in this container."""

    def __init__(self, gateway):
        self.gateway, self.process, self.read = gateway, None, 0

    def env(self):
        env = {k: v for k, v in os.environ.items() if not k.startswith("BILBO_EVAL_")}
        return env | {k: v for k, (v, _) in SERVER_ENV.items()} | {k: str(v) for k, (v, *_) in THRESHOLDS.items()}

    def start(self):
        log_file = WORK / "server.log"
        self.read = log_file.stat().st_size if log_file.exists() else 0  # earlier runs' lines are not this store's
        self.process = subprocess.Popen(["hindsight-api"], env=self.env(), cwd=LIVE, stdin=subprocess.DEVNULL, stdout=(WORK / "server.log").open("a"), stderr=subprocess.STDOUT)
        deadline = time.monotonic() + 600
        while True:
            self.check()
            try:
                urllib.request.urlopen(f"http://{SERVER}/health", timeout=5).close()
                return
            except OSError:
                if time.monotonic() > deadline:
                    sys.exit(f"hindsight: the server did not answer /health in 10 minutes; see {WORK / 'server.log'}")
                time.sleep(1)

    def check(self):
        if self.process.poll() is not None:
            sys.exit(f"hindsight: the server exited with {self.process.returncode}; see {WORK / 'server.log'}, then run again to resume from the last checkpoint")

    def stop(self):
        if self.process and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(60)
            except subprocess.TimeoutExpired:
                self.process.kill()

    def call(self, method, path, body=None):
        request = urllib.request.Request(f"http://{SERVER}{path}", json.dumps(body).encode() if body is not None else None, {"Content-Type": "application/json"}, method=method)
        try:
            with urllib.request.urlopen(request, timeout=600) as r:
                data = r.read()
        except urllib.error.HTTPError as e:
            sys.exit(f"hindsight: {method} {path}: {e.code} {e.read()[:500]!r}")
        except (OSError, http.client.HTTPException) as e:  # a wedged server: refused, reset or timed out
            sys.exit(f"hindsight: {method} {path}: {e}; see {WORK / 'server.log'}")
        return json.loads(data) if data else None

    def logged(self):
        """The server log's whole lines written since the last call."""
        with (WORK / "server.log").open("rb") as f:
            f.seek(self.read)
            data = f.read()
        data = data[: data.rfind(b"\n") + 1]
        self.read += len(data)
        return data.decode(errors="replace").splitlines()

    def get(self, path):
        return self.call("GET", path)

    def post(self, path, body=None):
        return self.call("POST", path, body)

    def stats(self, bank):
        return self.get(f"{bank_path(bank)}/stats?refresh=true")

    def failed(self, bank):
        done = self.get(f"{bank_path(bank)}/operations?status=failed&limit=20")
        return [(o["task_type"], o.get("error_message")) for o in done["operations"]]

    def pending(self, bank):
        """The operations still waiting, with the error a retried one keeps: why a bank stays busy."""
        found = []
        for status in ("pending", "processing"):
            found += [(o["task_type"], status, o.get("error_message")) for o in self.get(f"{bank_path(bank)}/operations?status={status}&limit=5")["operations"]]
        return found

    def quiet(self, bank):
        """Wait until the bank has no operation pending or processing, no fact awaiting consolidation and no failure, three
        polls 1 s apart. Consolidation, graph maintenance and vector index maintenance are queued inside the retain before
        its operation completes (API engine/memory_engine.py:6737, 6819-6853), so no poll falls between them. While the
        bank is busy and the LLM idle, the build stops after QUIET_IDLE: a call held through a usage limit is progress."""
        calm, idle_since = 0, time.monotonic()
        while calm < 3:
            self.check()
            self.gateway.check()
            s = self.stats(bank)
            if s["failed_operations"] or s["failed_consolidation"]:
                sys.exit(f"hindsight: failed work in {bank}: {self.failed(bank)}, failed operations {s['failed_operations']}, failed consolidation {s['failed_consolidation']}")
            ops = s["operations_by_status"]
            busy = ops.get("pending", 0) + ops.get("processing", 0) + s["pending_consolidation"]
            calm = 0 if busy else calm + 1
            if not busy or self.gateway.in_flight:
                idle_since = time.monotonic()
            else:
                idle_since = max(idle_since, self.gateway.active)
                if time.monotonic() - idle_since > QUIET_IDLE:
                    sys.exit(f"hindsight: {bank} stayed busy {QUIET_IDLE} s with the LLM idle: {ops}, {s['pending_consolidation']} facts awaiting consolidation: {self.pending(bank)}")
            time.sleep(1)


def write_configs(roots):
    """Two config files per repository: the Stop hook's, with its bank, and deepen's, without (CA core/bank.ts:344-375: the
    pages' scope names the repository only for a dynamic config; the bank itself is pinned by deepen's --bank)."""
    base = {k: v for k, (v, _) in INTEGRATION.items()}
    for repo, root in roots.items():
        (CONFIGS / f"{repo}.json").write_text(json.dumps(base | {"bankId": bank_of(root)}))
        (CONFIGS / f"{repo}.deepen.json").write_text(json.dumps(base))


def hook_env(config):
    return BASE_ENV | {"HINDSIGHT_CONFIG": str(config), "HINDSIGHT_DIAG_FILE": str(WORK / "hooks" / "diag.jsonl"), "HINDSIGHT_LOG_FILE": str(WORK / "hooks" / "plugin.log")}


def deepen(server, quiet, repo, bank):
    """The integration's setup engine, once per bank, in the foreground: it creates the bank with its coding missions,
    strategies and labels, and the five knowledge pages (CA deepen.ts:55-320). It makes no LLM call."""
    basename = bank.split("::", 1)[1]
    (REPOS / basename).mkdir(parents=True, exist_ok=True)
    command = ["node", f"{PLUGIN}/deepen.js", "--repo", str(REPOS / basename), "--bank", bank, "--harness", "claude-code", "--git-ingest", "none",
               "--config", str(CONFIGS / f"{repo}.deepen.json")]
    try:
        r = subprocess.run(command, env=hook_env(CONFIGS / f"{repo}.deepen.json"), cwd=HOME, stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=900)
    except subprocess.TimeoutExpired:
        sys.exit(f"hindsight: deepen.js for {bank} ran past 15 minutes")
    (WORK / "hooks" / f"deepen-{basename}.log").write_text(r.stdout + r.stderr)
    if r.returncode:
        sys.exit(f"hindsight: deepen.js for {bank} exited with {r.returncode}: {(r.stdout + r.stderr).strip()[-2000:]}")
    quiet(bank)
    overrides = server.get(f"{bank_path(bank)}/config")["overrides"]
    if "conversation" not in (overrides.get("retain_strategies") or {}) or overrides.get("retain_extraction_mode") != EXTRACTION_MODE:
        sys.exit(f"hindsight: the bank {bank} lacks the conversation strategy or the {EXTRACTION_MODE} extraction mode: {json.dumps(overrides)[:1000]}")
    tree = server.get(f"{bank_path(bank)}/knowledge-base/tree")["roots"]
    pages = [n for n in tree if n["kind"] == "page"]
    if len(pages) != 5 or any(f"Scope this page to {basename} ITSELF" not in (p["description"] or "") for p in pages):
        sys.exit(f"hindsight: the pages of {bank} are not scoped to {basename}: {[p['name'] for p in pages]}")


MEMORY_TAG = re.compile(r"<(hook_prompt|task-notification|system-reminder|hindsight_memory|hindsight_memories|hindsight_bank|relevant_memories|user_feedback|hindsight_knowledge|hindsight_knowledge_refresh)\b[\s\S]*?</\1>")


def rendered(entry):
    """What the Stop hook's reader keeps of a transcript line: one turn of prose and one action turn per tool call (CA
    core/transcript.ts:50-118, core/transcript-util.ts:32-67), as the JSONL lines the hook sends."""
    if entry.get("type") not in ("user", "assistant") or entry.get("isMeta") or entry.get("isSidechain") or entry.get("isCompactSummary"):
        return []
    content = (entry.get("message") or {}).get("content")
    if isinstance(content, str):
        text = MEMORY_TAG.sub("", content).strip()
        turns = [(entry["type"], text)] if text else []
    elif isinstance(content, list):
        texts = [t for t in (MEMORY_TAG.sub("", b["text"]).strip() for b in content if isinstance(b, dict) and b.get("type") == "text" and isinstance(b.get("text"), str)) if t]
        actions = [("action", action(b["name"], b.get("input"))) for b in content if isinstance(b, dict) and b.get("type") == "tool_use" and isinstance(b.get("name"), str)]
        turns = ([(entry["type"], "\n".join(texts))] if texts else []) + actions
    else:
        return []
    return [json.dumps({"role": role, "content": text, "timestamp": entry.get("timestamp")}, ensure_ascii=False, separators=(",", ":")) for role, text in turns]


def action(tool, tool_input):
    target = ""
    if isinstance(tool_input, dict):
        for key in ("file_path", "path", "notebook_path", "command", "pattern", "query", "url", "name", "id"):
            value = tool_input.get(key)
            if isinstance(value, str) and value.strip():
                target = value.strip().split("\n")[0]
                break
    if len(target) > 100:
        target = target[:100] + "…"
    return f"{tool} {target}" if target else tool


class Session:
    """One session, written to a transcript file line by line and handed to Hindsight's Stop hook as Claude Code hands it:
    when a reply ends. Claude Code's own record of the Stop (a stop_hook_summary entry, or the first attachment whose hookEvent
    is Stop) fires it once per event, even with no reply text or after an API error. Without a record a Stop fires at a system
    turn_duration entry, at the next typed prompt (before its line is written), at the end of the session, and at a compaction
    unless the turn goes on; a turn the user interrupted, an API error or a synthetic reply ended sends none, and its lines
    go out with the next Stop. A line whose uuid the session already wrote (a resumed session's copied history) goes into the
    transcript but is no event and never moves the clock back. Each step is one transcript line, so sessions open at the same
    time interleave by time. SessionStart and UserPromptSubmit write nothing to the store and are not run."""

    def __init__(self, repo, path, replay):
        self.repo, self.name, self.replay = repo, f"{repo}/{path.name}", replay
        self.lines = [line for line in path.read_text().splitlines() if line.strip()]
        self.entries = [json.loads(line) for line in self.lines]
        self.times, self.copy, when, seen = [], [], None, set()
        for entry in self.entries:
            uuid = entry.get("uuid")
            self.copy.append(bool(uuid) and uuid in seen)
            seen.add(uuid)
            at = stamp(entry)
            when = max(when, at) if self.copy[-1] and when and at else (at or when)
            self.times.append(when)
        first_time = next(t for t in self.times if t)
        self.times = [t or first_time for t in self.times]
        self.sid = next(e["sessionId"] for e in self.entries if e.get("sessionId"))
        self.cwd = next((e["cwd"] for e in self.entries if e.get("cwd")), "/")  # the first entries often carry none
        self.bank = replay.banks[repo]
        self.turn, self.index, self.ended, self.file = None, -1, False, None
        self.written, self.kept, self.stops = 0, 0, 0

    def due(self):
        return self.times[min(max(self.index, 0), len(self.entries) - 1)]

    def next_time(self):
        return self.due()

    def write(self):
        if self.file:
            self.file.write(self.lines[self.index] + "\n")
        self.written = self.index + 1
        self.kept += len(rendered(self.entries[self.index]))

    def stop(self):
        turn = self.turn
        if not turn or turn["stopped"] or not turn["reply"] or turn["interrupted"]:
            return
        turn["stopped"] = True
        if self.file:
            self.file.flush()
        self.stops += 1
        self.replay.stop(self, self.times[self.written - 1], turn["reply"])

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
            self.replay.begin(self)
        elif self.index < len(self.entries):
            self.event()
            self.index += 1
        else:
            self.stop()
            if self.file:
                self.file.close()
            self.ended = True

    def event(self):
        entry = self.entries[self.index]
        kind, subtype = entry.get("type"), entry.get("subtype")
        content = (entry.get("message") or {}).get("content")
        results = tool_results(content)
        if entry.get("isSidechain") or self.copy[self.index]:
            self.write()
            return
        record = (kind == "system" and subtype == "stop_hook_summary") or (kind == "attachment" and (entry.get("attachment") or {}).get("hookEvent") == "Stop")
        if record and self.turn and not self.turn["stopped"]:
            self.turn["interrupted"] = False
            self.turn["reply"] = self.turn["reply"] or " "
        typed = kind == "user" and not (results or entry.get("isMeta") or entry.get("isCompactSummary"))
        text = text_of(content) if typed else ""
        new_turn = text and not text.startswith((*NOTICES, "[Request interrupted"))
        if new_turn:
            self.stop()  # the turn before ends where it ran, before this prompt's line is written
        self.write()
        self.cwd = entry.get("cwd") or self.cwd  # Claude Code sends the cwd the session is in at each hook

        if record or (kind == "system" and subtype == "turn_duration"):
            self.stop()
        elif kind == "system" and subtype == "compact_boundary":
            trigger = (entry.get("compactMetadata") or {}).get("trigger") or "auto"
            if trigger == "manual" or not self.turn_goes_on():  # a typed /compact comes between turns
                self.stop()
        elif text.startswith("[Request interrupted"):
            if self.turn:
                self.turn["interrupted"] = True
        elif new_turn:
            self.turn = {"reply": "", "interrupted": False, "stopped": False}
        elif kind == "assistant":
            reply = text_of(content)
            api_error = bool(entry.get("isApiErrorMessage"))
            if reply and (api_error or (entry.get("message") or {}).get("model") == "<synthetic>"):
                # Claude Code's own text (an API error, a usage limit, "No response requested."), not the model's: never a
                # reply. A turn an API error ends fires StopFailure, which the integration does not register, not Stop; a
                # synthetic reply ends its turn with no Stop in all 10 of the corpus's. Claude Code's record, if any, wins.
                self.replay.counts["skipped_api_error_replies" if api_error else "skipped_synthetic_replies"] += 1
                if self.turn:
                    self.turn["interrupted"] = True
            elif self.turn and reply:
                # Text after a Stop means Claude went on (a Stop hook blocked it, a wakeup fired): Stop fires again.
                self.turn["reply"], self.turn["interrupted"], self.turn["stopped"] = reply, False, False


def page_minute(bank, page):
    """The minute the integration gives a page's hourly cron, hashed from the bank and the page (CA core/missions.ts:476-525)."""
    digest = hashlib.sha256(f"{bank}\x00{page}\x000".encode()).digest()
    return int.from_bytes(digest[:4], "big") % 60


PAGE_NAMES = ("Component map", "Core concepts", "Conventions and patterns", "Key decisions and rationale", "Initiatives and enhancements")


class Pages:
    """The knowledge pages on the sessions' clock. The server's own cron loop (API engine/maintenance.py:528-654) refreshes a
    page at the first 5-minute tick at which it is stale and its cron has fired since its last refresh; that loop is off
    here, so a page last refreshed at L that went stale at the Stop at S is refreshed at R = max(the first fire after L, S),
    just before the first Stop of its bank at or after R. A refresh writes no fact or observation. Nothing in a bank changes
    between its own Stops, so the page sees the state it had at R. `final` refreshes every stale page once at the end."""

    def __init__(self, server, quiet, mode, state):
        self.server, self.mode, self.quiet = server, mode, quiet
        self.banks = {bank: {name: {"id": p["id"], "cron": p["cron"], "last": datetime.fromisoformat(p["last"]), "stale_since": datetime.fromisoformat(p["stale_since"]) if p["stale_since"] else None,
                                    "refreshes": p["refreshes"]} for name, p in pages.items()} for bank, pages in state.items()}
        self.latest = {}

    def fire(self, cron, after):
        """The first fire of the cron after a time, by the server's own library."""
        from croniter import croniter

        return croniter(cron, after).get_next(datetime)

    def state(self):
        return {bank: {name: {"id": p["id"], "cron": p["cron"], "last": p["last"].isoformat(), "stale_since": p["stale_since"].isoformat() if p["stale_since"] else None,
                              "refreshes": p["refreshes"]} for name, p in pages.items()} for bank, pages in self.banks.items()}

    def tree(self, bank):
        return [n for n in self.server.get(f"{bank_path(bank)}/knowledge-base/tree")["roots"] if n["kind"] == "page"]

    def setup(self, bank, start):
        """After deepen: each page's id and cron; live, page creation refreshes the empty page at the bank's first session."""
        self.banks[bank] = {n["name"]: {"id": n["mental_model_id"], "cron": n["trigger"]["refresh_cron"], "last": start, "stale_since": None, "refreshes": 0} for n in self.tree(bank)}

    def is_stale(self, bank, page):
        """The cron loop's own test, with the tag scope and the retracted-grounding check; the tree's flag is the batch variant without the second check."""
        return bool(self.server.get(f"{bank_path(bank)}/mental-models/{page['id']}?detail=full")["is_stale"])

    def after_stop(self, bank, when):
        self.latest[bank] = when
        if self.mode != "hourly":
            return
        for page in self.banks[bank].values():
            if page["stale_since"] is None and self.is_stale(bank, page):
                page["stale_since"] = when

    def tick(self, bank, now):
        if self.mode != "hourly":
            return
        for name, page in self.banks[bank].items():
            if page["stale_since"] is None:
                continue
            due = max(self.fire(page["cron"], page["last"]), page["stale_since"])
            if due <= now:
                self.refresh(bank, name, page, due)

    def refresh(self, bank, name, page, due):
        started = time.monotonic()
        self.server.post(f"{bank_path(bank)}/mental-models/{page['id']}/refresh")
        self.quiet(bank)
        took = timedelta(seconds=time.monotonic() - started)
        if self.server.get(f"{bank_path(bank)}/mental-models/{page['id']}?detail=full")["last_refresh_failed_at"]:
            sys.exit(f"hindsight: the refresh of {name} in {bank} failed")
        page.update(last=due + took, stale_since=None, refreshes=page["refreshes"] + 1)  # live stamps last_refreshed_at when the refresh completes

    def finish(self):
        """The end of the timeline, the daemon still running (CA core/daemon.ts:36-37): hourly lets every stale page's cron fire."""
        for bank, pages in self.banks.items():
            if self.mode == "hourly":
                self.tick(bank, FAR)
            elif self.mode == "final":
                for name, page in pages.items():
                    if self.is_stale(bank, page):
                        self.refresh(bank, name, page, self.latest.get(bank, page["last"]))


class Replay:
    """The Stop hook, the server around it and the checks between them."""

    def __init__(self, server, gateway, pages, progress):
        self.server, self.gateway, self.pages, self.progress = server, gateway, pages, progress
        self.counts = Counter(progress["counts"])
        self.banks = progress["bank_names"]
        self.diag = WORK / "hooks" / "diag.jsonl"
        self.read = self.diag.stat().st_size if self.diag.exists() else 0  # earlier runs' lines are not this store's

    def check(self):
        self.server.check()
        self.gateway.check()

    def begin(self, session):
        session.file = (TRANSCRIPTS / f"{session.sid}.jsonl").open("w")
        if session.bank not in self.progress["banks"]:
            deepen(self.server, self.quiet, session.repo, session.bank)
            self.pages.setup(session.bank, session.times[0])
            self.progress["banks"][session.bank] = {"repo": session.repo, "checkout": self.progress["roots"][session.repo]}
            self.counts["deepen_runs"] += 1

    def quiet(self, bank):
        started = time.monotonic()
        self.server.quiet(bank)
        self.counts["quiet_seconds"] += time.monotonic() - started

    def quiet_all(self):
        for bank in self.progress["banks"]:
            self.quiet(bank)

    def tally(self):
        """What only the server log tells: the facts a chunk lost to a drifted schema (API engine/retain/fact_extraction.py:2440-2460)
        and a sub-chunk that could not be split small enough (:2567-2573). The operation still completes."""
        for line in self.server.logged():
            if "Fact extraction kept" in line:
                self.counts["extraction_partial_chunks"] += 1
            elif "dropping this sub-chunk" in line:
                self.counts["extraction_dropped_chunks"] += 1

    def logged(self):
        """The hook diagnostics' whole lines written since the last call."""
        if not self.diag.exists():
            return []
        with self.diag.open("rb") as f:
            f.seek(self.read)
            data = f.read()
        data = data[: data.rfind(b"\n") + 1]
        self.read += len(data)
        return [json.loads(line) for line in data.decode(errors="replace").splitlines() if line.strip()]

    def stop(self, session, when, reply):
        self.check()
        bank = session.bank
        self.pages.tick(bank, when)
        payload = json.dumps({"session_id": session.sid, "transcript_path": str(TRANSCRIPTS / f"{session.sid}.jsonl"), "cwd": session.cwd, "hook_event_name": "Stop",
                              "stop_hook_active": False, "last_assistant_message": reply}, ensure_ascii=False, separators=(",", ":"))
        started = time.monotonic()
        try:
            r = subprocess.run(["node", f"{PLUGIN}/claude-stop-hook.js"], input=payload, env=hook_env(CONFIGS / f"{session.repo}.json"), cwd=HOME, capture_output=True, text=True,
                               timeout=HOOK_TIMEOUT)
        except subprocess.TimeoutExpired:
            sys.exit(f"hindsight: the Stop hook of {session.name} ran past {HOOK_TIMEOUT} s, the host's timeout")
        self.counts["hook_seconds"] += time.monotonic() - started
        if r.returncode:
            sys.exit(f"hindsight: the Stop hook of {session.name} exited with {r.returncode}: {r.stderr.strip()[-1000:]}")
        self.counts["stops"] += 1
        self.quiet(bank)
        ok = 0
        for line in self.logged():
            if line["event"] == "retain_failed":
                sys.exit(f"hindsight: the Stop hook could not retain {session.name}: {line.get('error')}")
            if line["event"] == "retain_ok" and line.get("session") == session.sid:
                ok += 1
        self.counts["retain_ok"] += ok
        if not ok and session.kept:
            sys.exit(f"hindsight: the Stop hook of {session.name} returned without retaining its {session.kept} kept turns (no retain_ok in {self.diag})")
        self.pages.after_stop(bank, when)


def timeline(plan, done, replay, idle):
    """The sessions in one heap ordered by each session's next entry, so those open at once interleave by real time.
    `idle` is called with each session that ends when no other is open."""
    queue = [(start, n, None, repo, path) for n, (start, repo, path) in enumerate(plan) if f"{repo}/{path.name}" not in done]
    heapq.heapify(queue)
    open_now = 0
    while queue:
        replay.check()
        _, n, session, repo, path = heapq.heappop(queue)
        if session is None:
            session, open_now = Session(repo, path, replay), open_now + 1
        session.step()
        if not session.ended:
            heapq.heappush(queue, (session.next_time(), n, session, repo, path))
            continue
        open_now -= 1
        idle(session, open_now)


def save(server, gateway, pages, replay, progress):
    """Write the store to the checkpoint: the swap is two renames, and a run that finds only checkpoint.old uses it."""
    replay.tally()
    with gateway.lock:
        progress["llm"] = dict(gateway.counts)
    for bank, info in progress["banks"].items():
        s = server.stats(bank)
        info["stats"] = {k: s[k] for k in ("nodes_by_fact_type", "total_documents", "total_links")}
    progress["pages"] = pages.state()
    progress["counts"] = dict(replay.counts)
    tmp, old = WORK / "checkpoint.tmp", WORK / "checkpoint.old"
    shutil.rmtree(tmp, ignore_errors=True)
    tmp.mkdir(parents=True)
    admin("backup", str(tmp / "hindsight.zip"))
    (tmp / "progress.json").write_text(json.dumps(progress, indent=2) + "\n")
    shutil.rmtree(old, ignore_errors=True)
    if CHECKPOINT.exists():
        CHECKPOINT.rename(old)
    tmp.rename(CHECKPOINT)
    shutil.rmtree(old, ignore_errors=True)


def documents(server, bank):
    found, offset = [], 0
    while True:
        page = server.get(f"{bank_path(bank)}/documents?limit=100&offset={offset}")
        found += [d["id"] for d in page["items"]]
        offset += 100
        if offset >= page["total"]:
            return found


def verify_counts(server, progress):
    """A resumed store must hold exactly what the checkpoint recorded: every finished session's document, and each bank's counts."""
    for bank, info in progress["banks"].items():
        s = server.stats(bank)
        found = {k: s[k] for k in ("nodes_by_fact_type", "total_documents", "total_links")}
        if found != info["stats"]:
            sys.exit(f"hindsight: the restored bank {bank} does not match its checkpoint: {found} against {info['stats']}")
    for name, s in progress["sessions"].items():
        if s["stops"] and f"conversation:{s['sid']}" not in documents(server, s["bank"]):
            sys.exit(f"hindsight: the restored store lacks the document of {name}")


def collect(server, progress):
    """What the manifest says about the finished store, read while the server is up."""
    banks = {}
    for bank, info in progress["banks"].items():
        s = server.stats(bank)
        tree = server.get(f"{bank_path(bank)}/knowledge-base/tree")["roots"]
        models = {r["name"]: r for r in sql("SELECT name, length(content) AS chars, max_tokens, last_refreshed_at, last_refresh_failed_at FROM mental_models WHERE bank_id = $1", bank)}
        ops = {}
        for r in sql("SELECT operation_type, status, count(*) AS n FROM async_operations WHERE bank_id = $1 GROUP BY 1, 2 ORDER BY 1, 2", bank):
            ops.setdefault(r["operation_type"], {})[r["status"]] = r["n"]
        usage = {}
        for r in sql("SELECT operation, scope, status, count(*) AS calls, coalesce(sum(input_tokens), 0) AS input_tokens, coalesce(sum(output_tokens), 0) AS output_tokens,"
                     " coalesce(sum(cached_tokens), 0) AS cached_tokens FROM llm_requests WHERE bank_id = $1 GROUP BY 1, 2, 3 ORDER BY 1, 2, 3", bank):
            usage[f"{r['operation']}/{r['scope']}/{r['status']}"] = {k: int(r[k]) for k in ("calls", "input_tokens", "output_tokens", "cached_tokens")}
        page_state = progress["pages"].get(bank, {})
        banks[bank] = {
            "repo": info["repo"], "checkout": info["checkout"],
            "sessions": sum(1 for s_ in progress["sessions"].values() if s_["bank"] == bank),
            "stops": sum(s_["stops"] for s_ in progress["sessions"].values() if s_["bank"] == bank),
            "documents": sorted(documents(server, bank)),
            "nodes_by_fact_type": s["nodes_by_fact_type"], "observations": s["total_observations"], "links": s["total_links"],
            "operations": ops, "failed_operations": s["failed_operations"], "failed_consolidation": s["failed_consolidation"],
            "config": server.get(f"{bank_path(bank)}/config")["overrides"],
            "pages": {n["name"]: {"cron": n["trigger"]["refresh_cron"], "trigger": n["trigger"], "max_tokens": models[n["name"]]["max_tokens"], "source_query": n["description"], "tags": n["tags"], "refreshes": page_state.get(n["name"], {}).get("refreshes", 0),
                                  "last": page_state.get(n["name"], {}).get("last"), "content_chars": models[n["name"]]["chars"] or 0,
                                  "last_refresh_failed_at": str(models[n["name"]]["last_refresh_failed_at"]) if models[n["name"]]["last_refresh_failed_at"] else None}
                      for n in tree if n["kind"] == "page"},
            "llm": usage,
        }
    return {
        "banks": banks,
        "versions": {"hindsight": server.get("/version")["api_version"], "postgres": sql("SELECT version() AS v")[0]["v"],
                     "alembic": sql("SELECT version_num FROM alembic_version")[0]["version_num"]},
    }


def lost_work(progress, found):
    """What would make this store a different thing from the live use it stands for, so it is not packed."""
    lost = []
    for bank, b in found["banks"].items():
        if b["failed_operations"] or b["failed_consolidation"]:
            lost.append(f"{bank}: failed operations {b['failed_operations']}, failed consolidation {b['failed_consolidation']}")
        lost += [f"{bank}: the last refresh of {name} failed" for name, p in b["pages"].items() if p["last_refresh_failed_at"]]
    lost += [f"{name}: no document" for name, s in progress["sessions"].items() if s["stops"] and f"conversation:{s['sid']}" not in found["banks"][s["bank"]]["documents"]]
    return lost


def manifest(progress, skipped, found, tables):
    inputs = (WORK / "key.txt").read_text().splitlines()
    omit = ("_URL", "_KEY")
    config = [{"key": k, "value": v, "cite": cite} for k, (v, cite) in (SERVER_ENV | {f"integration.{k}": (v, c) for k, (v, c) in INTEGRATION.items()}).items() if not k.endswith(omit)]
    config += [{"key": k, "value": v, "default": default, "acts_at": at, "calibrated_for": "BAAI/bge-small-en-v1.5", "matched_by": "quantile over memory pairs", "cite": cite}
               for k, (v, default, at, cite) in THRESHOLDS.items()]
    config.append({"key": "pages", "value": PAGES, "cite": "hourly is live use's cron: the server's cron loop (" + API + "engine/maintenance.py:528-654) keeps running for as long as the server does, and a coding-agents daemon never stops on its own (" + CA + "core/daemon.ts:36-37); run on the replayed clock"})
    llm = {"model": LLM_MODEL} | {k: round(v) if isinstance(v, float) else v for k, v in sorted(progress["llm"].items())}
    by_operation = {}
    for b in found["banks"].values():
        for key, u in b["llm"].items():
            total = by_operation.setdefault(key, Counter())
            total.update(u)
    llm["by_operation"] = {k: dict(v) for k, v in sorted(by_operation.items())}
    pages = [p for b in found["banks"].values() for p in b["pages"].values()]
    return {
        "arm": "hindsight",
        "key": inputs[0],
        "inputs": inputs[1:],
        "device": os.environ["BILBO_EVAL_DEVICE"],
        "versions": found["versions"] | {
            # These three pins repeat the Dockerfile's and compose.yaml's: change them together.
            "image": "ghcr.io/vectorize-io/hindsight-api:0.10.3@sha256:f80f304db313cbc8775acd601fda7d094f5854ebf9bc252d8134eb9276d56b40",
            "pgvector": "pgvector/pgvector:0.8.7-pg18@sha256:2358fcba361ed2233a5ed81b5fe4ca779ccb304120ce531a3bf51c0ed7e2bc11",
            "coding_agents": {"version": "0.8.0", "tarball_sha256": "afc489206154f2e7254057e52b455be8751e8b8bbf5e4a720a104c136fbeed7c", "registry_integrity": "sha512-CViWmh2fnVjhQM8kqOJ2GIGC0kp/Gv4t5X+j+kzpzP+T54PiKQPsrQOunvmluEE3JCGnRQOIcz91Vu/0QUcZow=="},
            "node": subprocess.run(["node", "--version"], capture_output=True, text=True).stdout.strip(),
            "python": platform.python_version(),
            "arch": platform.machine(),
        },
        "config": config,
        "bank_config": {bank: b["config"] for bank, b in found["banks"].items()},
        "pages": {"mode": PAGES, "trigger": "per page, as the server's tree reports it: banks.<bank>.<page>.trigger (the cron is the integration's hashed minute per bank and page)", "banks": {bank: b["pages"] for bank, b in found["banks"].items()},
                  "refreshes": sum(p["refreshes"] for p in pages), "pages_with_content": sum(1 for p in pages if p["content_chars"]), "scope_names_repo": True},
        "embedder": {"provider": "openai", "model": "Qwen3-Embedding-0.6B Q8_0", "dim": 1024, "query_prefix": QUERY_PREFIX},
        "recall": {
            "query_prefix": QUERY_PREFIX,
            "reranker": "local cross-encoder/ms-marco-MiniLM-L-6-v2",
            "text_search": "native",
            "vector_extension": "pgvector",
            "thresholds": {k.removeprefix("HINDSIGHT_API_"): v for k, (v, _, at, _) in THRESHOLDS.items() if at == "recall"},
            "query_timestamp": f"the question's time (recency is measured from it, the wall clock if absent); the corpus ends at {progress['last_session_time']}",
            "last_session_time": progress["last_session_time"],
            "llm": "none",
            "worker": "off",
            "after_restore": "hindsight-admin run-db-migration --embedding-dimension 1024, restore, repair-bank --all",
            "prompt_time": {k: {"value": v, "cite": cite} for k, (v, cite) in PROMPT_TIME.items()},
        },
        "thresholds": {k.removeprefix("HINDSIGHT_API_"): {"value": v, "default": default, "calibrated_for": "BAAI/bge-small-en-v1.5", "matched_by": "quantile"} for k, (v, default, _, _) in THRESHOLDS.items()},
        "banks": {bank: {k: v for k, v in b.items() if k not in ("config", "pages", "llm")} for bank, b in found["banks"].items()},
        "sessions": {"replayed": len(progress["done"]), "skipped": skipped},
        "events": {"stops": progress["counts"].get("stops", 0), "retain_ok": progress["counts"].get("retain_ok", 0), "deepen_runs": progress["counts"].get("deepen_runs", 0),
                   "page_refreshes": sum(p["refreshes"] for p in pages)},
        "replay": {"hook_seconds": round(progress["counts"].get("hook_seconds", 0)), "quiet_seconds": round(progress["counts"].get("quiet_seconds", 0)),
                   "extraction_drops": {"partial_chunks": progress["counts"].get("extraction_partial_chunks", 0), "dropped_chunks": progress["counts"].get("extraction_dropped_chunks", 0)},
                   "skipped_replies": {"api_error": progress["counts"].get("skipped_api_error_replies", 0), "synthetic": progress["counts"].get("skipped_synthetic_replies", 0)}},
        "llm": llm,
        "store": {"file": "hindsight.zip", "tables": tables},
        "notes": NOTES,
        "ingest_seconds": round(progress["ingest_seconds"]),
    }


def interrupted(signum, frame):
    sys.exit("hindsight: stopped; run again to resume from the last checkpoint")


def main():
    if DRY:
        return dry_run()
    signal.signal(signal.SIGINT, interrupted)
    signal.signal(signal.SIGTERM, interrupted)
    try:
        urllib.request.urlopen(urllib.request.Request(f"{LLM_URL}/models", headers={"Authorization": f"Bearer {LLM_KEY}"} | LLM_HEADERS), timeout=30).close()
    except OSError as e:
        sys.exit(f"hindsight: the LLM endpoint {LLM_URL} does not answer: {e}")

    for directory in (HOME, HOOK_TMP, TRANSCRIPTS, CONFIGS, REPOS, WORK / "hooks"):
        directory.mkdir(parents=True, exist_ok=True)
    if not CHECKPOINT.exists() and (WORK / "checkpoint.old").exists():
        (WORK / "checkpoint.old").rename(CHECKPOINT)
    roots = checkout_roots()
    resuming = (CHECKPOINT / "progress.json").exists()
    if resuming:
        progress = json.loads((CHECKPOINT / "progress.json").read_text())
    else:
        progress = {"done": [], "sessions": {}, "banks": {}, "pages": {}, "counts": {}, "llm": {}, "ingest_seconds": 0}
    progress |= {"roots": roots, "bank_names": {repo: bank_of(root) for repo, root in roots.items()}}
    gateway = Gateway(progress["llm"])
    threading.Thread(target=gateway.serve_forever, daemon=True).start()
    server = Server(gateway)
    out = WORK / "store"
    try:
        if resuming:
            restore_database(CHECKPOINT / "hindsight.zip")
        elif sql("SELECT to_regclass('public.banks') AS t")[0]["t"] and sql("SELECT count(*) AS n FROM banks")[0]["n"]:
            sys.exit("hindsight: the database already holds banks from an earlier run: `docker-compose -p bilbo-retrieval-hindsight down -v`, then run again")
        server.start()
        if resuming:
            verify_counts(server, progress)
            log(f"resuming after {len(progress['done'])} sessions")
        write_configs(roots)
        replay = Replay(server, gateway, Pages(server, lambda bank: replay.quiet(bank), PAGES, progress["pages"]), progress)
        pages = replay.pages
        plan, skipped = sessions()
        state = {"started": time.monotonic()}

        def idle(session, open_now):
            progress["done"].append(session.name)
            progress["sessions"][session.name] = {"sid": session.sid, "bank": session.bank, "stops": session.stops, "kept": session.kept}
            progress["last_session_time"] = max(progress.get("last_session_time", ""), iso(session.times[-1]))
            if open_now == 0:
                replay.quiet_all()
                gateway.check()
                progress["ingest_seconds"] += time.monotonic() - state["started"]
                save(server, gateway, pages, replay, progress)
                state["started"] = time.monotonic()
                log(f"{len(progress['done'])}/{len(plan)} sessions: {replay.counts['stops']} Stops, {gateway.counts['calls']} LLM calls so far")

        timeline(plan, set(progress["done"]), replay, idle)
        pages.finish()
        replay.quiet_all()
        gateway.check()
        progress["ingest_seconds"] += time.monotonic() - state["started"]
        save(server, gateway, pages, replay, progress)  # a resume from here runs finish() as a no-op
        found = collect(server, progress)
        shutil.rmtree(out, ignore_errors=True)
        out.mkdir()
        admin("backup", str(out / "hindsight.zip"))
    finally:
        server.stop()
        gateway.shutdown()

    with zipfile.ZipFile(out / "hindsight.zip") as z:
        tables = {name: t["rows"] for name, t in json.loads(z.read("manifest.json"))["tables"].items()}
    done = manifest(progress, skipped, found, tables)
    # A store that lost work is not packed, and carries no `finished`: it would be compared with the other arms as it stands.
    lost = lost_work(progress, found)
    if not lost:
        done["finished"] = datetime.now(timezone.utc).isoformat(timespec="seconds")
    (out / "manifest.json").write_text(json.dumps(done, indent=2) + "\n")
    if lost:
        sys.exit(f"hindsight: the store lost work ({'; '.join(lost)}); see {out / 'manifest.json'}")
    drops = {k: progress["counts"][k] for k in ("extraction_partial_chunks", "extraction_dropped_chunks") if progress["counts"].get(k)}
    log(f"done: {out}" + (f"; Hindsight dropped facts while extracting, see replay.extraction_drops in the manifest: {drops}" if drops else ""))


class Dry:
    """The timeline with no server: what each Stop would send and the page refreshes the hourly rule would fire."""

    def __init__(self, banks):
        self.banks, self.counts, self.state, self.printed = banks, Counter(), {}, []
        self.pages, self.log = DryPages(self), []

    def check(self):
        pass

    def begin(self, session):
        session.file = None
        if session.bank not in self.pages.banks:
            self.pages.setup(session.bank, session.times[0])
            print(f"{iso(session.times[0])} bank {session.bank} set up for {session.name}")

    def stop(self, session, when, reply):
        st = self.state.setdefault(session.sid, {"seen": 0, "lines": [], "done": 0, "stored": [], "layout": set(), "calls": [], "turns": 0})
        for entry in session.entries[st["seen"]:session.written]:
            st["lines"] += rendered(entry)
        st["seen"] = session.written
        new = st["lines"][st["done"]:]
        if not st["calls"]:
            new = ['{"role":"system","content":"REF-ID: conversation:x"}'] + new
        now = (chunks(st["stored"]) if st["stored"] else []) + (chunks(new) if new else [])
        calls = sum(c not in st["layout"] for c in now)
        st["calls"].append(calls)
        st["layout"], st["stored"], st["done"] = set(now), st["stored"] + new, len(st["lines"])
        before = len(self.log)
        self.pages.tick(session.bank, when)
        self.pages.after_stop(session.bank, when)
        print(f"{iso(when)} Stop {session.sid[:8]} #{session.stops} {session.bank} lines {session.written} turns {len(st['lines'])} new {len(new)} extraction chunks {calls}"
              f" cwd {session.cwd}" + "".join(f"\n    refresh {b} / {p} at {iso(due)}" for b, p, due in self.log[before:]))
        self.counts["stops"] += 1


class DryPages(Pages):
    """Pages with no server: every page is stale after every Stop, with the integration's hashed minute for a cron."""

    def __init__(self, dry):
        self.dry, self.mode, self.banks, self.latest = dry, PAGES, {}, {}

    def fire(self, cron, after):
        """Every cron here is `M * * * *`."""
        at = after.replace(minute=int(cron.split()[0]), second=0, microsecond=0)
        return at if at > after else at + timedelta(hours=1)

    def setup(self, bank, start):
        self.banks[bank] = {name: {"id": name, "cron": f"{page_minute(bank, name)} * * * *", "last": start, "stale_since": None, "refreshes": 0} for name in PAGE_NAMES}

    def is_stale(self, bank, page):
        return True

    def refresh(self, bank, name, page, due):
        self.dry.log.append((bank, name, due))
        page.update(last=due + timedelta(seconds=DRY_REFRESH_SECONDS), stale_since=None, refreshes=page["refreshes"] + 1)


def chunks(lines, size=12000):
    """chunk_text on JSONL: whole lines packed up to `size` characters, a longer line cut (API engine/retain/fact_extraction.py:835-870)."""
    out, cur = [], []
    for line in lines:
        if len(line) + 1 > size:
            if cur:
                out.append("\n".join(cur))
                cur = []
            out += [line[i:i + size] for i in range(0, len(line), size)]
            continue
        if cur and sum(len(x) + 1 for x in cur) + len(line) + 1 > size:
            out.append("\n".join(cur))
            cur = []
        cur.append(line)
    return out + (["\n".join(cur)] if cur else [])


def dry_run():
    roots = checkout_roots()
    dry = Dry({repo: bank_of(root) for repo, root in roots.items()})
    plan, skipped = sessions()
    print(f"{len(plan)} Claude Code sessions to replay, {len(skipped)} skipped: {skipped}")
    timeline(plan, set(), dry, lambda session, open_now: print(f"{iso(session.times[-1])} session {session.name} ended after {session.stops} Stops, extraction chunks per Stop {dry.state[session.sid]['calls'] if session.sid in dry.state else []}"))
    before = len(dry.log)
    dry.pages.finish()
    for bank, name, due in dry.log[before:]:
        print(f"end: refresh {bank} / {name} at {iso(due)}")
    refreshes = Counter(b for b, _, _ in dry.log)
    print(f"{len(plan)} sessions, {dry.counts['stops']} Stops, {len(dry.log)} page refreshes if every Stop stales every page: {dict(refreshes)}")


if __name__ == "__main__":
    main()
