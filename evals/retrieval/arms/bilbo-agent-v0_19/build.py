"""Build the bilbo store: the L3 sessions eval's bilbo writer (l3/, copied byte for byte), driven over this eval's corpus.
Every project is written on its own, session by session: after each exchange a call asks the model whether
the agent would use bilbo's note skill (the decision pass), and where it would, a tool loop runs the skill as shipped
against the real bilbo binary. The assisted row adds a bare /note after the last exchange of every session. The LLM
calls go through a loopback gateway to BILBO_EVAL_LLM_URL. Once every project is written, their notes are merged
into one bilbo store, indexed once with the eval's embedder and saved with the writer's journals and a manifest in /work/store. The writer checkpoints
after every exchange in /work/states, and a run that finds them resumes from there."""

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

sys.path.insert(0, "/app/l3")

import render  # noqa: E402
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

CORPUS = Path("/corpus")
WORK = Path("/work")
L3 = Path("/app/l3")
SKILL = Path("/app/skill/SKILL.md")
BILBO = Path("/usr/local/bin/bilbo")


def log(msg):
    print(f"{ARM}: {msg}", file=sys.stderr, flush=True)


def stamp(entry):
    return datetime.fromisoformat(entry["timestamp"]) if entry.get("timestamp") else None


def sessions():
    """Every Claude Code session as (start, repo, path), in the order they started across all repos, as for mem0.
    A transcript with no timestamp (Cursor's) is left out, as the other arms do."""
    found, other = [], []
    for path in sorted(CORPUS.glob("*/*.jsonl")):
        start = next((t for t in map(stamp, map(json.loads, path.open())) if t), None)
        (found if start else other).append((start, path.parent.name, path))
    found.sort()
    limit = os.environ.get("BILBO_EVAL_SESSIONS")
    skipped = [{"file": f"{repo}/{path.name}", "reason": "no timestamp (Cursor)"} for _, repo, path in other]
    return found[: int(limit) if limit else None], skipped


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


# --- the LLM, through a loopback gateway ---------------------------------------------------------------------------------


class Gateway(ThreadingHTTPServer):
    """The writer builds its own L3 labels and sends them as X-L3-Label. Here each call goes to /l/<label>/v1, and the
    gateway sends the body unchanged to the eval's endpoint with the eval's key and headers: a label of L3's never reaches
    the endpoint, where it would replay or bill L3's own records. An upstream answer comes back as it came, with its
    Retry-After, which writer.Chat waits out; an upstream that cannot be reached is a 503 with no body, which it waits
    out too. Per label, the answered calls, those the endpoint served from a cache, the tokens and the statuses are
    counted, and /work/usage.json is rewritten after every call so that the totals survive a resume."""

    daemon_threads = True

    def __init__(self, upstream, key, headers, usage_path):
        super().__init__(("127.0.0.1", 0), Forward)
        self.upstream, self.key, self.headers = upstream.rstrip("/"), key, headers
        self.usage_path, self.lock = Path(usage_path), threading.Lock()
        self.usage = json.loads(self.usage_path.read_text()) if self.usage_path.exists() else {}

    def base_for(self, label):
        return f"http://127.0.0.1:{self.server_address[1]}/l/{quote(label, safe='')}/v1"

    def send(self, label, body):
        request = urllib.request.Request(
            self.upstream + "/chat/completions", body, method="POST",
            headers={"Content-Type": "application/json", "Authorization": f"Bearer {self.key}"} | self.headers,
        )  # fmt: skip
        try:
            with urllib.request.urlopen(request, timeout=1700) as r:
                status, headers, data = r.status, r.headers, r.read()
        except urllib.error.HTTPError as e:
            status, headers, data = e.code, e.headers, e.read()
        except (OSError, http.client.HTTPException) as e:  # refused, reset or timed out
            log(f"the LLM endpoint is unreachable: {e}")
            status, headers, data = 503, {}, b""
        self.record(label, status, headers.get("x-l3-cache") == "hit", data)
        return status, headers, data

    def record(self, label, status, cached, data):
        usage = {}
        if status == 200:
            try:
                usage = json.loads(data).get("usage") or {}
            except ValueError:
                pass
        with self.lock:
            c = self.usage.setdefault(label, {"requests": 0, "answered": 0, "cached": 0, "prompt_tokens": 0, "completion_tokens": 0, "status": {}})
            c["requests"] += 1
            c["status"][str(status)] = c["status"].get(str(status), 0) + 1
            if status == 200:
                c["answered"] += 1
                c["cached"] += cached
                c["prompt_tokens"] += usage.get("prompt_tokens") or 0
                c["completion_tokens"] += usage.get("completion_tokens") or 0
            writer.write_atomic(self.usage_path, json.dumps(self.usage, indent=1, sort_keys=True) + "\n")

    def totals(self):
        t = {"requests": 0, "answered": 0, "cached": 0, "prompt_tokens": 0, "completion_tokens": 0}
        status = Counter()
        with self.lock:
            for c in self.usage.values():
                for k in t:
                    t[k] += c[k]
                status.update(c["status"])
        return t | {"status": dict(sorted(status.items()))}


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


