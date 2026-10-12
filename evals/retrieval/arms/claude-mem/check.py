"""Check a claude-mem store unpacked in /work/store. As packed: SQLite is intact and its full-text tables verify, its
row counts and dates match the manifest, settings.json holds the settings the manifest lists, Chroma holds exactly the
documents claude-mem writes for the rows (but the ones the embedder rejected for length), and five documents embedded
again point the way their stored vectors do.
Then a copy is served the way the eval serves it, with the embedder and no LLM: it must report the same counts and add
no document, and four searches in each project must find observations through Chroma.
build-store.sh --check runs it in the build service. Its Chroma part (--chroma <dir>) runs in chroma-mcp's
environment."""

import datetime
import http.client
import json
import math
import os
import re
import shutil
import sqlite3
import subprocess
import sys
import time
import urllib.parse
import urllib.request
from pathlib import Path

STORE = Path("/work/store")
CHECK = Path("/tmp/check")  # the copies: opening SQLite, Chroma and the worker writes to them
PORT = 37777
EMBEDDER = os.environ.get("EMBEDDER_URL", "")
QUERIES = ("what was decided about the build", "which bug was fixed", "how are tests run", "what did the user prefer")
MIN_COSINE = 0.99  # as for dense and Graphiti; dense measured 0.9976 between a CPU and a GPU embedding
LAUNCHER = ["--python", "3.13", "--with", "onnxruntime>=1.20", "--with", "protobuf<7", "--with", "chromadb==1.5.9",
            "--with", "pydantic<2.14", "--from", "chroma-mcp==0.2.6"]
FALLBACKS = ("falling back to FTS5", "falling back to SQLite", "supplementing from SQLite", "semantic search failed")
LLM_ATTEMPTS = ("multi-turn (", "API usage", "network error")  # what the worker logs when it tries the observer


