"""Build the bilbo store: the L3 sessions eval's bilbo writer (l3/, copied byte for byte), driven over this eval's corpus.
Every project is written on its own, session by session: after each exchange a call asks the model whether
the agent would use bilbo's note skill (the decision pass), and where it would, a tool loop runs the skill as shipped
against the real bilbo binary. The assisted row adds a bare /note after the last exchange of every session. The LLM
calls go through a loopback gateway to BILBO_EVAL_LLM_URL.
The store is built in lanes (x-lanes in compose.yaml): with BILBO_EVAL_LANE set, this writes only that project, in a build
container of its own, into /work/states/<project> (scratch in /work/lanes/<project>); `--merge` then merges the projects'
notes into one bilbo store, indexed once with the eval's embedder and saved with the writer's journals and a manifest in
/work/store, and `--lanes` names the lanes. The writer checkpoints after every exchange, and a run that finds them
resumes from there; a copy of a project's state is kept at the end of each session in /work/backup.
Both arms mount one folder at /decisions: the decision pass of a project does not depend on the row, so whichever arm
reaches a project first draws it, under the writer's own lock, and the other finds every row done.
A seed (the published store this build continues) restores the five L3 projects' journals, decisions and notes after
checking that they render to the same exchanges, so only the other projects cost an LLM call. The renderer's input is
fixed first by replay.py (separators escaped, a teleported session started at its own first record). A call the model's
safeguards refuse stops both builds and is never sent again. BILBO_EVAL_DRY_RUN=1 renders every exchange on the host and
counts the calls, with no LLM and no Docker."""

import fcntl
import hashlib
import http.client
import json
import os
import platform
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
from collections import Counter
from datetime import datetime, timezone
from email.utils import parsedate_to_datetime
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from importlib.metadata import version
from math import isfinite
from pathlib import Path, PurePosixPath
from urllib.parse import quote, unquote, urlparse

HERE = Path(__file__).resolve().parent
L3 = Path("/app/l3") if Path("/app/l3").is_dir() else HERE / "l3"
sys.path.insert(0, str(L3))
sys.path.insert(1, str(HERE))

import render  # noqa: E402
import replay  # noqa: E402
import schema  # noqa: E402
import writer  # noqa: E402

ROW = os.environ.get("BILBO_EVAL_ROW", "")
# The arm names bilbo's version: a later bilbo may write and search another way.
ARM = f"bilbo-{ROW}-v0_19"
# L3's name for the row, in the writer's labels and journals, which the replay of L3's records matches.
L3_ARM = f"bilbo-{ROW}"
MODEL = "qwen3-embedding-0.6b"
DIMS = 1024
# bilbo's own query prefix for this model (config.rs:37, the value its setup writes for a qwen3-embedding model).
QUERY_PREFIX = "Instruct: Given a question, retrieve notes that answer it\nQuery: "
# The server reuses the KV entries of the prefix a request shares with the previous one, so a vector would depend on
# the text embedded before it. One token that no passage starts with, sent before the store's index, leaves none.
SENTINEL = "\u0000"
L3_COMMIT = "40b0633"
L3_FILES = {  # sha256 of what the L3 eval froze at that commit
    "writer.py": "b66aeaabd07ec670ad5a2306765ee7af5dcdb3dbccfb8540c5a5b234d9291e28",
    "schema.py": "ce9013d7ee203cd2201242a904d0ec930d9f73cd0fa6c5cc39bacd856c5942d7",
    "render.py": "006ad996440817a80a27129e382c5439006d6bfb2330730bbea1318e87b84fdb",
    "prompts/agent_frame.md": "02293b25f70885b686f9edbc538fac9a42b02edc634bb3e034a8d22553117e08",
    "prompts/agent_decide.md": "f3478879f829024d68e32de04c71cd7935bd630010d43b722a2bc22e47795179",
    "policy/schedule.jsonl": "9e5d814de3fe93fbebec39a54ea8975fecc1ce358a73b47d8caaaa79dbe91678",
}
KINDS = "plan|spec|design|decision|gotcha|research|review|report|reference"
JOURNALS = ("done.jsonl", "actions.jsonl", "edits.jsonl", "metrics.jsonl")
NAME = re.compile(r"^(\d{4}-\d\d-\d\dT)(\d\d)(\d\d)(\d\d)Z--(.+)\.jsonl$")
FNV_OFFSET, FNV_PRIME = 0xCBF29CE484222325, 0x100000001B3
# The five projects L3 wrote: byte-identical in both corpora, restored from the seed and not written again.
OLD = ("4gray__iptvnator", "MoonshotAI__kimi-cli", "ababushkin__linear-cycle-drain", "ngngardner__readaloud", "port-experimental__port-cli")
# What a decision call carries besides the window of the session it shows: the frame, the tail and the message wrappers.
DECIDE_OVERHEAD = 6430

DRY = os.environ.get("BILBO_EVAL_DRY_RUN") == "1"
LANE = os.environ.get("BILBO_EVAL_LANE", "")
ON_REFUSAL = os.environ.get("BILBO_EVAL_ON_REFUSAL", "stop")
REFUSAL_LIMIT = int(os.environ.get("BILBO_EVAL_REFUSAL_LIMIT", "3"))

CORPUS = Path(os.environ["BILBO_EVAL_CORPUS"]) if DRY and os.environ.get("BILBO_EVAL_CORPUS") else Path("/corpus")
WORK = Path("/work")
LANES = WORK / "lanes"
DECISIONS = Path("/decisions")
# The refusal memory is kept outside the store key (a trial, a full build and a fixed build.py share it) and holds the per-request
# locks both arms take around an upstream call.
REFUSAL_DIR = Path("/refusals")
REFUSALS = REFUSAL_DIR / "refusals.jsonl"
INFLIGHT = REFUSAL_DIR / "inflight"
SEED = Path(os.environ["BILBO_EVAL_SEED"]) if os.environ.get("BILBO_EVAL_SEED") else WORK / "seed" / "store"
SKILL = Path("/app/skill/SKILL.md")
BILBO = Path("/usr/local/bin/bilbo")


def log(msg):
    print(f"{ARM}: {msg}", file=sys.stderr, flush=True)


def stamp(entry):
    return datetime.fromisoformat(entry["timestamp"]) if entry.get("timestamp") else None