def render_all(plan, data):
    """L3's renderer, unchanged, over the kept sessions of each repo: transcripts linked under their session ids and a
    selection.json as the renderer reads it. Returns {repo: {"exchanges": path}} for the repos that render to at least one
    exchange."""
    shutil.rmtree(data, ignore_errors=True)
    raw = data / "raw" / "transcripts"
    raw.mkdir(parents=True)
    repos = {}
    for _, key, path in plan:
        m = NAME.match(path.name)
        sid = m[5]
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
        if path.stat().st_size:
            out[key] = {"exchanges": path}
    return out


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


def manifest(repos, found, embedder, gateway, skipped):
    names, vectors, vector_file = found
    inputs = (WORK / "key.txt").read_text().splitlines()
    by_repo, wall = {}, 0.0
    for key in sorted(repos):
        state = WORK / "states" / key
        w = json.loads((state / "manifest.json").read_text())
        rows = schema.load_exchanges(repos[key]["exchanges"])
        created = sorted(writer.split_frontmatter((WORK / "store" / "notes" / n).read_text())[0]["created"] for n in names[key])
        entry = {
            "sessions": sorted({r["session_id"] for r in rows}, key=lambda s: next(r["s"] for r in rows if r["session_id"] == s)),
            "exchanges": len(rows), "exchanges_sha256": sha256(repos[key]["exchanges"]), "notes": names[key],
            "notes_count": len(names[key]), "writer": w["counts"],
            "llm_calls": w["counts"]["decisions"] + w["counts"]["run_llm_calls"], "created": [created[0], created[-1]] if created else None,
        }  # fmt: skip
        if (state / "provenance.json").exists():
            entry["provenance"] = json.loads((state / "provenance.json").read_text())
        by_repo[key] = entry
        wall += w["wall_s"]
    return {
        "arm": ARM, "key": inputs[0], "inputs": inputs[1:], "device": os.environ["BILBO_EVAL_DEVICE"], "versions": versions(embedder),
        "method": {
            "row": ROW, "l3_commit": L3_COMMIT, "l3_commit_note": "local to the L3 eval's branch: the files are identified by l3_files", "l3_files": L3_FILES,
            "skill": {"path": writer.SKILL_PATH, "tag": writer.SKILL_TAG, "sha256": writer.SKILL_SHA256},
            "render_sha": render.render_sha(), "cap": render.CAP, "encoding": render.ENCODING, "decide_window": writer.DECIDE_WINDOW,
            "run_window": writer.RUN_WINDOW, "max_steps": writer.MAX_STEPS, "effort": "default",
            "schedule": "a bare /note after the last exchange of every session" if ROW == "assisted" else None,
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
        "repos": by_repo,
        "notes": sum(r["notes_count"] for r in by_repo.values()),
        "vectors": vectors,
        "llm": {"model": os.environ["BILBO_EVAL_LLM_MODEL"], "calls": sum(r["llm_calls"] for r in by_repo.values()), "gateway": gateway.totals()},
        "ingest_seconds": round(wall),
        "finished": datetime.now(timezone.utc).isoformat(timespec="seconds"),
    }  # fmt: skip


# --- main -------------------------------------------------------------------------------------------------------------


def progress(rows, key, gateway):
    """The writer's step hook: one line at the end of each session, counted from the done journal. It never raises."""
    last = {}
    for r in rows:
        last[r["session_id"]] = r["xid"]
    ends = set(last.values())

    def hook(point, xid):
        try:
            if point != "exchange_done" or xid not in ends:
                return
            finished = {r["xid"] for r in writer.jsonl(WORK / "states" / key / "done.jsonl")}
            log(f"{key} {len(ends & finished)}/{len(ends)} sessions, {len(finished)}/{len(rows)} exchanges, "
                f"{gateway.totals()['answered']} LLM calls")  # fmt: skip
        except Exception:
            pass

    return hook


def guards():
    if ROW not in ("agent", "assisted"):
        sys.exit("bilbo: BILBO_EVAL_ROW must be agent or assisted")
    for name, want in L3_FILES.items():
        if sha256(L3 / name) != want:
            sys.exit(f"{ARM}: {name} is not the file the L3 eval froze at {L3_COMMIT}")
    if sha256(SKILL) != writer.SKILL_SHA256:
        sys.exit(f"{ARM}: {SKILL} is not the note skill of {writer.SKILL_TAG}")


def stop(signum, frame):
    """Leave at once: a pool of decision calls would otherwise drain its whole queue first. The checkpoint tolerates a cut
    (trim_tail, restore_half, the writer's snapshot restore, atomic usage.json)."""
    log("stopped; run again to resume")
    os._exit(130)


def main():
    signal.signal(signal.SIGINT, stop)
    signal.signal(signal.SIGTERM, stop)
    guards()
    llm_url = os.environ["BILBO_EVAL_LLM_URL"].rstrip("/")
    llm_key = os.environ["BILBO_EVAL_LLM_KEY"]
    llm_model = os.environ["BILBO_EVAL_LLM_MODEL"]
    llm_headers = json.loads(os.environ.get("BILBO_EVAL_LLM_HEADERS") or "{}")
    embedder = os.environ["EMBEDDER_URL"].rstrip("/")
    try:
        request = urllib.request.Request(f"{llm_url}/models", headers={"Authorization": f"Bearer {llm_key}"} | llm_headers)
        urllib.request.urlopen(request, timeout=30).close()
    except (OSError, http.client.HTTPException) as e:
        sys.exit(f"{ARM}: the LLM endpoint {llm_url} does not answer: {e}")

    plan, skipped = sessions()
    repos = render_all(plan, WORK / "data")
    gateway = Gateway(llm_url, llm_key, llm_headers, WORK / "usage.json")
    threading.Thread(target=gateway.serve_forever, daemon=True).start()
    chat = writer.Chat(gateway.base_for)
    states, decisions = WORK / "states", WORK / "decisions"
    for key in sorted(repos):
        state = states / key
        rows = schema.load_exchanges(repos[key]["exchanges"])
        p = writer.Params(
            arm=L3_ARM, repo=key, state=state, exchanges=repos[key]["exchanges"], render_sha=render.render_sha(), decisions_dir=decisions,
            bilbo=BILBO, skill=SKILL, embedder=(embedder, MODEL, QUERY_PREFIX), schedule=L3 / "policy" / "schedule.jsonl" if ROW == "assisted" else None,
            rep=1, stage=None, workers=8, model=llm_model, effort=None,
        )  # fmt: skip
        file = writer.decisions_file(decisions, key, 1, "bilbo", llm_model)
        if any(trim_tail(f) for f in [file, *(state / j for j in JOURNALS)]):
            log(f"{key}: dropped the half-written last line of a file an interrupt cut")
        writer.STEP_HOOK = progress(rows, key, gateway)
        try:
            writer.run_decide(p, chat)
            if (session := restore_half(state, BILBO, SKILL, p.embedder)) is not None:
                log(f"{key}: an interrupt stopped session {session} in its first exchange: restored the store it started from")
            writer.run_capture(p, chat)
        except (writer.WriterError, schema.SchemaError) as e:
            sys.exit(f"{ARM}: {key}: {e}; run again to resume")
        log(f"{key}: written")

    found = build_store(repos, states, WORK / "store", BILBO, embedder, Path("/tmp/index-home"))
    (WORK / "store" / "manifest.json").write_text(json.dumps(manifest(repos, found, embedder, gateway, skipped), indent=2) + "\n")
    log(f"done: {WORK / 'store'}")


if __name__ == "__main__":
    main()