def chroma_part(path):
    """In the chroma-mcp environment: the collection's function, its ids with their dates, and five stored vectors."""
    import chromadb

    collection = chromadb.PersistentClient(path=path).get_collection("cm__claude-mem")
    configuration = collection.configuration
    function = configuration.get("embedding_function") if hasattr(configuration, "get") else None
    got = collection.get(include=["metadatas"])
    ids = sorted(got["ids"])
    picks = sorted({len(ids) * k // 4 for k in range(4)} | {len(ids) - 1}) if ids else []
    samples = {"ids": [], "documents": [], "embeddings": []}
    if picks:
        samples = collection.get(ids=[ids[i] for i in picks], include=["documents", "embeddings"])
    print(json.dumps({
        "function": getattr(function, "name", lambda: str(function))() if function is not None else None,
        "space": (configuration.get("hnsw") or {}).get("space") if hasattr(configuration, "get") else None,
        "epochs": dict(zip(got["ids"], (m.get("created_at_epoch") for m in got["metadatas"]))),
        "samples": [{"id": i, "document": d, "embedding": [float(x) for x in e]}
                    for i, d, e in zip(samples["ids"], samples["documents"], samples["embeddings"])],
    }))


if sys.argv[1:2] == ["--chroma"]:
    chroma_part(sys.argv[2])
    sys.exit(0)

manifest = json.loads((STORE / "manifest.json").read_text())


def docs_of(kind, row):
    """The Chroma documents claude-mem writes for a row, as (id, text) (ChromaSync.ts formatObservationDocs,
    formatSummaryDocs and the prompt document)."""
    if kind == "prompts":
        return [(f"prompt_{row['id']}", row["prompt_text"])]
    if kind == "summaries":
        fields = ("request", "investigated", "learned", "completed", "next_steps", "notes")
        return [(f"summary_{row['id']}_{f}", row[f]) for f in fields if row[f]]
    try:
        facts = json.loads(row["facts"]) if row["facts"] else []
        if not isinstance(facts, list):
            facts = [facts]
        facts = [f for f in facts if isinstance(f, str) and f.strip()]
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


def open_db(path):
    c = sqlite3.connect(path)
    c.row_factory = sqlite3.Row
    return c


def counts(db):
    one = lambda sql: db.execute(sql).fetchone()[0]
    return {
        "sdk_sessions": one("SELECT COUNT(*) FROM sdk_sessions"),
        "user_prompts": one("SELECT COUNT(*) FROM user_prompts"),
        "observations": {"total": one("SELECT COUNT(*) FROM observations"),
                         "by_type": {r[0]: r[1] for r in db.execute(
                             "SELECT type, COUNT(*) FROM observations GROUP BY type ORDER BY type")}},
        "session_summaries": one("SELECT COUNT(*) FROM session_summaries"),
        "tool_uses": one("SELECT COUNT(*) FROM tool_uses"),
    }


def embedded(text):
    conn = http.client.HTTPConnection(urllib.parse.urlparse(EMBEDDER).netloc, timeout=600)
    conn.request("POST", urllib.parse.urlparse(EMBEDDER).path.rstrip("/") + "/embeddings",
                 json.dumps({"input": text, "model": "text-embedding-ada-002"}), {"Content-Type": "application/json"})
    r = conn.getresponse()
    data = r.read()
    if r.status != 200:
        sys.exit(f"check: POST /embeddings: {r.status} {data[:300]!r}")
    return json.loads(data)["data"][0]["embedding"]


def cosine(a, b):
    norms = math.sqrt(sum(x * x for x in a) * sum(y * y for y in b))
    return sum(x * y for x, y in zip(a, b)) / norms if norms else math.nan


def api(path, method="GET", timeout=600):
    request = urllib.request.Request(f"http://127.0.0.1:{PORT}{path}", b"{}" if method == "POST" else None,
                                     {"Content-Type": "application/json"}, method=method)
    with urllib.request.urlopen(request, timeout=timeout) as r:
        return json.loads(r.read() or b"{}")


def logged(data):
    return "".join(p.read_text(errors="replace") for p in sorted((data / "logs").glob("*.log")))


shutil.rmtree(CHECK, ignore_errors=True)
(CHECK / "home").mkdir(parents=True)
shutil.copy2(STORE / "claude-mem.db", CHECK / "packed.db")
db = open_db(CHECK / "packed.db")
integrity = [r[0] for r in db.execute("PRAGMA integrity_check")]
fts = {}
for table in ("observations_fts", "session_summaries_fts"):
    try:
        db.execute(f"INSERT INTO {table}({table}) VALUES('integrity-check')")
        fts[table] = "ok"
    except sqlite3.Error as e:
        fts[table] = str(e)
print("integrity", integrity, "| full-text integrity-check", fts)

found, listed = counts(db), manifest["rows"]
epochs = {t: db.execute(f"SELECT MIN(created_at_epoch), MAX(created_at_epoch) FROM {t}").fetchone()
          for t in ("user_prompts", "observations", "session_summaries")}
ms = lambda iso: int(datetime.datetime.fromisoformat(iso.replace("Z", "+00:00")).timestamp() * 1000)
lo, hi = ms(manifest["time"]["first_event"]), ms(manifest["time"]["last_event"])
dated = all(a is None or (lo <= a and b <= hi + 180_000) for a, b in epochs.values())
rows_ok = all(found[k] == listed[k] for k in found)
print("rows", found, "manifest", {k: listed[k] for k in found}, "| match", rows_ok, "| dated within",
      manifest["time"]["first_event"], "to", manifest["time"]["last_event"],
      "+ 180 s", dated, {t: tuple(v) for t, v in epochs.items()})

settings = json.loads((STORE / "settings.json").read_text())
wanted = {e["key"]: e["value"] for e in manifest["config"] if e["key"].startswith("CLAUDE_MEM_")}
settings_ok = all(settings.get(k) == v for k, v in wanted.items())
print("settings.json holds the manifest's", len(wanted), "CLAUDE_MEM_ settings:", settings_ok)

# Chroma, in chroma-mcp's own environment: ids, metadata and five stored vectors.
shutil.copytree(STORE / "chroma", CHECK / "packed-chroma")
tool_env = os.environ | {"OPENAI_BASE_URL": EMBEDDER, "OPENAI_API_KEY": "local"}
r = subprocess.run(["uvx", *LAUNCHER, "python", __file__, "--chroma", str(CHECK / "packed-chroma")],
                   capture_output=True, text=True, env=tool_env)
if r.returncode:
    sys.exit(f"check: reading Chroma failed: {r.stderr.strip()[-2000:]}")
chroma = json.loads(r.stdout.strip().splitlines()[-1])

expected, expected_epoch = set(), {}
for kind, table in (("observations", "observations"), ("summaries", "session_summaries"), ("prompts", "user_prompts")):
    for row in db.execute(f"SELECT * FROM {table}"):
        for doc_id, _ in docs_of(kind, row):
            expected.add(doc_id)
            expected_epoch[doc_id] = row["created_at_epoch"]
rejected = manifest["chroma"]["rejected_by_embedder"]
for item in rejected:
    table = {"observations": "observations", "summaries": "session_summaries", "prompts": "user_prompts"}[item["kind"]]
    for doc_id, _ in docs_of(item["kind"], db.execute(f"SELECT * FROM {table} WHERE id = ?", (item["id"],)).fetchone()):
        expected.discard(doc_id)
have = set(chroma["epochs"])
# A batch claude-mem re-delivers with the same title and narrative reuses its row and writes the new batch's documents
# and date to Chroma (SessionStore.ts:3868-3873, ResponseProcessor.ts:1007-1015). Such a row is exempt from the id and
# date comparison only when its newer dates lie inside the build's time range and Chroma holds every id SQLite expects
# for it, and only while few rows are exempt. Every other id or date mismatch fails.
row_of = lambda doc_id: re.match(r"(?:obs|summary|prompt)_\d+", doc_id).group()
newer = {row_of(i) for i in have & expected if chroma["epochs"][i] > expected_epoch[i]}
redelivered = {r for r in newer
               if all(lo <= chroma["epochs"][i] <= hi + 180_000 for i in have if row_of(i) == r)
               and {i for i in expected if row_of(i) == r} <= have}
skipped = {i for i in have | expected if row_of(i) in redelivered}
exempt_ok = len(redelivered) <= 0.01 * len({row_of(i) for i in expected})
dates_ok = all(chroma["epochs"][i] == expected_epoch[i] for i in (have & expected) - skipped)
ids_ok = (have - skipped == expected - skipped and len(have) == manifest["chroma"]["documents"] and exempt_ok)
print("chroma", len(have), "documents, expected", len(expected), "manifest", manifest["chroma"]["documents"],
      "| ids match", ids_ok,
      "| missing", sorted(expected - have - skipped)[:5], "extra", sorted(have - expected - skipped)[:5],
      "| created_at_epoch matches", dates_ok, "| rows re-delivered with a newer date", len(redelivered),
      sorted(redelivered)[:10], "within 1%", exempt_ok,
      "| rejected by the embedder", rejected, "| function", chroma["function"], "space", chroma["space"])
cosines = []
for s in chroma["samples"]:
    cosines.append(cosine(embedded(s["document"]), s["embedding"]))
    print(f"document {s['id']} ({len(s['document'])} characters, {len(s['embedding'])} dimensions): "
          f"cosine {cosines[-1]:.6f}")
dims = [len(s["embedding"]) for s in chroma["samples"]]
embedded_ok = (bool(cosines) and all(math.isfinite(c) and c >= MIN_COSINE for c in cosines)
               and dims == [manifest["embedder"]["dim"]] * len(dims))
chroma_ok = ids_ok and dates_ok and embedded_ok and chroma["function"] == "openai"

# Served, as the run step serves it: a copy, real time, the embedder, and an observer URL nothing listens on.
data = CHECK / "data"
shutil.copytree(STORE, data, ignore=shutil.ignore_patterns("manifest.json"))
dead = {"CLAUDE_MEM_OPENAI_COMPAT_BASE_URL": "http://127.0.0.1:9/v1", "CLAUDE_MEM_LOG_LEVEL": "DEBUG"}
served = json.loads((data / "settings.json").read_text()) | dead
(data / "settings.json").write_text(json.dumps(served, indent=2) + "\n")
env = {"HOME": str(CHECK / "home"), "PATH": "/usr/local/bin:/usr/bin:/bin", "CLAUDE_MEM_DATA_DIR": str(data),
       "CLAUDE_MEM_WORKER_PORT": str(PORT), "CLAUDE_MEM_WORKER_AUTOSTART": "false", "DO_NOT_TRACK": "1",
       "OPENAI_BASE_URL": EMBEDDER, "OPENAI_API_KEY": "local"} | dead
env |= {k: v for k, v in os.environ.items() if k.startswith("UV_")}
started = time.monotonic()
worker = subprocess.Popen(["bun", "/opt/claude-mem/plugin/scripts/worker-service.cjs", "--daemon"], env=env,
                          cwd=CHECK / "home", stdin=subprocess.DEVNULL, stdout=(CHECK / "worker.out").open("w"),
                          stderr=subprocess.STDOUT, start_new_session=True)
searches, chroma_status, since, undated, first, asked_to_stop = {}, {}, "", [], None, False
BACKFILLED = ("Backfill check complete for all projects", "Backfill check ended before every project finished")
try:
    until = time.monotonic() + 240
    while True:
        if worker.poll() is not None:
            sys.exit(f"check: the worker exited with {worker.returncode}:\n"
                     f"{(CHECK / 'worker.out').read_text()[-3000:]}")
        try:
            api("/api/readiness", timeout=2)
            break
        except OSError:
            if time.monotonic() > until:
                sys.exit("check: the worker did not answer /api/readiness in 4 minutes")
            time.sleep(0.5)
    until = time.monotonic() + 600
    # The second line ends the start of a store with a row the embedder rejected.
    while not any(m in logged(data) for m in BACKFILLED):
        if time.monotonic() > until:
            sys.exit("check: the worker did not end its backfill check in 10 minutes")
        time.sleep(0.5)
    chroma_status = api("/api/chroma/status?deep=1")
    begun = len(logged(data))
    projects = [r[0] for r in db.execute("SELECT DISTINCT project FROM observations ORDER BY project")]
    for project in projects:
        for q in QUERIES:
            params = urllib.parse.urlencode({"query": q, "format": "json", "type": "observations", "limit": 10,
                                             "dateStart": "2000-01-01", "project": project})
            hits = api(f"/api/search?{params}")["observations"]
            searches[f"{project}: {q}"] = (len(hits), all(h.get("project") == project for h in hits))
    since = logged(data)[begun:]
    first = projects[0] if projects else None
    begun = len(logged(data))
    if first:
        params = urllib.parse.urlencode({"query": QUERIES[0], "format": "json", "type": "observations", "limit": 10,
                                         "project": first})
        undated = api(f"/api/search?{params}")["observations"]
    undated_fell_back = [m for m in FALLBACKS if m in logged(data)[begun:]]
    asked_to_stop = True
    try:
        api("/api/admin/shutdown", "POST", timeout=10)
    except OSError:
        pass  # the shutdown ends the connection
finally:
    if not asked_to_stop and worker.poll() is None:
        worker.terminate()
    try:
        worker.wait(120 if asked_to_stop else 30)
    except subprocess.TimeoutExpired:
        worker.kill()
    subprocess.run(["pkill", "-9", "-f", "chroma-mcp"])
seconds = round(time.monotonic() - started)
log = logged(data)
fallbacks = [m for m in FALLBACKS if m in since]
llm_tries = [m for m in LLM_ATTEMPTS if m in log]
db_after = open_db(data / "claude-mem.db")
after = counts(db_after)
r = subprocess.run(["uvx", *LAUNCHER, "python", __file__, "--chroma", str(data / "chroma")],
                   capture_output=True, text=True, env=tool_env)
after_ids = set(json.loads(r.stdout.strip().splitlines()[-1])["epochs"]) if r.returncode == 0 else None
serve_ok = (chroma_status.get("status") == "healthy" and after == found and after_ids == have and not llm_tries)
print("served in", seconds, "s | chroma status", chroma_status.get("status"), "| counts after", after == found,
      "| ids after", after_ids == have,
      "| LLM attempts in the log", llm_tries)
print("searches (hits, all in the project):", searches)
print("informational: the first query without dateStart found", len(undated), "observations in", first,
      "| fell back:", undated_fell_back)
print("fallbacks while searching:", fallbacks)
searches_ok = bool(searches) and all(n and same for n, same in searches.values()) and not fallbacks

ok = (integrity == ["ok"] and all(v == "ok" for v in fts.values()) and rows_ok and dated and settings_ok and chroma_ok
      and serve_ok and searches_ok)
print("OK" if ok else "FAILED")
sys.exit(0 if ok else 1)