def sessions():
    """Every Claude Code session as (start, repo, path), in the order they started across all repos, as for mem0, each read
    as the renderer will read it (replay.py). A transcript with no timestamp (Cursor's) is left out, as the other arms do.
    BILBO_EVAL_SESSIONS is N, the first N sessions, or repo:N,repo:N, the first N of each repository named."""
    found, other = [], []
    for path in sorted(CORPUS.glob("*/*.jsonl")):
        text = replay.transcript_text(path)[0]
        start = next((t for t in (stamp(json.loads(line)) for line in text.split("\n") if line.strip()) if t), None)
        (found if start else other).append((start, path.parent.name, path))
    found.sort()
    limit = os.environ.get("BILBO_EVAL_SESSIONS") or ""
    if ":" in limit:
        per = {repo: int(n) for repo, n in (part.split(":") for part in limit.split(","))}
        if unknown := set(per) - {repo for _, repo, _ in found}:
            sys.exit(f"{ARM}: BILBO_EVAL_SESSIONS names repositories the corpus lacks: {sorted(unknown)}")
        taken, kept = Counter(), []
        for start, repo, path in found:
            if taken[repo] < per.get(repo, 0):
                taken[repo] += 1
                kept.append((start, repo, path))
        found = kept
    elif limit:
        found = found[: int(limit)]
    skipped = [{"file": f"{repo}/{path.name}", "reason": "no timestamp (Cursor)"} for _, repo, path in other]
    return found, skipped


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


# --- the LLM, through a loopback gateway ---------------------------------------------------------------------------------


def append_escaped(path, row, lock=None):
    """writer.append_jsonl with the separators splitlines() breaks on written as JSON escapes (the row parses the same): a model that
    echoes U+0085, U+2028 or U+2029 into an action, edit or decision must not split a journal row for the writer's own line counts."""
    line = replay.escape_separators(json.dumps(row, ensure_ascii=False, sort_keys=True)) + "\n"
    if lock:
        lock.acquire()
    try:
        with open(path, "a", encoding="utf-8") as f:
            f.write(line)
            f.flush()
            os.fsync(f.fileno())
    finally:
        if lock:
            lock.release()


writer.append_jsonl = append_escaped


def seedless(body):
    """sha256 of a request without its seed: the writer retries a step whose output was invalid with a new seed, and a
    refused request must not go out again under one."""
    try:
        doc = json.loads(body)
    except ValueError:
        return hashlib.sha256(body).hexdigest()
    if isinstance(doc, dict):
        doc.pop("seed", None)
    return hashlib.sha256(json.dumps(doc, sort_keys=True, ensure_ascii=False).encode()).hexdigest()


def refusals():
    """The refusals both arms recorded in /decisions/refusals.jsonl."""
    if not REFUSALS.exists():
        return []
    rows = []
    for line in REFUSALS.read_text(encoding="utf-8").split("\n"):
        try:
            rows.append(json.loads(line))
        except ValueError:
            pass  # a cut last line
    return rows


def error_code(data):
    try:
        return (json.loads(data).get("error") or {}).get("code") or ""
    except (ValueError, AttributeError):
        return ""


def usage_totals(usages):
    """The counters of every label of the given usage documents, added up, and the same split by stage."""
    zero = {"requests": 0, "answered": 0, "cached": 0, "prompt_tokens": 0, "completion_tokens": 0}
    total, by_stage, status = dict(zero), {}, Counter()
    for usage in usages:
        for c in usage.values():
            stage = by_stage.setdefault(c.get("stage") or "run", dict(zero))
            for k in zero:
                total[k] += c[k]
                stage[k] += c[k]
            status.update(c["status"])
    return total | {"status": dict(sorted(status.items()))}, dict(sorted(by_stage.items()))


