"""Build the claude-mem store: replay every session through claude-mem's own Claude Code hook command, hook by hook and
in one timeline across the sessions it replays, into one claude-mem worker on loopback, with its default observer (its
`claude` provider: Claude Code driving BILBO_EVAL_LLM_MODEL on the account in CLAUDE_CODE_OAUTH_TOKEN or
ANTHROPIC_API_KEY) and Chroma embedding through the shared embedder. A Bun preload (clock.js) gives the worker and every hook the time of the
event being replayed, which claude-mem would otherwise stamp with the build's clock. Then save the worker's data
directory (SQLite, Chroma and its sync state) with a manifest in /work/store. Whenever no session is open, the worker is
stopped and its data goes to /work/checkpoint, and a run that finds it resumes from there.
The store is built in lanes (x-lanes in compose.yaml): with BILBO_EVAL_LANE set, this replays only that repository's
sessions, in a build container of its own, into /work/lanes/<lane>, and stops at its last checkpoint; `--merge` then
combines the lanes' checkpoints into one store, and `--lanes` names the lanes. BILBO_EVAL_SESSIONS is a trial: N for the
first N sessions, or `repo:N,repo:N` for the first N sessions of each repository named. `--count-chroma <dir>` and
`--merge-chroma <spec>` are modes of this file for the chroma-mcp environment: they count a Chroma directory, and copy
the lanes' Chroma documents into one."""

import fcntl
import heapq
import json
import mmap
import os
import platform
import re
import shutil
import signal
import sqlite3
import subprocess
import sys
import time
import urllib.request
import uuid
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path

NOTICES = ("<local-command-stdout>", "<local-command-stderr>", "<local-command-caveat>", "<system-reminder>")
CORPUS = Path("/corpus")
LANE = os.environ.get("BILBO_EVAL_LANE", "")  # the repository this container replays, when the store is built in lanes
LANES = Path("/work/lanes")
WORK = LANES / LANE if LANE else Path("/work")
MERGE = sys.argv[1:2] == ["--merge"]
CHECKPOINT = WORK / "checkpoint"
LIVE = Path("/tmp/live")
DATA = LIVE / "data"
CLOCK = LIVE / "clock"
HOME = Path("/tmp/home")
PLUGIN = Path("/opt/claude-mem/plugin")
WORKER = PLUGIN / "scripts" / "worker-service.cjs"
PORT = 37777
IO = WORK / "claude"  # what each Claude Code process claude-mem ran wrote on stdout, copied by claude.sh
# The observer claude-mem 13.35.0 runs with nothing set: its claude provider on CLAUDE_MEM_MODEL's default
# (SettingsDefaultsManager.ts:372, 401), or on CLAUDE_MEM_TIER_SIMPLE_MODEL's `haiku` for a generation that starts on
# read-only tool calls only (:488-489, SessionRoutes.ts:1026-1065), through this Claude Code. BILBO_EVAL_LLM_MODEL names
# the model, so the store's key and lock line do; another model than the default goes into settings.json.
DEFAULT_MODEL = "claude-haiku-4-5-20251001"
LLM_MODEL = os.environ.get("BILBO_EVAL_LLM_MODEL", "")
TIER_SIMPLE = "haiku"
CLAUDE_CODE = "2.1.296"
AUTH = ("CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_API_KEY")
# claude-mem's quota guard stops its observer once a window of a subscription reaches these, or on any window the API
# refused (SettingsDefaultsManager.ts:436-440, RateLimitStore.ts:401-485; overage only while it is in use); the build
# waits for the window's reset before an event once the account is within MARGIN of one, so the guard seldom trips.
QUOTA = {"five_hour": 0.95, "seven_day": 0.93, "seven_day_opus": 0.93, "seven_day_sonnet": 0.92, "overage": 0.95}
MARGIN = 0.02
RESET_SLACK = 120  # seconds waited past a window's reset
THROTTLE_MAX = 900  # the longest wait after a refusal that is not a spent window (a per-minute limit), doubling from 60 s
# claude-mem refreshes these two windows from every reply; any other reading it keeps until the worker restarts, since
# it ages it by the replayed clock (RateLimitStore.ts:131-158, 430).
REFRESHED = ("five_hour", "seven_day")
# After a quota stop, claude-mem sends nothing until its breaker's window passes on its clock, the replay's: 30 minutes,
# or 90 s for a throttle (quota-cooldown.ts:179, 192).
BREAKER_MS = {"rate_limit": 90_000}
BREAKER_DEFAULT_MS = 1_800_000
STUCK = 900  # seconds claude-mem may hold messages with nothing moving and no quota stop before the build fails
QUIET = 5  # seconds with no observer turn open and nothing moving before the build moves the clock for a held observer
TRANSIENT_ERRORS = ("rate_limit", "server_error", "overloaded")  # Claude Code's error kinds that a retry can heal
# How the observer's 3-minute idle wait (SessionMessageBuffer.ts:5) is honoured: "virtual" lets clock.js end it when the
# replay's clock passes 3 minutes; "real" leaves claude-mem's own timer and waits it out in real time wherever the
# replay predicts an exit.
IDLE = "virtual"
IDLE_MS = 180_000
SKIP_TOOLS = (
    "listmcpresourcestool", "slashcommand", "skill", "todowrite", "askuserquestion")  # CLAUDE_MEM_SKIP_TOOLS default
EMBED_CTX = 8192
# Documents per Chroma read: a get of a whole large collection fails with SQLite's "too many SQL variables" (54,548
# documents did).
CHROMA_PAGE = 5000
LAUNCHER = [
    "--python", "3.13", "--with", "onnxruntime>=1.20", "--with", "protobuf<7", "--with", "chromadb==1.5.9", "--with",
    "pydantic<2.14", "--from", "chroma-mcp==0.2.6"]
BASE_IMAGE = "ghcr.io/astral-sh/uv:0.12.19-python3.12-trixie-slim@sha256:10b94fe3550b7a96e3982860bb82cdefba6300264a7a40bb9334acf21a450974"
NPM_SHA256 = "a5d7b646541e1a839572095a31682b0e121929c1b3c6e80e29b27eb0a702a67a"
BUN_SHA256 = {
    "x86_64": "36368faef7527875d5ffa52e53cd48021741f2a83eb6208a8dd64068d422a913",
    "aarch64": "54328bbc2d9c8e0c9f892c544d66c57a83b84139e34909e5ee81758f1ac8fda7"}
EMBEDDER_URL = os.environ.get("EMBEDDER_URL", "")

# What differs from a fresh install, each with where claude-mem 13.35.0's docs or source ask for it (paths in its repo).
# settings.json travels with the store.
SETTINGS = {
    "CLAUDE_MEM_CHROMA_EMBEDDING_FUNCTION": (
        "openai",
        "src/shared/SettingsDefaultsManager.ts:504-510: the shared embedder is served on an OpenAI-compatible endpoint, which chroma-mcp's openai function reads from OPENAI_BASE_URL"),
    "CLAUDE_MEM_WORKER_AUTOSTART": (
        "false",
        "docs/public/configuration.mdx:29: false when you run the worker yourself (a container, `worker-service.cjs --daemon`)"),
    "CLAUDE_MEM_WORKER_PORT": (str(PORT), "docs/public/configuration.mdx:27: fixed instead of 37700 + uid % 100"),
} | ({"CLAUDE_MEM_MODEL": (LLM_MODEL, "docs/public/configuration.mdx:16: the observer's model when it is not the default, BILBO_EVAL_LLM_MODEL")}
     if LLM_MODEL != DEFAULT_MODEL else {})