class Gateway(ThreadingHTTPServer):
    """The writer builds its own L3 labels and sends them as X-L3-Label. Here each call goes to /l/<label>/v1, and the
    gateway sends the body unchanged to the eval's endpoint with the eval's key and headers. An X-L3-Label among those
    headers is the build's base label: the call then goes out as <base>/decide/<project> or <base>/run-<row>/<project>,
    so the endpoint's ledger splits the stages and projects of this build, and a label of L3's never reaches it. An
    upstream answer comes back as it came, with its Retry-After, which writer.Chat waits out; an upstream that cannot be
    reached is a 503 with no body, which it waits out too. Per label, the answered calls, those the endpoint served from a
    cache, the tokens and the statuses are counted, and the lane's usage.json is rewritten after every call so that the
    totals survive a resume.
    A refusal by the model's safeguards (502 l3_refused) is written to /decisions/refusals.jsonl, shared by both arms, and
    that request is never sent again. In the default mode (BILBO_EVAL_ON_REFUSAL=stop) the first one stops every call of
    both builds with a 403, which the writer treats as a stop; in `record` mode the refusal reaches the writer as a model
    failure, up to BILBO_EVAL_REFUSAL_LIMIT of them. A call cap of the endpoint (429) is also turned into a 403: the
    writer would wait out a 429, and a cap is for the lead to raise."""

    daemon_threads = True
    LABEL = re.compile(r"^l3s/([^/]+)/[^/]+/([^/]+)/r\d+$")

    def __init__(self, upstream, key, headers, usage_path):
        super().__init__(("127.0.0.1", 0), Forward)
        self.upstream, self.key = upstream.rstrip("/"), key
        self.base = next((v for k, v in headers.items() if k.lower() == "x-l3-label"), None)
        self.headers = {k: v for k, v in headers.items() if k.lower() != "x-l3-label"}
        self.usage_path, self.lock = Path(usage_path), threading.Lock()
        self.usage = json.loads(self.usage_path.read_text()) if self.usage_path.exists() else {}

    def base_for(self, label):
        return f"http://127.0.0.1:{self.server_address[1]}/l/{quote(label, safe='')}/v1"

    def route(self, label):
        """(the label the endpoint sees or None, the stage) of a writer label; None for a label the writer never builds
        when a base label is set."""
        m = self.LABEL.match(label)
        if not self.base:
            return None, "decide" if m and m[1] == "decide" else "run"
        if not m or m[1] not in ("decide", "run"):
            return None
        return (f"{self.base}/decide/{m[2]}" if m[1] == "decide" else f"{self.base}/run-{ROW}/{m[2]}"), m[1]

    def local(self, status, code, message, label, stage):
        data = json.dumps({"error": {"code": code, "message": message}}).encode()
        self.record(label, stage, status, False, data)
        return status, {"Content-Type": "application/json"}, data

    def stopped(self, label, stage, rows):
        return self.local(403, "bilbo_refusal_stop", f"{len(rows)} refusals in {REFUSALS}; the lead decides", label, stage)

    def send(self, label, body):
        routed = self.route(label)
        if routed is None:
            return 404, {}, b'{"error": {"code": "not_found"}}'
        upstream_label, stage = routed
        count = upstream_label or label
        rows = refusals()
        if rows and (ON_REFUSAL == "stop" or len(rows) > REFUSAL_LIMIT):
            return self.stopped(count, stage, rows)
        sha = seedless(body)
        # Identical bodies of the two arms (the first step of an agent-triggered run is the same text in both) go one at a time, so a
        # request the model refuses is refused once: the second copy waits here, re-reads the refusals and gets the local 502.
        INFLIGHT.mkdir(parents=True, exist_ok=True)
        with open(INFLIGHT / f"{sha}.lock", "w") as twin:
            fcntl.flock(twin, fcntl.LOCK_EX)
            rows = refusals()
            if rows and (ON_REFUSAL == "stop" or len(rows) > REFUSAL_LIMIT):
                return self.stopped(count, stage, rows)
            if any(r.get("sha") == sha for r in rows):
                return self.local(502, "l3_refused", "refused before; not sent again", count, stage)
            return self.upstream_call(body, sha, upstream_label, count, stage)

    def upstream_call(self, body, sha, upstream_label, count, stage):
        extra = {"X-L3-Label": upstream_label} if upstream_label else {}
        request = urllib.request.Request(
            self.upstream + "/chat/completions", body, method="POST",
            headers={"Content-Type": "application/json", "Authorization": f"Bearer {self.key}"} | self.headers | extra,
        )  # fmt: skip
        try:
            with urllib.request.urlopen(request, timeout=1700) as r:
                status, headers, data = r.status, r.headers, r.read()
        except urllib.error.HTTPError as e:
            status, headers, data = e.code, e.headers, e.read()
        except (OSError, http.client.HTTPException) as e:  # refused, reset or timed out
            log(f"the LLM endpoint is unreachable: {e}")
            status, headers, data = 503, {}, b""
        code = error_code(data) if status in (429, 502) else ""
        if (status == 429 and code in ("l3_claude_call_cap", "l3_call_cap")) or (status == 502 and code == "l3_cap_after_retry"):
            log(f"{count}: the endpoint's call cap is reached ({code}); the lead raises it and runs again")
            return self.local(403, code, "the endpoint's call cap is reached", count, stage)
        self.record(count, stage, status, headers.get("x-l3-cache") == "hit", data)
        if status == 502 and code == "l3_refused":
            self.refused(count, sha)
            rows = refusals()
            if ON_REFUSAL == "stop" or len(rows) > REFUSAL_LIMIT:
                return self.stopped(count, stage, rows)
        return status, headers, data

    def refused(self, label, sha):
        log(f"REFUSED: the model's safeguards refused a call of {label} (request {sha[:12]}); recorded in {REFUSALS}")
        REFUSALS.parent.mkdir(parents=True, exist_ok=True)
        with open(REFUSALS.with_suffix(".lock"), "w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            if not any(r.get("sha") == sha for r in refusals()):
                writer.append_jsonl(REFUSALS, {"time": datetime.now(timezone.utc).isoformat(timespec="seconds"), "arm": ARM, "lane": LANE, "label": label, "sha": sha})

    def record(self, label, stage, status, cached, data):
        usage = {}
        if status == 200:
            try:
                usage = json.loads(data).get("usage") or {}
            except ValueError:
                pass
        with self.lock:
            c = self.usage.setdefault(label, {"stage": stage, "requests": 0, "answered": 0, "cached": 0, "prompt_tokens": 0, "completion_tokens": 0, "status": {}})
            c["requests"] += 1
            c["status"][str(status)] = c["status"].get(str(status), 0) + 1
            if status == 200:
                c["answered"] += 1
                c["cached"] += cached
                c["prompt_tokens"] += usage.get("prompt_tokens") or 0
                c["completion_tokens"] += usage.get("completion_tokens") or 0
            writer.write_atomic(self.usage_path, json.dumps(self.usage, indent=1, sort_keys=True) + "\n")

    def totals(self):
        with self.lock:
            return usage_totals([self.usage])[0]


def retry_after(value):
    """Seconds for the writer, which reads a Retry-After as a number: a date becomes the seconds until it, anything else
    is dropped (the writer then waits 60 s)."""
    if not value:
        return None
    try:
        x = float(value)
        return max(0.0, x) if isfinite(x) else None
    except ValueError:
        pass
    try:
        until = parsedate_to_datetime(value)
        return max(0, int((until - datetime.now(until.tzinfo or timezone.utc)).total_seconds()))
    except (TypeError, ValueError):
        return None


class Forward(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    PATH = re.compile(r"^/l/([^/]+)/v1/chat/completions$")

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("Content-Length") or 0))
        m = self.PATH.match(self.path)
        if not m:
            return self.answer(404, {}, b'{"error": {"code": "not_found"}}')
        self.answer(*self.server.send(unquote(m[1]), body))

    def answer(self, status, headers, data):
        self.send_response(status)
        self.send_header("Content-Type", headers.get("Content-Type") or "application/json")
        wait = retry_after(headers.get("Retry-After"))
        if wait is not None:
            self.send_header("Retry-After", str(int(wait)) if wait == int(wait) else str(wait))
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        pass


# --- rendering ------------------------------------------------------------------------------------------------------------


def render_all(plan, data, only=None):
    """L3's renderer, unchanged, over the kept sessions of each repo (only the one named, for a lane): the transcripts as
    replay.py fixes them, written under their session ids (linked when nothing changed) and a selection.json as the
    renderer reads it, then each exchanges file with its separators escaped, since schema.py and the journals split lines
    on them. Returns ({repo: {"exchanges": path}} for the repos that render to at least one exchange, {file: what replay
    changed})."""
    shutil.rmtree(data, ignore_errors=True)
    raw = data / "raw" / "transcripts"
    raw.mkdir(parents=True)
    repos, changes = {}, {}
    for _, key, path in plan:
        if only and key != only:
            continue
        m = NAME.match(path.name)
        sid = m[5]
        text, changed = replay.transcript_text(path)
        if changed:
            (raw / f"{sid}.jsonl").write_text(text, encoding="utf-8")
            changes[f"{key}/{path.name}"] = changed
        else:
            (raw / f"{sid}.jsonl").symlink_to(path)
        repos.setdefault(key.replace("__", "/", 1), {"key": key, "sessions": []})["sessions"].append(
            {"session_id": sid, "created_at": f"{m[1]}{m[2]}:{m[3]}:{m[4]}Z"}
        )
    (data / "selection.json").write_text(json.dumps({"dev": "", "test": [], "repos": repos}, indent=1))
    out = {}
    for rid in sorted(repos):
        key = repos[rid]["key"]
        render.main(["--data", str(data), "--selection", str(data / "selection.json"), "--repo", rid, "--out", str(data / "exchanges")])
        path = data / "exchanges" / f"{key}.jsonl"
        replay.escape_file(path)
        if path.stat().st_size:
            out[key] = {"exchanges": path}
    return out, changes


# --- resuming ---------------------------------------------------------------------------------------------------------


def trim_tail(path):
    """A kill can cut the append of a journal row: drop the bytes after the last newline, which writer.jsonl would
    choke on."""
    if not path.exists():
        return False
    data = path.read_bytes()
    keep = data[: data.rfind(b"\n") + 1]
    if keep == data:
        return False
    tmp = path.with_name(path.name + ".tmp")
    tmp.write_bytes(keep)
    os.replace(tmp, path)
    return True


def restore_half(state, bilbo, skill, embedder):
    """A run that stopped inside the first exchange of a session may leave the note file it was writing in the store, with
    no done row to say the session had begun, and the writer would then snapshot the store with that orphan in it
    (L3's defect L15, writer.py:1002-1008). When snap/meta.json names a session with no done row, restore its snapshot
    the way the writer restores a half-done session: back to the store and journals the session started from, then
    check and index. Returns the session restored, or None."""
    st = writer.State(Path(state).resolve())
    meta = st.snap / "meta.json"
    if not meta.exists():
        return None
    session = json.loads(meta.read_text())["session"]
    if any(r["xid"].startswith(f"{session}#") for r in writer.jsonl(st.done)):
        return None
    sb = writer.make_sandbox(st.dir, bilbo, embedder, "bilbo")
    tool = writer.load_tool("bilbo", skill, sb.vstore)
    if not writer.restore_snapshot(st, sb, session):
        return None
    writer.after_run(sb, tool)
    return session


def schedule_for(key, rows):
    """The assisted row's /note schedule of a project: the last exchange of every session. L3 wrote its schedule for the five
    old projects only; this rule gives that file exactly there, and the new projects get theirs from the same rule. Stops
    when either does not hold."""
    derived = [s[-1]["xid"] for s in writer.group_sessions(rows)]
    l3 = {r["xid"] for r in schema.load_schedule(L3 / "policy" / "schedule.jsonl")}
    if key in OLD and set(derived) != l3 & {r["xid"] for r in rows}:
        sys.exit(f"{ARM}: the last exchange of every session of {key} is not what l3/policy/schedule.jsonl holds")
    if key not in OLD and set(derived) & l3:
        sys.exit(f"{ARM}: l3/policy/schedule.jsonl names exchanges of {key}, which L3 did not write")
    return [{"request": "/note", "xid": x} for x in derived]


def write_schedule(path, schedule):
    writer.write_atomic(path, "".join(json.dumps(r) + "\n" for r in schedule))


def seed_manifest():
    return json.loads((SEED / "manifest.json").read_text()) if (SEED / "manifest.json").exists() else None


def check_seed(sm, key, exchanges, rows, model=None):
    """Stops unless the seed is the published store of this row, built by the same writer, skill and model, and its
    project renders to the exchanges it was written from, all of which it has done."""
    bad = []
    if sm["arm"] != ARM:
        bad.append(f"the seed is the store of {sm['arm']}")
    if model and sm["llm"]["model"] != model:
        bad.append(f"the seed was written by {sm['llm']['model']}, this build by {model}")
    if sm["method"]["l3_files"] != L3_FILES:
        bad.append("the seed was written by other L3 files")
    if sm["method"]["skill"]["sha256"] != writer.SKILL_SHA256:
        bad.append("the seed was written with another note skill")
    if key not in sm["repos"]:
        bad.append(f"the seed holds no {key}")
    else:
        if sm["repos"][key]["exchanges_sha256"] != sha256(exchanges):
            bad.append(f"{key} renders to other exchanges than the seed's")
        done = {r["xid"] for r in writer.jsonl(SEED / "writer" / key / "done.jsonl")}
        if done != {r["xid"] for r in rows}:
            bad.append(f"the seed's done journal of {key} is not the rendered exchanges")
    if bad:
        sys.exit(f"{ARM}: cannot continue the seed: {'; '.join(bad)}")