# What else is set around claude-mem, outside settings.json. The worker's environment reaches Claude Code but for
# CLAUDE_CODE_* and the Anthropic credentials (src/supervisor/env-sanitizer.ts, src/shared/EnvManager.ts:54-81).
ENV = {
    "CLAUDE_MEM_TELEMETRY": (
        "0",
        "docs/public/telemetry.mdx:236: claude-mem's own switch; DO_NOT_TRACK, its other one, would reach Claude Code, which then fetches none of its feature flags"),
    "DISABLE_AUTOUPDATER": (
        "1",
        "keeps the pinned Claude Code: an update lands in ~/.local/bin/claude, which claude-mem probes and prefers when newer (src/shared/find-claude-executable.ts:312, 522)"),
    "BILBO_CLAUDE_IO": (
        str(LANES / "<lane>" / "claude") if MERGE else str(IO),
        "claude.sh copies each Claude Code process's stdout there, which the LLM usage is read from"),
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
# GeneratorExitHandler.ts:129, hook-spool.ts:422), or that its observer cannot run until someone acts
# (ResponseProcessor.ts:475, GeneratorRunner.ts:196, dependency-preflight.ts:178, SessionRoutes.ts:340): any fails the
# build.
LOSS_LINES = (
    "Batch parked: its paid-send budget is spent", "Session ended with parked batches",
    "Dropping the buffered summarize after its one rescue pass",
    "Hook spool entry was never accepted within its retry window", "authentication failed; run /login",
    "Claude generator start requires setup", "Claude CLI dependency preflight failed",
    "Claude setup dependency still unavailable")
# claude-mem drops a batch after a second reply that is not XML (ResponseProcessor.ts:564): the tool and the model
# behaving as they would live, so it is listed in the manifest, not fatal. So is a refusal the API repeats, such as a
# 400, which Claude Code hands claude-mem as `API Error: 400 ...`; a usage limit, a 5xx and an overload keep their batch.
DROPPED_LINE = "response again for the same queued batch — dropping it"
# Counted in the manifest, not fatal.
COUNTED_LINES = {
    "compression_unusable": "Oversized field compression unusable",
    "compression_failed": "Oversized field compression failed",
    "oversized_field": "Oversized field too large to condense; falling back to truncation",
    "non_xml": "returned non-XML",
    # The observer's pauses (agents/ResponseProcessor.ts, ClaudeProvider.ts, http/routes/SessionRoutes.ts,
    # SessionManager.ts, supervisor/process-registry.ts).
    "quota_guard_stops": "Aborting session for quota guard", "quota_prose": "SDK returned quota-limit prose",
    "breaker_refusals": "Skipping generator start while the provider cooldown is active",
    "transport_pauses": "Transport pause: scheduled buffered-work resume",
    "response_stalls": "Observer prompt went unanswered", "slot_waits": "Pool limit reached"}
NOTES = {
    "hooks_not_replayed": "SessionStart and PreToolUse only read what claude-mem would show the live agent (GET /api/context/inject, /api/observations/by-file), and Setup installs dependencies, which the image does; none reaches the store",
    "serial": "each event waits until claude-mem has stored what it gave the observer before the clock moves, so no LLM call is in flight across a clock jump",
    "search": "/api/search drops Chroma hits older than 90 days unless dateStart is sent (SearchManager.ts:121,431, ChromaSearchStrategy.ts:196) and falls back to SQLite FTS5 when Chroma finds nothing (SearchOrchestrator.ts:135-174); this store's rows are dated by the replayed sessions, so the run step should send dateStart",
    "serve": "copy the store to a writable directory, set CLAUDE_MEM_DATA_DIR, OPENAI_BASE_URL and OPENAI_API_KEY, and start `bun worker-service.cjs --daemon`; no LLM is needed to search",
    "quiet_points": "the worker is stopped at each one, which finalizes open generators",
    "llm": "what Claude Code reported for the calls behind this store, the calls of work an interrupt threw away not counted: calls are the model replies by the model each named, tokens are from its result lines (modelUsage); unreported are replies whose process was stopped before its result line (claude-mem's quota guard does that), their output not known; windows are the account's utilization at the first and last reply that reported it, which counts everything else the account did meanwhile",
    "clock_claude_code": "Claude Code keeps real time: the model is told the build's date, not the session's, while claude-mem's prompts carry each event's own time",
    "quota_guard_clock": "claude-mem's quota guard compares Anthropic's reset times with the replayed clock: its 15-minute grace before a five-hour reset never fires, and a reading of a window other than the five-hour and weekly ones lapses only when the worker restarts (RateLimitStore.ts:430, 467-481); the build waits before the guard's thresholds, so the guard seldom trips",
    "tier_routing": "claude-mem runs a generation that starts on Read, Glob, Grep or LS calls on its simple tier, the `haiku` alias (SessionRoutes.ts:1022-1065), which this Claude Code resolves to the model `llm.models` lists beside the default",
    "projects": "a worktree's sessions are filed under <repo>/<worktree> keys (drain-cycle/ABA-301, ...) and stay there, since claude-mem adopts a worktree's rows into its repo only when the checkout is gone or merged (WorktreeAdoption.ts:470-560): a per-repo search must pass every key `projects` lists for the repo",
    "queries": "a search embeds the query like a document: the openai embedding function takes no query instruction, which Qwen3 would take (ai-memory sends one)",
    "image_only_prompts": "sent with an empty prompt as Claude Code sends them, and stored by claude-mem as [media prompt]; the mem0 and ai-memory arms skip them",
    "lanes": "each repository's sessions are replayed as a lane of their own, in a build container of their own with its own worker, clock, quota guard and checkpoints, the lanes at once, or as many at a time as the build runs, against the one embedder; a lane applies one worker's rules to its repository's sessions, and claude-mem keeps its content by project, no project spanning two repositories; it differs from one worker only in what that worker shares across repositories: the observer's two process slots and its quota breaker, the next event that bounds a clock step, and the quiet points where the worker stops, so a session whose observer a lane's quiet point shuts down, where one worker would idle it out while another repository's session goes on, is completed up to 3 minutes earlier; the merge renumbers the rows in time order across the lanes, each lane's own order kept, remaps the ids rows and the sync state hold, rebuilds the full-text index, copies Chroma's vectors under the new ids, and checks every row and document against its lane's, a vector to within 1e-6 in each component, since hnswlib normalizes a copied cosine vector again and float32 rounding moves it by about 1e-8; counts, LLM usage, hook_seconds and ingest_seconds add up the lanes, and `lanes` lists each one's"}
HOOK_ENV = {
    "HOME": str(HOME), "PATH": "/usr/local/bin:/usr/bin:/bin", "CLAUDE_MEM_DATA_DIR": str(DATA),
    "CLAUDE_MEM_WORKER_PORT": str(PORT), "BUN_OPTIONS": ENV["BUN_OPTIONS"][0], "BILBO_CLOCK_FILE": str(CLOCK),
    "CLAUDE_MEM_TELEMETRY": ENV["CLAUDE_MEM_TELEMETRY"][0]}
WORKER_ENV = (
    HOOK_ENV
    | {k: ENV[k][0] for k in ("OPENAI_BASE_URL", "OPENAI_API_KEY", "DISABLE_AUTOUPDATER", "BILBO_CLAUDE_IO")}
    | {k: os.environ[k] for k in os.environ if k.startswith("UV_")}
    | ({"BILBO_VIRTUAL_IDLE": "1"} if IDLE == "virtual" else {})
    | ({"CLAUDE_CODE_OAUTH_TOKEN": os.environ["CLAUDE_CODE_OAUTH_TOKEN"]} if os.environ.get("CLAUDE_CODE_OAUTH_TOKEN") else {})
)
# How each credential reaches Claude Code: claude-mem drops both from the worker's environment for the child it spawns.
AUTH_CONFIG = {
    "CLAUDE_CODE_OAUTH_TOKEN": (
        "env",
        "src/shared/oauth-token.ts:543-566, 625-638: with no credential store, claude-mem reads the worker's token at each spawn and hands it to Claude Code, as it would a login from the store; its quota guard applies"),
    "ANTHROPIC_API_KEY": (
        "<data dir>/.env",
        "src/shared/EnvManager.ts:106-116, 321: the only way an API key reaches Claude Code; claude-mem's quota guard then never stops the observer (RateLimitStore.ts isApiKeyAuth), and the .env never goes into a checkpoint or the store"),
}
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
        for entry in own_entries(path):
            if entry.get("uuid") in seen:  # a line Claude Code wrote again fires no hook
                continue
            if entry.get("uuid"):
                seen.add(entry["uuid"])
            t = stamp(entry)
            start = t if t and (start is None or t < start) else start
        (found if start else other).append((start, path.parent.name, path))
    found.sort()
    limit = os.environ.get("BILBO_EVAL_SESSIONS") or ""
    if ":" in limit:  # repo:N,repo:N: the first N sessions of each repository named
        per = {repo: int(n) for repo, n in (part.split(":") for part in limit.split(","))}
        if unknown := set(per) - {repo for _, repo, _ in found}:
            sys.exit(f"claude-mem: BILBO_EVAL_SESSIONS names repositories the corpus lacks: {sorted(unknown)}")
        taken, kept = Counter(), []
        for start, repo, path in found:
            if taken[repo] < per.get(repo, 0):
                taken[repo] += 1
                kept.append((start, repo, path))
        found = kept
    elif limit:
        found = found[: int(limit)]
    return found, [f"{repo}/{path.name}" for _, repo, path in other]


def posix_path(value):
    """A Windows path at the POSIX form of its path: C:\\Users\\a\\b as /Users/a/b."""
    return "/" + value[3:].replace("\\", "/") if isinstance(value, str) and re.match(r"[A-Za-z]:\\", value) else value


def posix_input(tool_input):
    """A tool's input with its file and search paths at the POSIX form; the transcript line keeps what was recorded."""
    return {k: posix_path(v) if k in ("file_path", "notebook_path", "path") else v for k, v in tool_input.items()}


def session_id(path):
    """A transcript's own session id, from its name <time>--<id>.jsonl."""
    return path.stem.rsplit("--", 1)[-1]


def imported(entry, sid):
    """Whether a record is another session's: one continued from another machine holds the earlier one's history."""
    return entry.get("sessionId") not in (None, sid)


def own_entries(path):
    """A transcript's parsed records, but those of another session."""
    sid = session_id(path)
    return (e for e in map(json.loads, path.open()) if not imported(e, sid))


def checkout_roots():
    """Each repository's main checkout: the directory most of its sessions start in."""
    starts = {}
    for path in sorted(CORPUS.glob("*/*.jsonl")):
        cwd = next((posix_path(e["cwd"]) for e in own_entries(path) if e.get("cwd")), None)
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


def bump(counter, key, n=1):
    counter[key] = counter.get(key, 0) + n


def resets_at(value):
    """A reset time as epoch seconds: Claude Code reports seconds, its SDK documents milliseconds."""
    return value / 1000 if isinstance(value, (int, float)) and value >= 1e12 else value


def quota_hold():
    """claude-mem's breaker for its claude provider while one is armed, as quota-cooldown.ts persists it: the window it
    stopped for, when (armedAtMs, on the replay's clock) and, for a refused credential, cause `auth`."""
    try:
        rows = json.loads((DATA / "quota-cooldown.json").read_text())
    except (OSError, ValueError):
        return None
    return next((r for r in rows if isinstance(r, dict) and r.get("provider") == "claude"), None)


def breaker_end(hold):
    """When claude-mem's breaker lets a probe through, on the replay's clock (resolveQuotaCooldownMs)."""
    return hold["armedAtMs"] + BREAKER_MS.get(hold.get("window"), BREAKER_DEFAULT_MS)


class Claude:
    """What each Claude Code process claude-mem ran reported on stdout, read from the copies claude.sh keeps in IO: calls
    and tokens by model, replies by session, open turns, the API's refusals and retries, and the account's limits. A
    process's file starts with claude.sh's own line: its pid, its working directory, observer-sessions/<session db id>
    for an observer and that directory itself for a one-off call (a field condense), and its --model."""

    FIELDS = (
        ("inputTokens", "input_tokens"), ("cacheCreationInputTokens", "cache_creation_input_tokens"),
        ("cacheReadInputTokens", "cache_read_input_tokens"), ("outputTokens", "output_tokens"), ("costUSD", "cost_usd"))

    def __init__(self, counts):
        self.counts = counts  # kept in progress["llm"] at each checkpoint
        for name in ("by_model", "turns", "error_turns", "processes", "observer_model_args", "api_retries", "api_errors",
                     "unreported", "windows"):
            self.counts.setdefault(name, {})
        # A rerun finds the files of the run it resumes: what they hold after the checkpoint was thrown away with
        # that work, and what they hold before it is in counts already, so none is read again.
        self.files = {p.name: {"offset": p.stat().st_size, "done": True} for p in IO.glob("*.jsonl")}
        self.windows = {}  # window -> (utilization, reset in epoch s), as Claude Code last reported it
        self.overage_in_use = False
        self.refused = None  # the reset of a window the API refused once it was spent
        self.throttled, self.throttle_streak = False, 0  # a refusal of a window that was not spent: a per-minute limit
        self.replies = 0  # successful observer replies read
        self.by_session = Counter()  # successful observer replies read, by session db id
        self.size = 0
        self.errors = {}  # session db id -> the API error its observer's latest reply was, or None for a model reply

    def pump(self):
        for path in sorted(IO.glob("*.jsonl")):
            state = self.files.setdefault(path.name, {"offset": 0})
            if state.get("done"):
                continue
            with path.open("rb") as f:
                f.seek(state["offset"])
                data = f.read()
            data = data[: data.rfind(b"\n") + 1]
            state["offset"] += len(data)
            self.size += len(data)
            for line in data.decode(errors="replace").split("\n"):
                try:
                    entry = json.loads(line)
                except ValueError:
                    continue
                if isinstance(entry, dict):
                    self.read(state, entry)
            if not data and state.get("pid") and not alive(state["pid"]) and path.stat().st_size == state["offset"]:
                state["done"] = True  # its process has ended and every line of it is read

    def read(self, state, entry):
        c, kind = self.counts, entry.get("type")
        if kind == "bilbo_process":
            name = Path(entry.get("cwd") or "").name
            state.update(session=int(name) if name.isdigit() else None, usage={}, pending={}, seen=set(),
                         pid=entry.get("pid"), open=False)
            bump(c["processes"], "observer" if state["session"] is not None else "one_off")
            if state["session"] is not None:
                bump(c["observer_model_args"], entry.get("model") or "")
            return
        if "pending" not in state:
            return
        role = "observer" if state["session"] is not None else "one_off"
        if kind == "system" and entry.get("subtype") == "init":
            state["open"] = True  # Claude Code starts each turn with an init line and ends it with a result line
        elif kind == "assistant":
            message = entry.get("message") or {}
            if entry.get("error") or message.get("model") == "<synthetic>":
                bump(c["api_errors"], entry.get("error") or "unknown")
                if state["session"] is not None:
                    self.errors[state["session"]] = text_of(message.get("content"))[:300]
            elif message.get("id") not in state["seen"]:  # a reply split into frames repeats its id
                state["seen"].add(message.get("id"))
                state["pending"][message.get("id")] = message.get("usage") or {}
                bump(c["by_model"].setdefault(message.get("model") or "", {}), "calls")
                self.throttle_streak = 0
                if state["session"] is not None:
                    self.replies += 1
                    self.by_session[state["session"]] += 1
                    self.errors[state["session"]] = None
        elif kind == "result":
            totals = entry.get("modelUsage") or {}  # this process's totals so far
            for name, now in totals.items():
                before = state["usage"].get(name) or {}
                model = c["by_model"].setdefault(name, {})
                for src, dst in self.FIELDS:
                    bump(model, dst, (now.get(src) or 0) - (before.get(src) or 0))
            state["usage"] = totals or state["usage"]
            state["pending"], state["open"] = {}, False
            bump(c["error_turns" if entry.get("is_error") else "turns"], role)
        elif kind == "rate_limit_event":
            info = entry.get("rate_limit_info") or {}
            c["rate_limit_events"] = c.get("rate_limit_events", 0) + 1
            reported = {
                w: (s.get("utilization"), resets_at(s.get("resetsAt")))
                for w, s in (info.get("unifiedWindows") or {}).items()
                if isinstance(s, dict)
            }
            claim = info.get("rateLimitType")
            if claim and info.get("utilization") is not None:
                reported.setdefault(claim, (info["utilization"], resets_at(info.get("resetsAt"))))
            self.overage_in_use = bool(info.get("isUsingOverage"))
            seen = datetime.now(timezone.utc).isoformat(timespec="seconds")
            for window, (used, reset) in reported.items():
                self.windows[window] = (used, reset)
                if isinstance(used, (int, float)):
                    snap = {"utilization": used, "resets_at": iso(reset * 1000) if reset else None, "seen": seen}
                    w = c["windows"].setdefault(window, {})
                    w.setdefault("first", snap)
                    w["last"], w["max"] = snap, max(w.get("max", 0), used)
            if info.get("status") == "rejected":
                used = (reported.get(claim) or (None, None))[0]
                if isinstance(used, (int, float)) and used >= QUOTA.get(claim, 1.0) - MARGIN:
                    c["refusals"] = c.get("refusals", 0) + 1
                    self.refused = resets_at(info.get("resetsAt"))
                else:  # Claude Code reports a per-minute limit as a refusal of the five-hour window (probe out3)
                    c["throttles"] = c.get("throttles", 0) + 1
                    self.throttled, self.throttle_streak = True, self.throttle_streak + 1
        elif kind == "system" and entry.get("subtype") == "api_retry":
            bump(c["api_retries"], str(entry.get("error_status")))

    def open_turns(self):
        """Claude Code processes with a turn begun and not ended, whose process is still alive."""
        return sum(1 for state in self.files.values() if state.get("open") and alive(state.get("pid")))

    def settle(self):
        """Once the worker has stopped, so every process has ended: a reply whose result line never came was cut off
        (claude-mem's quota guard kills its process), and only its input is known."""
        self.pump()
        for state in self.files.values():
            for usage in (state.get("pending") or {}).values():
                bump(self.counts["unreported"], "replies")
                for k in ("input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"):
                    bump(self.counts["unreported"], k, usage.get(k) or 0)
            state["pending"], state["open"] = {}, False

    def near_limit(self):
        """(window, its reset) for a window within MARGIN of the guard's threshold, or spent and refused, that has not
        reset."""
        now = time.time()
        for window, threshold in QUOTA.items():
            if window == "overage" and not self.overage_in_use:
                continue
            used, reset = self.windows.get(window, (None, None))
            if isinstance(used, (int, float)) and used >= threshold - MARGIN and reset and reset > now:
                return window, reset
        if self.refused and self.refused > now:
            return "refused", self.refused
        return None

    def models(self):
        return {m for m, v in self.counts["by_model"].items() if m and (v.get("calls") or v.get("output_tokens"))}


def alive(pid):
    """Whether a process of this container is still running: claude.sh's pid is Claude Code's, which it execs."""
    if not isinstance(pid, int):
        return False
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    try:  # a process that has ended but not been reaped yet is not running
        return Path(f"/proc/{pid}/stat").read_text().split(") ")[-1][:1] != "Z"
    except OSError:
        return False


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
        self.waiting_until = 0.0  # monotonic time until which claude-mem said it waits on purpose (a transport backoff)
        self.overflow_until = {}  # session db id -> the replayed time its overflow cooldown ends (SessionRoutes.ts:266)
        self.clock = None  # the replay's Clock, which dates the overflow cooldown
        self.counted, self.errors, self.rejected, self.dropped = Counter(counts), [], {}, []

    def start(self, strict=True):
        self.backfilled = False
        if not (DATA / "settings.json").exists():
            DATA.mkdir(parents=True, exist_ok=True)
            (DATA / "settings.json").write_text(json.dumps({k: v for k, (v, _) in SETTINGS.items()}, indent=2) + "\n")
        if os.environ.get("ANTHROPIC_API_KEY"):  # written at every start: copy_store leaves it out of checkpoints
            env = DATA / ".env"
            env.touch(mode=0o600)
            env.write_text(f"ANTHROPIC_API_KEY={os.environ['ANTHROPIC_API_KEY']}\n")
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
        ai = api("/api/health")["ai"]
        want = "API key" if os.environ.get("ANTHROPIC_API_KEY") else "Claude Code OAuth token (env"
        if ai["provider"] != "claude" or not ai.get("authMethod", "").startswith(want):
            sys.exit(f"claude-mem: the worker's observer is {ai['provider']} on {ai.get('authMethod')}, not claude on {want}")
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
        for name, pattern in (("chroma-mcp", "chroma-mcp"), ("Claude Code", "/opt/claude-code/claude")):
            left = subprocess.run(["pgrep", "-f", pattern], capture_output=True, text=True).stdout.split()
            if left:
                subprocess.run(["pkill", "-9", "-f", pattern])
                sys.exit(
                    f"claude-mem: {name} outlived the worker ({' '.join(left)}); run again to resume from the last checkpoint")
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
            for line in data.decode(errors="replace").split("\n"):
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
        backoff = re.search(r"Transport pause: scheduled buffered-work resume.*?delayMs=(\d+)", line)
        cooling = re.search(r"overflow cooldown is active \{sessionId=(\d+),.*?retryInMs=(\d+)", line)
        if cooling and self.clock:
            self.overflow_until[int(cooling.group(1))] = self.clock.value + int(cooling.group(2))
        if backoff:
            self.waiting_until = max(self.waiting_until, time.monotonic() + int(backoff.group(1)) / 1000)
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
        """How the observer of this session ended within the seconds given: "idle" when it exited because it was idle,
        "gone" when its generator paused or exited another way, None when it did not end."""
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
                return "idle"
            if db_id in self.gone:
                self.gone.discard(db_id)
                return "gone"
            time.sleep(0.1)
        return None


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
        self.sid = session_id(path)
        seen = set()
        self.copied = {k for k, e in enumerate(self.entries) if imported(e, self.sid)}
        recorded = next(
            (e["cwd"] for k, e in enumerate(self.entries) if k not in self.copied and e.get("cwd")),
            "/")  # the first entries often carry none
        self.launch = posix_path(recorded)
        self.cwd = self.launch
        self.transcript = HOME / ".claude" / "projects" / re.sub("[^A-Za-z0-9]", "-", recorded) / f"{self.sid}.jsonl"
        self.transcript.parent.mkdir(parents=True, exist_ok=True)
        self.out = self.transcript.open("w")
        self.turn, self.tools, self.index, self.ended = None, {}, -1, False
        self.prompt_id, self.mode = None, "default"
        self.expected = set()  # the tool_use_ids claude-mem should keep in tool_uses
        self.sent = Counter()
        # Claude Code sometimes writes a line after lines with later timestamps, and one session re-writes its earlier
        # history. A line whose uuid appeared before, or that is another session's (one continued from another machine
        # holds the earlier one's), is written to the transcript and fires no hook. A session is placed
        # in the timeline by the earliest timestamp among the lines still to come that replay, so a late-written line is
        # delivered at its own time, before other sessions move the clock past it.
        for k, entry in enumerate(self.entries):
            if k in self.copied:
                continue
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
        self.cwd = posix_path(entry.get("cwd")) or self.cwd  # Claude Code sends the cwd the session is in at each hook
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
                    "tool_name": name, "tool_input": posix_input(tool_input), "tool_use_id": b.get("tool_use_id"),
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
    """The clock, the worker and Claude Code's reports, and what the replay needs of them: each event moves the clock to
    its time, runs its hook, and waits until claude-mem has stored what the event gave the observer, so every LLM call
    starts at its event's time and none is in flight across a clock jump."""

    def __init__(self, clock, worker, claude, checkouts, counts, idle, rejected):
        self.clock, self.worker, self.claude, self.checkouts, self.counts, self.idle = (
            clock, worker, claude, checkouts, counts, idle)
        self.alive, self.db_ids = {}, {}  # session id → the clock when its observer last took a message
        self.noted = Counter(claude.by_session)  # observer replies by session db id already turned into self.alive
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
        The clock never moves while an observer turn is open, and an exit that frees a process slot for a parked observer
        lets that observer work before the clock moves on."""
        self.quiesce()
        self.worker.pump()
        for sid in [s for s in self.alive if self.db_id(s) in self.worker.gone]:
            del self.alive[sid]
            self.uncertain.discard(sid)
        for sid, due in sorted(((s, last + IDLE_MS) for s, last in self.alive.items()), key=lambda x: x[1]):
            if due > t:
                break
            parked = self.worker.status().get("parkedSessions", 0)
            self.clock.set(due)
            self.idle["predicted"] += 1
            exit_ = self.worker.wait_idle(self.db_id(sid), 15 if IDLE == "virtual" else 200)
            if exit_ == "idle":
                self.idle["observed"] += 1
            elif exit_ == "gone" or sid in self.uncertain:
                # its generator paused or exited another way, during a drain or now: the exit may never come
                self.idle["uncertain_missed"] += 1
            elif IDLE == "virtual":
                sys.exit(
                    f"claude-mem: the observer of session {sid} did not exit idle at {iso(due)}, so the replay's clock model is wrong; "
                    "run again to resume from the last checkpoint"
                )
            else:
                self.idle["missed"] += 1
            del self.alive[sid]
            self.uncertain.discard(sid)
            if parked:
                self.wait_drain(due)
        self.clock.set(t)

    def deliver(self, session, name, payload, when, message, more=False):
        # The clock first, so the observers whose idle exits come before this event end before a long account wait
        # (an open Claude Code process waits it out otherwise).
        self.advance(millis(when))
        self.wait_for_account()
        self.checkouts.ensure(session.cwd)
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
        before = self.noted[db_id]
        # Another hook of this step follows at the same time, so the clock may not step: it would stamp that hook late.
        drained = self.wait_drain(millis(when) if more else self.horizon)
        if drained and self.noted[db_id] == before:
            # claude.sh's copy of a reply can land a moment after claude-mem has stored it
            until = time.monotonic() + 2
            while self.claude.by_session[db_id] == before and time.monotonic() < until:
                time.sleep(0.1)
                self.claude.pump()
        self.note_replies()
        if not drained:
            self.alive.pop(session.sid, None)  # its generator is held: no idle exit is coming
            self.uncertain.discard(session.sid)

    def note_replies(self):
        """Each session whose observer replied since the last look took a message at the clock now: claude-mem's claim
        saw it, and clock.js arms the observer's idle timer from it."""
        self.claude.pump()
        self.worker.pump()
        for db_id, n in list(self.claude.by_session.items()):
            if n == self.noted[db_id]:
                continue
            self.noted[db_id] = n
            sid = self.sid_of(db_id)
            if sid is None:
                continue
            self.alive[sid] = self.clock.value
            (self.uncertain.add if db_id in self.worker.gone else self.uncertain.discard)(sid)

    def sid_of(self, db_id):
        sid = next((s for s, d in self.db_ids.items() if d == db_id), None)
        if sid is None:
            rows = query("SELECT content_session_id FROM sdk_sessions WHERE id = ?", db_id)
            if rows:
                sid = rows[0][0]
                self.db_ids[sid] = db_id
        return sid

    def quiesce(self):
        """Wait until no observer turn is open, so no LLM call is in flight when the clock moves."""
        until = time.monotonic() + STUCK
        while self.claude.open_turns():
            self.worker.check()
            if time.monotonic() > max(until, self.worker.waiting_until):
                sys.exit(f"claude-mem: an observer turn stayed open for {STUCK} s; run again to resume from the last checkpoint")
            time.sleep(0.1)
            self.claude.pump()

    def wait_spool(self):
        until = time.monotonic() + 120
        while spooled():
            self.worker.check()
            if time.monotonic() > until:
                sys.exit("claude-mem: a hook event stayed in the spool for 2 minutes")
            time.sleep(0.05)

    def wait_for_account(self):
        """Wait out a window of the account that is within MARGIN of claude-mem's quota guard, or that the API refused
        once spent, as Claude Code last reported it: the replay pauses for a usage limit, as the other arms' builds do,
        and claude-mem's guard seldom trips. A refusal of a window that was not spent (a per-minute limit) is waited out
        for 60 s, doubling with each one in a row, up to THROTTLE_MAX."""
        self.claude.pump()
        if self.claude.throttled:
            wait = min(60 * 2 ** (max(self.claude.throttle_streak, 1) - 1), THROTTLE_MAX)
            self.counts["throttle_waits"] += 1
            self.counts["throttle_wait_seconds"] += wait
            log(f"the API refused a call with no window spent (a per-minute limit): waiting {wait} s")
            self.sleep(wait)
            self.claude.throttled = False
        near = self.claude.near_limit()
        if not near:
            return
        window, reset = near
        wait = max(0.0, reset - time.time()) + RESET_SLACK
        used = (self.claude.windows.get(window) or (None, None))[0]
        self.counts["limit_waits"] += 1
        self.counts["limit_wait_seconds"] += wait
        log(f"the account's {window} window is at {used}: waiting {round(wait)} s for its reset at {iso(reset * 1000)}")
        self.sleep(wait, f"still waiting for the {window} window's reset at {iso(reset * 1000)}")
        if window == "refused":
            self.claude.refused = None

    def sleep(self, seconds, note=None):
        """Sleep in real time, the worker watched, with a line every 30 minutes for whoever watches the log."""
        until, told = time.monotonic() + seconds, time.monotonic()
        while time.monotonic() < until:
            self.worker.check()
            time.sleep(min(30.0, max(0.0, until - time.monotonic())))
            if note and time.monotonic() - told > 1800:
                log(note)
                told = time.monotonic()

    def wait_drain(self, horizon):
        """Wait until claude-mem has stored what the last event gave the observer: no message queued, none parked.
        Once nothing moves and no observer turn is open, a held observer is helped along, never past the next event of
        the timeline: claude-mem's quota breaker holds it, so the account is waited out in real time and the clock
        steps to the breaker's end, since the breaker runs on the replayed clock; an observer waits for one of
        claude-mem's process slots (two by default), so the clock steps to the latest idle exit due, which frees one; or
        claude-mem's overflow cooldown holds it, and the clock steps to the cooldown's end. False when the clock cannot
        step: the replay then goes on with those messages queued, and the manifest counts it. A breaker on a window
        claude-mem never refreshes, or for a refused credential, fails the build: a rerun's new worker forgets the
        stale reading. Anything else that moves nothing for STUCK seconds, past a backoff claude-mem announced, fails
        the build too."""
        told = moved = time.monotonic()
        last = None
        while True:
            self.worker.check()
            self.worker.pump()
            self.claude.pump()
            self.note_replies()
            status = self.worker.status()
            depth, parked = status["queueDepth"], status.get("parkedSessions", 0)
            if depth == 0 and parked == 0:
                return True
            mark = (depth, parked, self.claude.size, sum(self.worker.offsets.values()))
            if mark != last:
                last, moved = mark, time.monotonic()
            quiet = not self.claude.open_turns() and time.monotonic() - moved > QUIET
            room = horizon - 1 - self.clock.value
            hold = quota_hold()
            if hold and breaker_end(hold) > self.clock.value:
                if hold.get("cause") == "auth":
                    sys.exit("claude-mem: claude-mem's breaker holds the observer for a refused credential")
                self.wait_for_account()
                if hold.get("window") not in (*REFRESHED, "observer_text", "rate_limit", "unknown", None):
                    sys.exit(
                        f"claude-mem: claude-mem's quota guard stopped the observer on its {hold['window']} reading, which it "
                        "keeps until the worker restarts; run again to resume from the last checkpoint")
                if not quiet:
                    time.sleep(0.1)
                    continue
                if breaker_end(hold) - self.clock.value > room:
                    self.counts["breaker_holds"] += 1
                    log(
                        f"claude-mem's quota breaker holds {depth} messages and the clock cannot reach its end before the next event: the replay goes on without them")
                    return False
                self.counts["breaker_steps"] += 1
                self.advance(breaker_end(hold))
                last, moved = None, time.monotonic()  # its sweep lets a probe through within 60 s
                continue
            cooling = [t for t in self.worker.overflow_until.values() if t > self.clock.value]
            if quiet and (parked or cooling):
                # The latest due exit frees every slot an idle observer holds: each took its last message by now.
                target = min(cooling) if cooling and not parked else self.clock.value + IDLE_MS
                kind = "slot" if parked else "overflow"
                if target > horizon - 1:
                    self.counts[f"{kind}_holds"] += 1
                    log(f"{depth} messages wait for claude-mem's observer {kind} and the clock cannot reach it before the next event: the replay goes on without them")
                    return False
                self.counts[f"{kind}_steps"] += 1
                self.advance(target)
                last, moved = None, time.monotonic()
                continue
            if time.monotonic() - max(moved, self.worker.waiting_until) > STUCK:
                sys.exit(
                    f"claude-mem: {depth} messages ({parked} sessions parked) waited {STUCK} s for the observer with nothing moving; "
                    f"latest observer errors {dict(list(self.claude.errors.items())[-3:])}; run again to resume from the last checkpoint")
            if time.monotonic() - told > 300:
                log(f"{depth} messages wait for the observer")
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
        self.check_run(progress)
        self.worker.stop()
        self.claude.settle()
        self.alive.clear()
        self.db_ids.clear()
        progress["marks"] = state
        progress["clock"] = self.clock.value
        progress["idle"] = dict(self.idle)
        progress["counts"] = dict(self.counts)
        progress["llm"] = self.claude.counts
        progress["log"] = {
            "counted": dict(self.worker.counted), "errors": self.worker.errors, "dropped": self.worker.dropped}
        progress["at_checkpoint"] = {
            "dropped": len(self.worker.dropped), "messages": self.counts["messages"],
            "api_errors": dict(self.claude.counts["api_errors"])}
        save(progress)
        if not final:
            self.worker.start()
            self.verify_marks(progress)
        return True

    def check_run(self, progress):
        """What would fail the build at its end, checked at each quiet point instead, before more of the LLM is spent:
        observations written by a model Claude Code never reported (claude-mem records the model each reply names);
        since the last checkpoint, more batches dropped than a model's odd reply explains, or API errors a retry does not
        heal, both what a refusal the API repeats (a 400) would leave."""
        self.claude.pump()
        served = {m for (m,) in query("SELECT DISTINCT generated_by_model FROM observations") if m}
        allowed = self.claude.models()
        if served - allowed:
            sys.exit(
                f"claude-mem: observations were generated by {sorted(served - allowed)}, not by a model Claude Code reported {sorted(allowed)}")
        since = progress.get("at_checkpoint") or {}
        dropped = len(self.worker.dropped) - since.get("dropped", 0)
        messages = self.counts["messages"] - since.get("messages", 0)
        if dropped > max(3, messages // 100):
            sys.exit(f"claude-mem: claude-mem dropped {dropped} batches for {messages} messages since the last checkpoint")
        errors = {k: n - (since.get("api_errors") or {}).get(k, 0) for k, n in self.claude.counts["api_errors"].items()}
        lasting = {k: n for k, n in errors.items() if k not in TRANSIENT_ERRORS and n}
        if sum(lasting.values()) > 3:
            sys.exit(f"claude-mem: Claude Code reported API errors a retry does not heal since the last checkpoint: {lasting}")

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
    """Run in the chroma-mcp environment: how many documents a Chroma directory holds, by doc_type, read a page at a
    time."""
    import chromadb

    collection = chromadb.PersistentClient(path=path).get_collection("cm__claude-mem")
    total, kinds = collection.count(), Counter()
    for offset in range(0, total, CHROMA_PAGE):
        page = collection.get(include=["metadatas"], limit=CHROMA_PAGE, offset=offset)["metadatas"]
        kinds.update(m.get("doc_type") for m in page)
    if sum(kinds.values()) != total:
        sys.exit(f"claude-mem: Chroma's pages held {sum(kinds.values())} documents, not its {total}")
    print(json.dumps({"count": total, "by_doc_type": dict(kinds)}))


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
        "claude_code": out("/opt/claude-code/claude", "--version"),
        "claude_code_sha256": subprocess.run(
            ["sha256sum", "/opt/claude-code/claude"], capture_output=True, text=True).stdout.split()[0],
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
    config = [{"key": k, "value": v, "where": "settings.json", "cite": cite} for k, (v, cite) in SETTINGS.items()]
    config += [{"key": k, "value": v, "where": "environment", "cite": cite} for k, (v, cite) in ENV.items()]
    config += [{"key": k, "value": "set", "where": how, "cite": cite}
               for k, (how, cite) in AUTH_CONFIG.items() if os.environ.get(k)]
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
            "planned": len(plan), "replayed": replayed, "skipped": skipped_files, "completed": statuses.get("completed", 0),
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
            "observer_replies": llm.get("turns", {}).get("observer", 0),
            "limit_waits": counts.get("limit_waits", 0),
            "limit_wait_seconds": round(counts.get("limit_wait_seconds", 0)),
            "throttle_waits": counts.get("throttle_waits", 0),
            "throttle_wait_seconds": round(counts.get("throttle_wait_seconds", 0)),
            "breaker_steps": counts.get("breaker_steps", 0),
            "breaker_holds": counts.get("breaker_holds", 0),
            "slot_steps": counts.get("slot_steps", 0),
            "slot_holds": counts.get("slot_holds", 0),
            "overflow_steps": counts.get("overflow_steps", 0),
            "overflow_holds": counts.get("overflow_holds", 0),
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
        "llm": {
            "model": LLM_MODEL, "provider": "claude", "claude_code": CLAUDE_CODE, "tier_simple_model": TIER_SIMPLE,
            "models": sorted(m for m, v in llm.get("by_model", {}).items() if m and v.get("calls"))}
        | {k: v for k, v in sorted(llm.items())},
        "ingest_seconds": round(progress["ingest_seconds"]),
        "finished": datetime.now(timezone.utc).isoformat(timespec="seconds"),
    }


# The tables the merge combines, each with the column that orders its rows in time and the integer ids of other tables
# it holds (SessionStore.ts, tool-uses.ts). Every other table must be empty in every lane, but the migrations' ledger,
# which every lane holds alike.
MERGED = {
    "sdk_sessions": ("started_at_epoch", {}),
    "observations": ("created_at_epoch", {}),
    "session_summaries": ("created_at_epoch", {}),
    "user_prompts": ("created_at_epoch", {"session_db_id": "sdk_sessions"}),
    "tool_uses": ("created_at_epoch", {"session_db_id": "sdk_sessions", "observation_id": "observations"}),
}
KINDS = {"observations": "observations", "summaries": "session_summaries", "prompts": "user_prompts"}  # Chroma's and the sync state's
PROJECT_IDS = {
    "observations": "SELECT id FROM observations WHERE project = ?",
    "summaries": "SELECT id FROM session_summaries WHERE project = ?",
    "prompts": "SELECT p.id FROM user_prompts p JOIN sdk_sessions s ON s.id = p.session_db_id WHERE s.project = ?"}
SYNC_KEYS = {"observations", "summaries", "prompts", "pending", "fragmentReconciliation", "titleOnlyRequeued", "rebuildPending"}
# Columns of the merged tables named like ids that hold no id of a merged table: Claude Code's and claude-mem's own
# strings, and cloud sync's, which must be empty. A new one fails the merge until it is placed here or in MERGED.
TEXT_IDS = {
    "content_session_id", "memory_session_id", "tool_use_id", "agent_id", "native_prompt_id", "or_generation_id",
    "or_session_id", "origin_device_id", "origin_local_id"}
STORE_FILES = {"claude-mem.db", "chroma", "chroma-sync-state.json", "settings.json", *MARKERS}
CHROMA_RECORD = ".claude-mem-chroma-store.json"
# How far a copied vector may move in any component: float32 rounding as hnswlib normalizes an already normalized cosine
# vector again (about 1e-8), a hundred times over; a wrong or swapped vector is off by orders of magnitude more.
VECTOR_TOLERANCE = 1e-6


def merge(plan, skipped_files):
    """The lanes' stores as one, as one worker holds every repository's rows: SQLite's rows renumbered in time order
    across the lanes, each lane's own order kept, with the ids they hold of each other remapped; Chroma's documents and
    vectors copied under the new ids; the sync state's marks moved to them. Every lane must have replayed all its
    sessions, and the merged store must hold every row and document of every lane, equal but for its ids, or the build
    fails. Writes /work/store and its manifest."""
    lanes = []
    for lane in dict.fromkeys(repo for _, repo, _ in plan):
        saved = LANES / lane / "checkpoint"
        want = {f"{repo}/{path.name}" for _, repo, path in plan if repo == lane}
        progress = json.loads((saved / "progress.json").read_text()) if (saved / "progress.json").exists() else {"done": []}
        if set(progress["done"]) != want:
            sys.exit(f"claude-mem: lane {lane} has replayed {len(progress['done'])} of its {len(want)} sessions; run again to finish it")
        lanes.append((lane, saved, progress))
    if len({(saved / "settings.json").read_text() for _, saved, _ in lanes}) != 1:
        sys.exit("claude-mem: the lanes' settings.json differ")
    if len({(saved / MARKERS[0]).exists() for _, saved, _ in lanes}) != 1:
        sys.exit(f"claude-mem: only some lanes hold {MARKERS[0]}")
    tmp, out = WORK / "store.tmp", WORK / "store"
    shutil.rmtree(tmp, ignore_errors=True)
    tmp.mkdir(parents=True)
    # The lane of the first session started its worker at the first event, as one worker would have: its migrations'
    # ledger, settings and cleanup marker are the store's. claude-mem writes the reclaim marker on a real 60 s timer, in
    # the first worker run that lasts that long, which a short lane may never have: the first lane's that has one.
    for name in ("settings.json", *MARKERS):
        holder = next((saved for _, saved, _ in lanes if (saved / name).exists()), None)
        if holder:
            shutil.copy2(holder / name, tmp / name)
    log(f"merging {len(lanes)} lanes: SQLite")
    ids, rows = merge_sqlite(lanes, tmp / "claude-mem.db")
    merge_sync_state(lanes, ids, tmp)
    log("merging: Chroma")
    documents = merge_chroma_dirs(lanes, ids, tmp / "chroma")
    log(f"merged {documents['documents']} Chroma documents: vectors within {documents['vector_max_difference']:.3g} of the "
        f"lanes' in every component, cosine at least 1 - {1 - documents['vector_min_cosine']:.3g}")
    progress = merged_progress(lanes, ids)
    if extra := {f.name for f in tmp.iterdir()} - STORE_FILES:
        sys.exit(f"claude-mem: the merged store holds files a store does not: {sorted(extra)}")
    shutil.rmtree(out, ignore_errors=True)
    tmp.rename(out)
    found = manifest(progress, skipped_files, plan, len(progress["done"]))
    found["lanes"] = {
        lane: {
            "sessions": len(p["done"]), "ingest_seconds": round(p["ingest_seconds"]),
            "llm_calls": sum(m.get("calls", 0) for m in p["llm"].get("by_model", {}).values()),
            "rows": {t: len(ids[t][lane]) for t in MERGED},
            "chroma_documents": documents["lanes"].get(lane, 0),
            "waits": {k: round(p["counts"].get(k, 0)) for k in (
                "limit_waits", "limit_wait_seconds", "throttle_waits", "throttle_wait_seconds", "breaker_steps",
                "breaker_holds", "slot_steps", "slot_holds", "overflow_steps", "overflow_holds", "skipped_checkpoints")}}
        for lane, _, p in lanes}
    found["merge"] = {
        "rows": rows, "chroma_documents": documents["documents"],
        "vector_tolerance": VECTOR_TOLERANCE, "vector_max_difference": documents["vector_max_difference"],
        "vector_min_cosine": documents["vector_min_cosine"],
        "checked": "every row of every lane is in the store, equal but for its ids; every Chroma document with its id, text and metadata equal but for the new id, and its vector within vector_tolerance in every component: a copied vector is normalized again by hnswlib as it is added, which moves it by float32 rounding only; SQLite's integrity, foreign keys and full-text tables verify against their rows; every project's sync marks leave exactly its lane's rows, renumbered, for claude-mem's backfill"}
    (out / "manifest.json").write_text(json.dumps(found, indent=2) + "\n")


def merge_sqlite(lanes, dest):
    """The lanes' databases as one: the first lane's, emptied of the merged tables through claude-mem's own triggers
    (which keep the full-text tables in step), then filled with every lane's rows under new ids, then checked row by row.
    Returns the new ids, {table: {lane: {old: new}}}, and the row counts."""
    shutil.copy2(lanes[0][1] / "claude-mem.db", dest)
    sources = {
        lane: sqlite3.connect(f"file:{saved / 'claude-mem.db'}?mode=ro&immutable=1", uri=True) for lane, saved, _ in lanes}
    schemas = {
        lane: sorted(c.execute("SELECT type, name, tbl_name, sql FROM sqlite_master WHERE name != 'sqlite_sequence'"))
        for lane, c in sources.items()}
    if len({json.dumps(v) for v in schemas.values()}) != 1:
        sys.exit("claude-mem: the lanes' databases have different schemas")
    first = next(iter(sources.values()))
    virtual = [n for (n,) in first.execute("SELECT name FROM sqlite_master WHERE type = 'table' AND sql LIKE 'CREATE VIRTUAL TABLE%'")]
    tables = [
        n for (n,) in first.execute("SELECT name FROM sqlite_master WHERE type = 'table'")
        if n != "sqlite_sequence" and n not in virtual and not n.startswith(tuple(f"{v}_" for v in virtual))]
    for t in tables:
        if t in MERGED or t == "schema_versions":
            continue
        held = {lane: c.execute(f'SELECT COUNT(*) FROM "{t}"').fetchone()[0] for lane, c in sources.items()}
        if any(held.values()):
            sys.exit(f"claude-mem: the lanes hold rows in {t}, which the merge does not combine: {held}")
    if len({json.dumps(c.execute("SELECT id, version FROM schema_versions ORDER BY id").fetchall()) for c in sources.values()}) != 1:
        sys.exit("claude-mem: the lanes' databases ran different migrations")
    projects = {}
    for lane, c in sources.items():
        for (project,) in c.execute(
                "SELECT project FROM sdk_sessions UNION SELECT project FROM observations UNION SELECT project FROM session_summaries "
                "UNION SELECT project FROM tool_uses UNION SELECT merged_into_project FROM observations "
                "UNION SELECT merged_into_project FROM session_summaries"):
            if project is not None:
                projects.setdefault(project.lower(), set()).add(lane)
    shared = {p: sorted(ls) for p, ls in projects.items() if len(ls) > 1}
    if shared:
        sys.exit(f"claude-mem: projects span lanes, whose state one worker would keep together: {shared}")
    for t, (_, refs) in MERGED.items():
        cols = [r[1] for r in first.execute(f'PRAGMA table_info("{t}")')]
        if unknown := [c for c in cols if c.endswith("_id") and c not in refs and c not in TEXT_IDS]:
            sys.exit(f"claude-mem: {t} has columns named like ids the merge does not know: {unknown}")
        if "origin_local_id" in cols:
            synced = {lane: c.execute(f'SELECT COUNT(*) FROM "{t}" WHERE origin_device_id IS NOT NULL OR origin_local_id IS NOT NULL').fetchone()[0]
                      for lane, c in sources.items()}
            if any(synced.values()):
                sys.exit(f"claude-mem: {t} holds cloud sync's ids of local rows, which the merge does not renumber: {synced}")

    db = sqlite3.connect(dest)
    for t in reversed(MERGED):
        db.execute(f'DELETE FROM "{t}"')
    db.execute(f"DELETE FROM sqlite_sequence WHERE name IN ({', '.join('?' * len(MERGED))})", tuple(MERGED))
    ids = {t: {lane: {} for lane in sources} for t in MERGED}
    for t, (when, refs) in MERGED.items():
        # In time order across the lanes; heapq.merge keeps each lane's own order whatever its times.
        streams = [
            [((w if w is not None else -1), k, lane, i) for i, w in c.execute(f'SELECT id, "{when}" FROM "{t}" ORDER BY id')]
            for k, (lane, c) in enumerate(sources.items())]
        for new, (_, _, lane, old) in enumerate(heapq.merge(*streams), start=1):
            ids[t][lane][old] = new
        cols = [r[1] for r in db.execute(f'PRAGMA table_info("{t}")')]
        names = ", ".join('"' + c + '"' for c in cols)
        insert = f'INSERT INTO "{t}" ({names}) VALUES ({", ".join("?" * len(cols))})'
        for new, lane, old in sorted((new, lane, old) for lane, m in ids[t].items() for old, new in m.items()):
            row = dict(zip(cols, sources[lane].execute(f'SELECT * FROM "{t}" WHERE id = ?', (old,)).fetchone()))
            row["id"] = new
            for col, target in refs.items():
                if row[col] is not None:
                    if row[col] not in ids[target][lane]:
                        sys.exit(f"claude-mem: {t} {old} of lane {lane} holds {col} {row[col]}, which no {target} row has")
                    row[col] = ids[target][lane][row[col]]
            try:
                db.execute(insert, [row[c] for c in cols])
            except sqlite3.IntegrityError as e:
                sys.exit(f"claude-mem: {t} {old} of lane {lane} does not fit the merged store: {e}")
    # Rebuilt from the rows, so the first lane's deleted rows leave nothing behind in the index.
    for fts in virtual:
        db.execute(f'INSERT INTO "{fts}"("{fts}") VALUES (\'rebuild\')')
    db.commit()
    db.execute("VACUUM")
    db.close()

    # The check: every row of every lane, equal but for its ids, and nothing else.
    db = sqlite3.connect(dest)
    rows = {}
    for t, (_, refs) in MERGED.items():
        cols = [r[1] for r in db.execute(f'PRAGMA table_info("{t}")')]
        total = sum(c.execute(f'SELECT COUNT(*) FROM "{t}"').fetchone()[0] for c in sources.values())
        if db.execute(f'SELECT COUNT(*), MIN(id), MAX(id) FROM "{t}"').fetchone() != (total, 1 if total else None, total or None):
            sys.exit(f"claude-mem: the merged {t} does not hold the lanes' {total} rows under ids 1 to {total}")
        for lane, c in sources.items():
            for row in c.execute(f'SELECT * FROM "{t}" ORDER BY id'):
                want = dict(zip(cols, row))
                want["id"] = ids[t][lane][want["id"]]
                for col, target in refs.items():
                    if want[col] is not None:
                        want[col] = ids[target][lane][want[col]]
                got = db.execute(f'SELECT * FROM "{t}" WHERE id = ?', (want["id"],)).fetchone()
                if got != tuple(want[c] for c in cols):
                    sys.exit(f"claude-mem: the merged {t} {want['id']} differs from {t} {row[0]} of lane {lane}")
        rows[t] = total
    for t in tables:
        if t not in MERGED and t != "schema_versions" and db.execute(f'SELECT COUNT(*) FROM "{t}"').fetchone()[0]:
            sys.exit(f"claude-mem: the merged {t} holds rows")
    if db.execute("SELECT * FROM schema_versions ORDER BY id").fetchall() != first.execute("SELECT * FROM schema_versions ORDER BY id").fetchall():
        sys.exit("claude-mem: the merged migrations' ledger differs from the first lane's")
    integrity = [r[0] for r in db.execute("PRAGMA integrity_check")]
    dangling = len(db.execute("PRAGMA foreign_key_check").fetchall())
    before = sum(len(c.execute("PRAGMA foreign_key_check").fetchall()) for c in sources.values())
    if integrity != ["ok"] or dangling != before:
        sys.exit(f"claude-mem: the merged database fails its integrity check {integrity}, or has {dangling} foreign keys that name no row where the lanes had {before}")
    for fts in virtual:
        try:
            db.execute(f'INSERT INTO "{fts}"("{fts}", rank) VALUES (\'integrity-check\', 1)')
        except sqlite3.Error as e:
            sys.exit(f"claude-mem: the merged {fts} does not match its rows: {e}")
    db.rollback()
    db.execute("PRAGMA wal_checkpoint(TRUNCATE)")
    db.close()
    for name in ("claude-mem.db-wal", "claude-mem.db-shm"):
        (dest.parent / name).unlink(missing_ok=True)
    for c in sources.values():
        c.close()
    return ids, rows


def merge_sync_state(lanes, ids, dest):
    """Each project's marks in chroma-sync-state.json under the merged ids (ChromaSyncState.ts): a mark is the highest id
    of a kind Chroma holds for the project, and the pending ids are rows it still lacks. A project caught up in its lane
    must be caught up in the store."""
    merged = {}
    store = sqlite3.connect(f"file:{dest / 'claude-mem.db'}?mode=ro&immutable=1", uri=True)
    for lane, saved, _ in lanes:
        path = saved / "chroma-sync-state.json"
        state = json.loads(path.read_text()) if path.exists() else {}
        c = sqlite3.connect(f"file:{saved / 'claude-mem.db'}?mode=ro&immutable=1", uri=True)
        for project, marks_ in state.items():
            if project in merged:
                sys.exit(f"claude-mem: two lanes' sync state hold {project}")
            if set(marks_) - SYNC_KEYS:
                sys.exit(f"claude-mem: {project}'s sync state holds what the merge does not know: {sorted(set(marks_) - SYNC_KEYS)}")
            new = dict(marks_)
            for kind, table in KINDS.items():
                if kind not in marks_:
                    continue
                own = [i for (i,) in c.execute(PROJECT_IDS[kind], (project,))]
                new[kind] = max((ids[table][lane][i] for i in own if i <= marks_[kind]), default=0)
                # claude-mem's backfill sends a project's rows past its mark (ChromaSync.ts ensureBackfilled): the same
                # rows, renumbered, as in the lane.
                after = {j for (j,) in store.execute(PROJECT_IDS[kind], (project,)) if j > new[kind]}
                if after != {ids[table][lane][i] for i in own if i > marks_[kind]}:
                    sys.exit(f"claude-mem: {project}'s {kind} mark leaves other rows to the backfill than in lane {lane}")
            for key in ("pending", "fragmentReconciliation"):
                if key in marks_:
                    try:
                        new[key] = {kind: [ids[KINDS[kind]][lane][i] for i in v] for kind, v in marks_[key].items()}
                    except KeyError as e:
                        sys.exit(f"claude-mem: {project}'s sync state names {key} {e}, which lane {lane} does not hold")
            merged[project] = new
        c.close()
    store.close()
    (dest / "chroma-sync-state.json").write_text(json.dumps(merged, indent=2))


def merge_chroma_dirs(lanes, ids, dest):
    """Chroma's part of the merge, in the chroma-mcp environment, on copies: opening a Chroma directory writes to it."""
    work = WORK / "merge.tmp"  # beside the lanes, not on the container's own disk
    shutil.rmtree(work, ignore_errors=True)
    spec = {"out": str(dest), "lanes": []}
    for lane, saved, _ in lanes:
        shutil.copytree(saved / "chroma", work / lane)
        spec["lanes"].append({
            "lane": lane, "chroma": str(work / lane),
            "ids": {kind: {str(old): new for old, new in ids[table][lane].items()} for kind, table in KINDS.items()}})
    (work / "spec.json").write_text(json.dumps(spec))
    r = subprocess.run(
        ["uvx", *LAUNCHER, "python", "/app/build.py", "--merge-chroma", str(work / "spec.json")], capture_output=True,
        text=True, env=os.environ | {"OPENAI_BASE_URL": EMBEDDER_URL, "OPENAI_API_KEY": "local"})
    shutil.rmtree(work, ignore_errors=True)
    if r.returncode:
        sys.exit(f"claude-mem: merging Chroma failed: {r.stderr.strip()[-2000:]}")
    # claude-mem's record of what last wrote the Chroma directory, which it rewrites at each chroma-mcp start
    # (ChromaMcpManager.ts writeChromaStoreRecord): the latest of the lanes', as one worker's last start left it.
    records = [saved / "chroma" / CHROMA_RECORD for _, saved, _ in lanes if (saved / "chroma" / CHROMA_RECORD).exists()]
    if records:
        shutil.copy2(max(records, key=lambda f: json.loads(f.read_text()).get("updatedAt", "")), dest / CHROMA_RECORD)
    return json.loads(r.stdout.strip().splitlines()[-1])


def merge_chroma(spec_path):
    """Run in the chroma-mcp environment: every lane's Chroma documents, with their vectors and metadata, into one
    collection configured as the lanes' are, under the merged ids and in their time order; then each read back and
    compared with its lane's: its document and metadata equal, its vector within VECTOR_TOLERANCE in every component.
    A vector does not come back bit for bit: hnswlib normalizes a cosine vector as it adds it, and the lane's had been
    normalized once already, so float32 rounding moves it by about 1e-8; embedding again instead would tie the merge to
    the embedder giving the same bits twice. Prints what it wrote and the largest deviation."""
    import chromadb
    import numpy

    spec = json.loads(Path(spec_path).read_text())
    prefixes = {"obs": "observations", "summary": "summaries", "prompt": "prompts"}
    lanes = []
    for lane in spec["lanes"]:
        client = chromadb.PersistentClient(path=lane["chroma"])
        if [c.name for c in client.list_collections()] != ["cm__claude-mem"]:
            sys.exit(f"claude-mem: lane {lane['lane']}'s Chroma holds other collections than cm__claude-mem")
        collection = client.get_collection("cm__claude-mem")
        lanes.append((lane["lane"], collection, {k: {int(o): n for o, n in m.items()} for k, m in lane["ids"].items()}))
    shapes = {json.dumps([c.configuration_json, c.metadata], sort_keys=True, default=str) for _, c, _ in lanes}
    if len(shapes) != 1:
        sys.exit(f"claude-mem: the lanes' Chroma collections are configured differently: {shapes}")

    def renamed(doc_id, metadata, ids):
        found = re.fullmatch(r"(obs|summary|prompt)_(\d+)(.*)", doc_id)
        if not found:
            sys.exit(f"claude-mem: Chroma's {doc_id} is not a document id claude-mem writes")
        prefix, old, rest = found.groups()
        new = ids[prefixes[prefix]].get(int(old))
        if new is None or metadata.get("sqlite_id") != int(old):
            sys.exit(f"claude-mem: Chroma's {doc_id} names no row the merge renumbered")
        return f"{prefix}_{new}{rest}", metadata | {"sqlite_id": new}

    def every(collection, include):
        offset = 0
        while True:
            got = collection.get(include=include, limit=CHROMA_PAGE, offset=offset)
            if not got["ids"]:
                return
            yield got
            offset += len(got["ids"])

    order, counted = [], Counter()
    for k, (lane, collection, ids) in enumerate(lanes):
        for got in every(collection, ["metadatas"]):
            for doc_id, metadata in zip(got["ids"], got["metadatas"]):
                new_id, new_metadata = renamed(doc_id, metadata, ids)
                order.append((metadata.get("created_at_epoch") or 0, new_metadata["sqlite_id"], new_id, k, doc_id))
                counted[lane] += 1
    order.sort()
    if len({new_id for _, _, new_id, _, _ in order}) != len(order):
        sys.exit("claude-mem: two lanes' Chroma documents take the same merged id")
    first = lanes[0][1]
    target = chromadb.PersistentClient(path=spec["out"]).create_collection(
        "cm__claude-mem", configuration=first.configuration, metadata=first.metadata,
        embedding_function=first.configuration["embedding_function"])

    def batches():
        for b in range(0, len(order), 1000):
            part = order[b : b + 1000]
            got = {}
            for k in {k for *_, k, _ in part}:
                lane, collection, ids = lanes[k]
                found = collection.get(
                    ids=[doc_id for *_, kk, doc_id in part if kk == k], include=["documents", "metadatas", "embeddings"])
                for doc_id, document, metadata, embedding in zip(
                        found["ids"], found["documents"], found["metadatas"], found["embeddings"]):
                    got[(k, doc_id)] = (document, metadata, embedding)
            if missing := [doc_id for *_, k, doc_id in part if (k, doc_id) not in got]:
                sys.exit(f"claude-mem: Chroma did not return {missing[:5]}")
            yield [(new_id, k, doc_id, *got[(k, doc_id)]) for _, _, new_id, k, doc_id in part]

    for part in batches():
        target.add(
            ids=[new_id for new_id, *_ in part], documents=[d for _, _, _, d, _, _ in part],
            metadatas=[renamed(doc_id, m, lanes[k][2])[1] for _, k, doc_id, _, m, _ in part],
            embeddings=[e for *_, e in part])
    # The check, read back from the merged collection.
    if target.count() != len(order):
        sys.exit(f"claude-mem: the merged Chroma holds {target.count()} documents, not {len(order)}")
    if json.dumps([target.configuration_json, target.metadata], sort_keys=True, default=str) != shapes.pop():
        sys.exit("claude-mem: the merged Chroma collection is configured differently from the lanes'")
    kinds, largest, lowest = Counter(), 0.0, 1.0
    for part in batches():
        found = target.get(ids=[new_id for new_id, *_ in part], include=["documents", "metadatas", "embeddings"])
        stored = {i: (d, m, e) for i, d, m, e in zip(found["ids"], found["documents"], found["metadatas"], found["embeddings"])}
        for new_id, k, doc_id, document, metadata, embedding in part:
            d, m, e = stored[new_id]
            if d != document or m != renamed(doc_id, metadata, lanes[k][2])[1]:
                sys.exit(f"claude-mem: the merged Chroma's {new_id} differs from {doc_id} of lane {lanes[k][0]}")
            got, want = numpy.asarray(e, dtype=numpy.float64), numpy.asarray(embedding, dtype=numpy.float64)
            off = float(numpy.max(numpy.abs(got - want))) if got.shape == want.shape else float("inf")
            cosine = float(got @ want / (numpy.linalg.norm(got) * numpy.linalg.norm(want))) if off != float("inf") else 0.0
            if not off <= VECTOR_TOLERANCE:  # a NaN fails too
                sys.exit(f"claude-mem: the merged Chroma's vector of {new_id} is off {doc_id}'s of lane {lanes[k][0]} by {off:.3g}")
            kinds[m.get("doc_type")] += 1
            largest, lowest = max(largest, off), min(lowest, cosine)
    print(json.dumps({
        "documents": len(order), "by_doc_type": dict(kinds), "lanes": dict(counted),
        "vector_max_difference": largest, "vector_min_cosine": lowest}))


def merged_progress(lanes, ids):
    """The lanes' progress as one: counts and the LLM's usage add up, a window's first and last readings are the earliest
    and the latest of any lane, and session db ids and rejected rows follow the merge."""
    merged = {
        "done": [], "sessions": {}, "counts": Counter(), "idle": Counter(), "llm": {}, "rejected": [],
        "ingest_seconds": 0, "log": {"counted": Counter(), "errors": [], "dropped": []}}
    for lane, _, progress in lanes:
        sessions = ids["sdk_sessions"][lane]
        merged["done"] += progress["done"]
        merged["sessions"] |= {name: s | {"db_id": sessions[s["db_id"]]} for name, s in progress["sessions"].items()}
        merged["counts"].update(progress["counts"])
        merged["idle"].update(progress["idle"])
        merged["ingest_seconds"] += progress["ingest_seconds"]
        merged["rejected"] += [[k, ids[KINDS[k]][lane][i], n] for k, i, n in progress["rejected"]]
        merged["log"]["counted"].update(progress["log"]["counted"])
        merged["log"]["errors"] += [f"[{lane}] {e}" for e in progress["log"]["errors"]]
        merged["log"]["dropped"] += [
            d | {"session_db_id": sessions.get(d["session_db_id"])} for d in progress["log"].get("dropped", [])]
        add_usage(merged["llm"], progress["llm"])
    return merged


def add_usage(total, part):
    for key, value in part.items():
        if key == "windows":
            for window, w in value.items():
                t = total.setdefault("windows", {}).setdefault(window, {})
                if "first" in w and ("first" not in t or w["first"]["seen"] < t["first"]["seen"]):
                    t["first"] = w["first"]
                if "last" in w and ("last" not in t or w["last"]["seen"] > t["last"]["seen"]):
                    t["last"] = w["last"]
                t["max"] = max(t.get("max", 0), w.get("max", 0))
        elif isinstance(value, dict):
            add_usage(total.setdefault(key, {}), value)
        else:
            total[key] = total.get(key, 0) + value


def interrupted(signum, frame):
    sys.exit("claude-mem: stopped; run again to resume from the last checkpoint")


def main():
    if sys.argv[1:2] == ["--count-chroma"]:
        return count_chroma(sys.argv[2])
    if sys.argv[1:2] == ["--merge-chroma"]:
        return merge_chroma(sys.argv[2])
    if sys.argv[1:2] == ["--lanes"]:
        for repo in dict.fromkeys(repo for _, repo, _ in sessions()[0]):
            print(repo)
        return
    signal.signal(signal.SIGINT, interrupted)
    signal.signal(signal.SIGTERM, interrupted)
    given = [k for k in AUTH if os.environ.get(k)]
    if len(given) != 1:
        sys.exit(f"claude-mem: set exactly one of {' or '.join(AUTH)} for the observer's account (set: {given or 'none'})")
    if not LLM_MODEL:
        sys.exit("claude-mem: BILBO_EVAL_LLM_MODEL is not set")
    # One build at a time in a work dir: lanes share the mount, so this holds across containers.
    WORK.mkdir(parents=True, exist_ok=True)
    lock = (WORK / ".lock").open("w")
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        sys.exit(f"claude-mem: another build is working in {WORK}")

    HOME.mkdir(parents=True, exist_ok=True)
    LIVE.mkdir(parents=True, exist_ok=True)
    plan, skipped_files = sessions()
    if MERGE:
        merge(plan, skipped_files)
        return finish(given[0])
    every = plan  # every lane's sessions, whose checkouts a lane makes too, as one worker's build has them all
    if LANE:
        plan = [s for s in plan if s[1] == LANE]
        if not plan:
            sys.exit(f"claude-mem: the lane {LANE} has no session")
    IO.mkdir(parents=True, exist_ok=True)
    if not CHECKPOINT.exists() and (WORK / "checkpoint.old").exists():
        (WORK / "checkpoint.old").rename(CHECKPOINT)
    first = millis(plan[0][0])
    if (CHECKPOINT / "progress.json").exists():
        progress = restore()
        log(f"resuming after {len(progress['done'])} sessions")
    else:
        DATA.mkdir(parents=True)
        progress = {
            "done": [], "sessions": {}, "counts": {}, "llm": {}, "idle": {}, "marks": {}, "rejected": [],
            "ingest_seconds": 0, "clock": first, "log": {"counted": {}, "errors": []}}
    counts = Counter(progress["counts"])
    clock = Clock(progress["clock"])
    claude = Claude(progress["llm"])
    worker = Worker(progress["log"]["counted"])
    worker.errors = progress["log"]["errors"]
    worker.dropped = progress["log"].get("dropped", [])
    try:
        if len(progress["done"]) < len(plan):  # a rerun of a build that finished goes straight to the store
            launches = [
                (repo, posix_path(next((e["cwd"] for e in own_entries(path) if e.get("cwd")), "/")))
                for _, repo, path in every
            ]
            checkouts = Checkouts(checkout_roots(), launches)
            worker.clock = clock
            replay = Replay(
                clock, worker, claude, checkouts, counts, Counter(progress["idle"]),
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
                        progress_line(progress, len(plan), counts, claude)
                    started = time.monotonic()
            progress["ingest_seconds"] += time.monotonic() - started
            if not replay.quiet_point(finished, progress, float("inf"), final=True):
                sys.exit("claude-mem: the last quiet point could not drain the observer; run again to resume from the last checkpoint")
    finally:
        worker.kill()
    if LANE:
        log(f"the lane {LANE} is done: its {len(plan)} sessions are in {CHECKPOINT}, for the merge")
        return

    out = WORK / "store"
    copy_store(out)
    (out / "manifest.json").write_text(
        json.dumps(manifest(progress, skipped_files, plan, len(progress["done"])), indent=2) + "\n")
    finish(given[0])


def finish(credential):
    """No file of /work may hold the account's credential, the lanes' included."""
    secret = os.environ[credential].encode()
    leaked = []
    for f in WORK.rglob("*"):
        if f.is_file() and not f.is_symlink() and f.stat().st_size:
            with f.open("rb") as fh, mmap.mmap(fh.fileno(), 0, access=mmap.ACCESS_READ) as m:
                if m.find(secret) != -1:
                    leaked.append(str(f.relative_to(WORK)))
    if leaked:
        sys.exit(f"claude-mem: the account's credential is in {leaked[:5]}; remove them before anything leaves /work")
    log(f"done: {WORK / 'store'}")


def progress_line(progress, total, counts, claude):
    calls = {name: m.get("calls", 0) for name, m in sorted(claude.counts["by_model"].items()) if name}
    hooks = sum(counts[k] for k in ("UserPromptSubmit", "PostToolUse", "PostToolUseFailure", "Stop", "SessionEnd"))
    log(f"{len(progress['done'])}/{total} sessions: {hooks} hooks, {sum(calls.values())} LLM calls so far {calls}")


if __name__ == "__main__":
    main()