def restore_seed(key, exchanges, rows, model):
    """Continue the published store for a project it holds: its journals, decisions, traces and notes, copied into the
    state the writer resumes from and compared byte for byte with the seed's. The writer then finds every exchange done
    and calls nothing. The shared decisions file gets the seed's decisions under its lock (the other arm's seed holds the
    same bytes). Does nothing when the state already exists. Returns True when it restored the project."""
    state = WORK / "states" / key
    sm = seed_manifest()
    if sm is None or key not in sm["repos"] or state.exists():
        return False
    check_seed(sm, key, exchanges, rows, model)
    src, info = SEED / "writer" / key, sm["repos"][key]
    tmp = state.with_name(key + ".seed.tmp")
    shutil.rmtree(tmp, ignore_errors=True)
    pairs = []
    for f in (*JOURNALS, "manifest.json", "provenance.json"):
        if (src / f).exists():
            pairs.append((src / f, tmp / f))
    pairs.append((src / "decisions.jsonl", tmp / "decisions" / "bilbo.jsonl"))
    if (src / "traces").is_dir():
        pairs += [(t, tmp / "traces" / t.name) for t in sorted((src / "traces").iterdir())]
    pairs += [(SEED / "notes" / n, tmp / "store" / "notes" / n) for n in info["notes"]]
    for a, b in pairs:
        b.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(a, b)
        if sha256(a) != sha256(b):
            sys.exit(f"{ARM}: {b} differs from the seed's {a}")
    shared = writer.decisions_file(DECISIONS, key, 1, "bilbo", model)
    shared.parent.mkdir(parents=True, exist_ok=True)
    data = (src / "decisions.jsonl").read_bytes()
    with open(shared.with_suffix(".lock"), "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if shared.exists():
            if shared.read_bytes() != data:
                sys.exit(f"{ARM}: {shared} is not the seed's decisions of {key}")
        else:
            part = shared.with_name(shared.name + ".tmp")
            part.write_bytes(data)
            os.replace(part, shared)
    (LANES / key).mkdir(parents=True, exist_ok=True)
    writer.write_atomic(LANES / key / "seed.json", json.dumps({"notes": len(info["notes"]), "llm_calls": info["llm_calls"]}) + "\n")
    os.replace(tmp, state)
    log(f"{key}: restored from the seed: {len(rows)} exchanges, {len(info['notes'])} notes, no LLM call")
    return True


def require_seed_decisions(lane, state, model):
    """A seeded project is never drawn again: its shared decisions file must exist and be the restored one, else a lost decisions folder
    would make decide() draw the whole pass anew (805 calls for the old five)."""
    if lane not in OLD:
        return
    shared = writer.decisions_file(DECISIONS, lane, 1, "bilbo", model)
    if not shared.exists() or shared.read_bytes() != (state / "decisions" / "bilbo.jsonl").read_bytes():
        sys.exit(f"{ARM}: {lane} is a seeded project and its shared decisions file {shared} is missing or not the restored one; "
                 "drawing it again would cost its whole pass: restore the decisions folder")  # fmt: skip


def decide(p, chat, rows):
    """The decision pass of one project, shared by both arms: under the writer's own lock on the shared file, whichever arm
    comes first draws it and the other finds every row done. A cut last line is dropped under the lock, never beside a
    writer that is still appending. Returns the rows this arm drew."""
    tool, _, sessions, _, _ = writer._load(p)
    out = writer.decisions_path(p, tool)
    out.parent.mkdir(parents=True, exist_ok=True)
    with open(out.with_suffix(".lock"), "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if trim_tail(out):
            log(f"{p.repo}: dropped the half-written last line of {out.name}")
        had = len(writer.jsonl(out))
        try:
            writer.decision_pass(chat, f"l3s/{p.stage or 'decide'}/{tool.name}/{p.repo}/r{p.rep}", tool, sessions, out, p.workers)
        finally:
            try:
                count_drawn(p.repo, len(writer.jsonl(out)) - had)
            except Exception:
                pass
        got = [r["xid"] for r in writer.jsonl(out)]
    if len(got) != len(set(got)) or set(got) != {x["xid"] for x in rows}:
        sys.exit(f"{ARM}: {out} holds {len(got)} rows for {len(rows)} exchanges ({len(got) - len(set(got))} doubled): "
                 "two passes met; stop both builds and look before resuming")  # fmt: skip
    return len(got) - had


def count_drawn(key, n):
    """Add the decisions this arm drew to the lane's tally, which a resumed lane keeps."""
    f = LANES / key / "drawn.json"
    f.parent.mkdir(parents=True, exist_ok=True)
    total = n + (json.loads(f.read_text())["drawn"] if f.exists() else 0)
    writer.write_atomic(f, json.dumps({"drawn": total}) + "\n")


def backup(key):
    """A copy of the project's state, made at the end of a session (the writer is between two sessions then): the newest
    two are kept in /work/backup/<project>."""
    dest = WORK / "backup" / key
    dest.mkdir(parents=True, exist_ok=True)
    for old in dest.glob("*.tmp"):
        shutil.rmtree(old, ignore_errors=True)
    name = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
    shutil.copytree(WORK / "states" / key, dest / f"{name}.tmp", symlinks=True, ignore=shutil.ignore_patterns("snap"))
    os.replace(dest / f"{name}.tmp", dest / name)
    for old in sorted(dest.iterdir())[:-2]:
        shutil.rmtree(old, ignore_errors=True)


# --- the store ---------------------------------------------------------------------------------------------------------


def fnv_name(root):
    """The name bilbo gives a store's vector file: FNV-1a 64 of the normalized root path (vectors.rs:104-107)."""
    h = FNV_OFFSET
    for b in str(PurePosixPath(root)).encode():
        h = ((h ^ b) * FNV_PRIME) & 0xFFFFFFFFFFFFFFFF
    return f"{h:016x}.vectors"


def read_vectors(path):
    """(model name, dims, keys) of a bilbo vector file (vectors.rs:124-172)."""
    data = Path(path).read_bytes()
    if not data.startswith(b"BILBOVEC1\n"):
        raise ValueError(f"{path} is not a bilbo vector file")
    n = len(b"BILBOVEC1\n")
    length = int.from_bytes(data[n : n + 4], "little")
    name = data[n + 4 : n + 4 + length].decode()
    n += 4 + length
    dims, count = int.from_bytes(data[n : n + 4], "little"), int.from_bytes(data[n + 4 : n + 8], "little")
    record = 8 + 4 * dims
    if len(data) != n + 8 + count * record:
        raise ValueError(f"{path} is cut")
    return name, dims, [int.from_bytes(data[n + 8 + i * record : n + 16 + i * record], "little") for i in range(count)]


def bilbo_env(store, scratch):
    """What bilbo sees when it indexes the finished store: its root, config and cache inside the store, nothing of the
    host."""
    scratch = Path(scratch)
    for d in ("home", "config", "data", "state"):
        (scratch / d).mkdir(parents=True, exist_ok=True)
    return {
        "HOME": str(scratch / "home"), "BILBO_HOME": str(store), "BILBO_CONFIG": str(store / "config"),
        "XDG_CACHE_HOME": str(store / "cache"), "XDG_CONFIG_HOME": str(scratch / "config"),
        "XDG_DATA_HOME": str(scratch / "data"), "XDG_STATE_HOME": str(scratch / "state"),
        "PATH": "/usr/bin:/bin", "NO_COLOR": "1", "LANG": "en_US.UTF-8", "LC_ALL": "en_US.UTF-8",
    }  # fmt: skip


def post(embedder, path, body):
    request = urllib.request.Request(embedder + path, json.dumps(body).encode(), {"Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=3600) as r:
        return json.loads(r.read())


def index_store(store, bilbo, embedder, scratch):
    """One sentinel request, then `bilbo index` on a fresh cache. Returns the number of vectors and the file's name."""
    post(embedder, "/v1/embeddings", {"input": [SENTINEL]})
    p = subprocess.run([str(bilbo), "index"], env=bilbo_env(store, scratch), cwd=scratch, capture_output=True, text=True, timeout=7200)
    m = re.search(r"^embedded (\d+), kept (\d+), dropped (\d+)$", p.stdout, re.M)
    if p.returncode != 0 or not m or m[2] != "0" or m[3] != "0":
        sys.exit(f"{ARM}: bilbo index did not embed a fresh cache (exit {p.returncode}): {p.stdout[-300:]!r} {p.stderr[-300:]!r}")
    if not int(m[1]):
        return 0, None
    name = fnv_name(store)
    model, dims, keys = read_vectors(store / "cache" / "bilbo" / name)
    if (model, dims, len(keys)) != (MODEL, DIMS, int(m[1])):
        sys.exit(f"{ARM}: the vector file holds {len(keys)} vectors of {model} ({dims} dims), bilbo embedded {m[1]}")
    return len(keys), f"cache/bilbo/{name}"


def topic_of(name):
    return re.sub(rf"^({KINDS})-", "", name).removesuffix(".md")


def build_store(repos, states, out, bilbo, embedder, scratch):
    """The store: every project's notes in one bilbo root, one fresh index of it, and the writer's journals and traces.
    bilbo allows one note per topic whatever the kind, so a topic two projects share stops the build; the states stay."""
    shutil.rmtree(out, ignore_errors=True)
    (out / "cache").mkdir(parents=True)
    (out / "notes").mkdir()
    config = [("embedder.url", embedder), ("embedder.model", MODEL), ("embedder.query_prefix", QUERY_PREFIX)]
    writer.write_atomic(out / "config", "".join(f"{k} = {writer._quote(v)}\n" for k, v in config))
    names, topics = {}, {}
    for key in sorted(repos):
        state = states / key
        named = {e["path"] for e in writer.jsonl(state / "edits.jsonl")}
        notes = sorted((state / "store" / "notes").glob("*.md"))
        orphans = [f"notes/{n.name}" for n in notes if f"notes/{n.name}" not in named]
        if orphans:
            sys.exit(f"{ARM}: {key} holds notes no edit row names ({', '.join(orphans)}); an interrupted run left them: delete {state} and rebuild")
        for n in notes:
            topic = topic_of(n.name)
            if topic in topics:
                other, file = topics[topic]
                sys.exit(f"{ARM}: {key}/{n.name} and {other}/{file} share the topic {topic!r}, and bilbo keeps one note per topic; "
                         "the states are kept, so a run again after a decision resumes with no LLM call")  # fmt: skip
            topics[topic] = (key, n.name)
            shutil.copyfile(n, out / "notes" / n.name)
        names[key] = [n.name for n in notes]
        dest = out / "writer" / key
        dest.mkdir(parents=True)
        for f in (*JOURNALS, "manifest.json", "provenance.json"):
            if (state / f).exists():
                shutil.copyfile(state / f, dest / f)
        shutil.copyfile(state / "decisions" / "bilbo.jsonl", dest / "decisions.jsonl")
        if (state / "traces").is_dir():  # only a run of the skill makes it
            shutil.copytree(state / "traces", dest / "traces")
    vectors, vector_file = index_store(out, bilbo, embedder, scratch)
    log(f"{sum(map(len, names.values()))} notes, {vectors} vectors")
    return names, vectors, vector_file


def versions(embedder):
    return {
        "bilbo": "0.19.0", "bilbo_sha256": sha256(BILBO), "arch": platform.machine(), "python": platform.python_version(),
        "tiktoken": version("tiktoken"), "llama_cpp": json.loads(urllib.request.urlopen(embedder + "/props", timeout=60).read())["build_info"],
    }  # fmt: skip


SETTINGS = [
    {"key": "embedder.url / embedder.model", "value": "the eval's Qwen3-Embedding-0.6B Q8_0 (qwen3-embedding-0.6b)",
     "cite": "bilbo's pinned local model, src/host/model.rs:7-22; docs/guides/embedders.md:18,93-110"},
    {"key": "embedder.query_prefix", "value": QUERY_PREFIX, "cite": "src/shared/config.rs:37,881-888; docs/guides/embedders.md:49-50"},
    {"key": "embedder.min_similarity", "value": "0.5 (default, unset)", "cite": "src/shared/config.rs:6; docs/reference/configuration.md:26"},
    {"key": "scopes", "value": "none",
     "cite": "a non-loopback embedder.url withholds nothing without a local scope: src/search/vectors.rs:37; docs/guides/embedders.md:125-128"},
    {"key": "bilbo index", "value": "after every writer run, and once over the merged store",
     "cite": "setup's timer runs it every 15 min (docs/guides/embedders.md:71-77); unindexed passages rank by keywords only (:90)"},
    {"key": "note skill", "value": f"plugins/bilbo/skills/note/SKILL.md at v0.19.0, sha256 {writer.SKILL_SHA256}", "cite": "the plugin as shipped"},
    {"key": "compaction hook", "value": "its text before the first prompt after a compaction",
     "cite": "plugins/bilbo/hooks/hooks.json at v0.19.0 (writer.HOOK_TEXT)"},
    {"key": "digest hook", "value": "off in the build", "cite": "it only reads (src/search/digest.rs); a search-time lever"},
]  # fmt: skip

SEARCH = [
    "bilbo recall [--limit k] -- <question> per question, with this store's config",
    "embedder.query_prefix as in config; embedder.min_similarity 0.5",
    "on llama.cpp b11206 a query vector depends on the request before it (cosine 0.99972 against the same query after a "
    "sentinel): a searcher that wants history-free queries sends one sentinel request before each bilbo recall",
    "--kind filters exist; bilbo digest (the prompt hook) is the other read path, gated at 0.55; no rerank, no recency",
]

INDEX_NOTE = (
    "llama.cpp b11206 reuses the prefix a passage shares with the passage embedded before it (bilbo embeds a store's "
    "passages in a fixed order, 16 to a request), so a vector can differ from its text's vector alone by up to 0.0075 per "
    "dimension (cosine 0.999 or more); one sentinel request before the index makes the vectors depend only on the notes."
)


def read_json(path, default):
    path = Path(path)
    return json.loads(path.read_text()) if path.exists() else default


def manifest(repos, found, embedder, skipped, changes):
    names, vectors, vector_file = found
    inputs = (WORK / "key.txt").read_text().splitlines()
    by_repo, lanes, usages, wall = {}, {}, [], 0.0
    drawn, seeded = {}, {}
    for key in sorted(repos):
        state = WORK / "states" / key
        w = json.loads((state / "manifest.json").read_text())
        rows = schema.load_exchanges(repos[key]["exchanges"])
        created = sorted(writer.split_frontmatter((WORK / "store" / "notes" / n).read_text())[0]["created"] for n in names[key])
        drawn[key] = read_json(LANES / key / "drawn.json", {"drawn": 0})["drawn"]
        if (LANES / key / "seed.json").exists():
            seeded[key] = read_json(LANES / key / "seed.json", {})
        entry = {
            "sessions": sorted({r["session_id"] for r in rows}, key=lambda s: next(r["s"] for r in rows if r["session_id"] == s)),
            "exchanges": len(rows), "exchanges_sha256": sha256(repos[key]["exchanges"]), "notes": names[key],
            "notes_count": len(names[key]), "writer": w["counts"],
            "llm_calls": w["counts"]["decisions"] + w["counts"]["run_llm_calls"], "created": [created[0], created[-1]] if created else None,
            "decide_drawn_here": drawn[key] > 0,
        }  # fmt: skip
        if (state / "provenance.json").exists():
            entry["provenance"] = json.loads((state / "provenance.json").read_text())
        by_repo[key] = entry
        usage = read_json(LANES / key / "usage.json", {})
        usages.append(usage)
        lanes[key] = {"usage": usage_totals([usage])[0], "wall_s": w["wall_s"]}
        wall += w["wall_s"]
    total, by_stage = usage_totals(usages)
    seed = None
    if seeded:
        sm = seed_manifest()
        line = next((i.split() for i in inputs[1:] if i.startswith("seed ")), None)
        seed = {
            "archive": line[1] if line else None, "sha256": line[2] if line else None, "key": sm["key"], "repos": sorted(seeded),
            "notes": sum(v["notes"] for v in seeded.values()), "llm_calls": sum(v["llm_calls"] for v in seeded.values()),
        }  # fmt: skip
    schedule = None
    if ROW == "assisted":
        schedule = ("a bare /note after the last exchange of every session, derived from the exchanges; on the five L3 projects it is "
                    "l3/policy/schedule.jsonl exactly")  # fmt: skip
    return {
        "arm": ARM, "key": inputs[0], "inputs": inputs[1:], "device": os.environ["BILBO_EVAL_DEVICE"], "versions": versions(embedder),
        "method": {
            "row": ROW, "l3_commit": L3_COMMIT, "l3_commit_note": "local to the L3 eval's branch: the files are identified by l3_files", "l3_files": L3_FILES,
            "skill": {"path": writer.SKILL_PATH, "tag": writer.SKILL_TAG, "sha256": writer.SKILL_SHA256},
            "render_sha": render.render_sha(), "cap": render.CAP, "encoding": render.ENCODING, "decide_window": writer.DECIDE_WINDOW,
            "run_window": writer.RUN_WINDOW, "max_steps": writer.MAX_STEPS, "effort": "default",
            "schedule": schedule,
        },
        "settings": SETTINGS,
        "index": {
            "root": "/work/store", "cache": "cache/bilbo/<FNV-1a 64 of the root as 16 hex digits>.vectors", "vector_file": vector_file, "model": MODEL,
            "dims": DIMS, "query_prefix": QUERY_PREFIX, "sentinel": SENTINEL, "note": INDEX_NOTE,
            "relocate": "a search at another root renames the file to that root's hash",
        },
        "search": SEARCH,
        "sessions": sum(len(r["sessions"]) for r in by_repo.values()),
        "skipped": skipped,
        "seed": seed,
        "decisions": {"shared": "bilbo-v0_19", "folder": os.environ.get("BILBO_EVAL_DECISIONS_NAME", ""), "drawn_here": drawn},
        "replay": {
            "separators_escaped": {f: c["separators"] for f, c in changes.items() if "separators" in c},
            "teleported": {f: c["teleported_records"] for f, c in changes.items() if "teleported_records" in c},
        },
        "refusals": [r for r in refusals() if r.get("arm") == ARM],
        "lanes": lanes,
        "repos": by_repo,
        "notes": sum(r["notes_count"] for r in by_repo.values()),
        "vectors": vectors,
        "llm": {"model": os.environ["BILBO_EVAL_LLM_MODEL"], "calls": sum(r["llm_calls"] for r in by_repo.values()), "gateway": total, "by_stage": by_stage},
        "ingest_seconds": round(wall),
        "finished": datetime.now(timezone.utc).isoformat(timespec="seconds"),
    }  # fmt: skip


# --- main -------------------------------------------------------------------------------------------------------------


def progress(rows, key, gateway):
    """The writer's step hook: at the end of each session one line, counted from the done journal, and a backup of the
    project's state. It never raises."""
    last = {}
    for r in rows:
        last[r["session_id"]] = r["xid"]
    ends = set(last.values())
    failed = []

    def hook(point, xid):
        try:
            if point != "exchange_done" or xid not in ends:
                return
            finished = {r["xid"] for r in writer.jsonl(WORK / "states" / key / "done.jsonl")}
            log(f"{key} {len(ends & finished)}/{len(ends)} sessions, {len(finished)}/{len(rows)} exchanges, "
                f"{gateway.totals()['answered']} LLM calls")  # fmt: skip
            try:
                backup(key)
            except Exception as e:
                if not failed:
                    failed.append(e)
                    log(f"{key}: no backup of the state: {e}")
        except Exception:
            pass

    return hook


def guards():
    if ROW not in ("agent", "assisted"):
        sys.exit("bilbo: BILBO_EVAL_ROW must be agent or assisted")
    if ON_REFUSAL not in ("stop", "record"):
        sys.exit(f"{ARM}: BILBO_EVAL_ON_REFUSAL must be stop or record")
    for name, want in L3_FILES.items():
        if sha256(L3 / name) != want:
            sys.exit(f"{ARM}: {name} is not the file the L3 eval froze at {L3_COMMIT}")
    if not DRY and sha256(SKILL) != writer.SKILL_SHA256:
        sys.exit(f"{ARM}: {SKILL} is not the note skill of {writer.SKILL_TAG}")


def stop(signum, frame):
    """Leave at once: a pool of decision calls would otherwise drain its whole queue first. The checkpoint tolerates a cut
    (trim_tail, restore_half, the writer's snapshot restore, atomic usage.json)."""
    log("stopped; run again to resume")
    os._exit(130)


def llm_settings():
    llm_url = os.environ["BILBO_EVAL_LLM_URL"].rstrip("/")
    llm_key = os.environ["BILBO_EVAL_LLM_KEY"]
    llm_headers = json.loads(os.environ.get("BILBO_EVAL_LLM_HEADERS") or "{}")
    try:
        request = urllib.request.Request(f"{llm_url}/models", headers={"Authorization": f"Bearer {llm_key}"} | llm_headers)
        urllib.request.urlopen(request, timeout=30).close()
    except (OSError, http.client.HTTPException) as e:
        sys.exit(f"{ARM}: the LLM endpoint {llm_url} does not answer: {e}")
    return llm_url, llm_key, llm_headers


def lane_main(lane):
    """One project, in this container: restore it from the seed when the seed holds it, draw (or find) its decision pass,
    write its sessions. The merge builds the store from the states of all lanes."""
    guards()
    llm_model = os.environ["BILBO_EVAL_LLM_MODEL"]
    llm_url, llm_key, llm_headers = llm_settings()
    embedder = os.environ["EMBEDDER_URL"].rstrip("/")
    plan = [x for x in sessions()[0] if x[1] == lane]
    if not plan:
        sys.exit(f"{ARM}: the first {os.environ.get('BILBO_EVAL_SESSIONS') or 'all'} sessions hold none of {lane}")
    (LANES / lane).mkdir(parents=True, exist_ok=True)
    held = open(LANES / lane / ".lock", "w")
    try:
        fcntl.flock(held, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        sys.exit(f"{ARM}: {lane} is being built by another container")
    repos = render_all(plan, LANES / lane / "data", only=lane)[0]
    if lane not in repos:
        return log(f"{lane}: its sessions render to no exchange")
    exchanges = repos[lane]["exchanges"]
    rows = schema.load_exchanges(exchanges)
    schedule = None
    if ROW == "assisted":
        schedule = LANES / lane / "schedule.jsonl"
        write_schedule(schedule, schedule_for(lane, rows))
    gateway = Gateway(llm_url, llm_key, llm_headers, LANES / lane / "usage.json")
    threading.Thread(target=gateway.serve_forever, daemon=True).start()
    chat = writer.Chat(gateway.base_for)
    state = WORK / "states" / lane
    restore_seed(lane, exchanges, rows, llm_model)
    if any([trim_tail(state / j) | replay.escape_file(state / j) for j in JOURNALS]):
        log(f"{lane}: dropped the half-written last line of a journal, or escaped a separator in one")
    p = writer.Params(
        arm=L3_ARM, repo=lane, state=state, exchanges=exchanges, render_sha=render.render_sha(), decisions_dir=DECISIONS,
        bilbo=BILBO, skill=SKILL, embedder=(embedder, MODEL, QUERY_PREFIX), schedule=schedule,
        rep=1, stage=None, workers=8, model=llm_model, effort=None,
    )  # fmt: skip
    writer.STEP_HOOK = progress(rows, lane, gateway)
    require_seed_decisions(lane, state, llm_model)
    try:
        decide(p, chat, rows)
        if (session := restore_half(state, BILBO, SKILL, p.embedder)) is not None:
            log(f"{lane}: an interrupt stopped session {session} in its first exchange: restored the store it started from")
        writer.run_capture(p, chat)
    except (writer.WriterError, schema.SchemaError) as e:
        sys.exit(f"{ARM}: {lane}: {e}; run again to resume")
    log(f"{lane}: written")


def merge_main():
    """The store from the lanes' states, once every project is written."""
    guards()
    embedder = os.environ["EMBEDDER_URL"].rstrip("/")
    plan, skipped = sessions()
    repos, changes = render_all(plan, WORK / "data")
    unfinished = []
    for key in sorted(repos):
        mf = WORK / "states" / key / "manifest.json"
        done = {r["xid"] for r in writer.jsonl(WORK / "states" / key / "done.jsonl")}
        have = json.loads(mf.read_text()) if mf.exists() else {}
        want = {r["xid"] for r in schema.load_exchanges(repos[key]["exchanges"])}
        if not have.get("done") or done != want:
            unfinished.append(key)
        elif have["exchanges_sha256"] != sha256(repos[key]["exchanges"]):
            sys.exit(f"{ARM}: {key} was written from other exchanges than the corpus renders to now")
    if unfinished:
        sys.exit(f"{ARM}: lanes not finished: {', '.join(unfinished)}; run again to resume them")
    states = WORK / "states"
    found = build_store(repos, states, WORK / "store", BILBO, embedder, Path("/tmp/index-home"))
    (WORK / "store" / "manifest.json").write_text(json.dumps(manifest(repos, found, embedder, skipped, changes), indent=2) + "\n")
    log(f"done: {WORK / 'store'}")


def dry_main():
    """What the build would do, with no LLM, no Docker and no binary: every exchange rendered, the calls counted per
    project, and the seed and the schedule checked. The table is the checkpoint the live run is compared with."""
    guards()
    scratch = Path(tempfile.mkdtemp(prefix="bilbo-dry-"))
    try:
        plan, skipped = sessions()
        repos, changes = render_all(plan, scratch / "data")
        stats = json.loads((scratch / "data" / "rendered_stats.json").read_text())
        sm = seed_manifest()
        print(f"{'project':34} {'sess':>4} {'exch':>5} {'/note':>5} {'decide window tok':>17}  old-five sha vs seed   drops")
        tot, table = Counter(), []
        for rid in sorted({k.replace("__", "/", 1) for _, k, _ in plan}):
            key = rid.replace("/", "__", 1)
            if key not in repos:
                table.append((key, 0, 0, 0, 0, "", stats[rid]["dropped"]))
                continue
            rows = schema.load_exchanges(repos[key]["exchanges"])
            sess = writer.group_sessions(rows)
            notes = len(schedule_for(key, rows))
            window = sum(y["tokens"] for s in sess for i in range(len(s)) for y in writer.window(s, i, writer.DECIDE_WINDOW))
            table.append((key, len(sess), len(rows), notes, window, sha256(repos[key]["exchanges"]), stats[rid]["dropped"]))
            if sm and key in OLD:
                check_seed(sm, key, repos[key]["exchanges"], rows)
        for key, ns, nx, nn, wt, sha, drops in table:
            verdict = ""
            if key in OLD and sm and key in sm["repos"]:
                verdict = "EQUAL" if sm["repos"][key]["exchanges_sha256"] == sha else "DIFFERS"
            print(f"{key:34} {ns:4} {nx:5} {nn:5} {wt:17,}  {verdict:20}   {drops}")
            part = "old" if key in OLD else "new"
            tot[part + "_sessions"] += ns
            tot[part + "_exchanges"] += nx
            tot[part + "_notes"] += nn
            tot[part + "_window_tokens"] += wt
        print(dict(tot))
        if all(k in repos for k in OLD):
            print("schedule rule (last exchange of every session) on the old five: EQUALS l3/policy/schedule.jsonl")
        print("transcripts changed by the fixes:", sorted(changes))
        print("skipped (no timestamp):", [r["file"] for r in skipped])
        print(f"\n{'new project':34} {'sessions':>8} {'exchanges = decide calls':>25} {'decide window M':>16} {'decide est. M':>14}")
        out = []
        for key, ns, nx, nn, wt, sha, drops in sorted((t for t in table if t[0] not in OLD), key=lambda t: t[2]):
            est = wt + DECIDE_OVERHEAD * nx
            print(f"{key:34} {ns:8} {nx:25} {wt / 1e6:16.2f} {est / 1e6:14.1f}")
            out.append({"project": key, "sessions": ns, "exchanges": nx, "notes": nn, "window_tokens": wt, "decide_estimate_tokens": est})
        new = [t for t in table if t[0] not in OLD]
        print(f"{'new, in all':34} {sum(t[1] for t in new):8} {sum(t[2] for t in new):25} {sum(t[4] for t in new) / 1e6:16.2f} "
              f"{(sum(t[4] for t in new) + DECIDE_OVERHEAD * sum(t[2] for t in new)) / 1e6:14.1f}")  # fmt: skip
        if os.environ.get("BILBO_EVAL_DRY_OUT"):
            Path(os.environ["BILBO_EVAL_DRY_OUT"]).write_text(json.dumps({"row": ROW, "decide_overhead": DECIDE_OVERHEAD, "projects": out}, indent=1) + "\n")
    finally:
        shutil.rmtree(scratch, ignore_errors=True)


def main():
    args = sys.argv[1:]
    if DRY:
        return dry_main()
    guards()
    if args == ["--lanes"]:
        for repo in sorted({repo for _, repo, _ in sessions()[0]}):
            print(repo)
        return
    signal.signal(signal.SIGINT, stop)
    signal.signal(signal.SIGTERM, stop)
    if args == ["--merge"]:
        return merge_main()
    if LANE and not args:
        return lane_main(LANE)
    sys.exit(f"{ARM} builds in lanes: run it through scripts/build-store.sh")


if __name__ == "__main__":
    main()
