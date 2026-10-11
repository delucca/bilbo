"""Check an ai-memory store unpacked in /work/store. As packed: the database is intact and its counts match the
manifest, every latest page's file in the wiki ends with its body, config.toml holds the settings the manifest lists,
every latest page with a body has a unit vector from the embedder, and up to five pages embedded again point the way
their stored vectors do.
Then a copy is served the way the eval serves it, with the embedder, no LLM and maintenance off: it must report the
same counts and add no vector, and four searches in each project must find pages through the vector stream.
build-store.sh --check runs it in the build service."""

import array
import collections
import hashlib
import http.client
import json
import math
import os
import shutil
import sqlite3
import subprocess
import sys
import time
import tomllib
import urllib.request
import uuid
from pathlib import Path
from urllib.parse import urlparse

STORE = Path("/work/store")
CHECK = Path("/tmp/check")  # the served copy: serving writes to its data directory
BIN = "/usr/local/bin/ai-memory"
SERVER = "127.0.0.1:7801"
EMBEDDER = urlparse(os.environ["EMBEDDER_URL"])
QUERIES = ("what was decided about the build", "which bug was fixed", "how are tests run", "what did the user prefer")
SYNTH_FOOTER = "_Synthesised by ai-memory (M3, no-LLM heuristic)._"
MAX_INPUT = 8000  # the bytes ai-memory sends the embedder (crates/ai-memory-llm/src/text.rs, truncate_for_embedding)
MIN_COSINE = 0.99  # as for dense and Graphiti; dense measured 0.9976 between a CPU and a GPU embedding
env = {"HOME": str(CHECK / "home"), "PATH": "/usr/local/bin:/usr/bin:/bin", "NO_COLOR": "1",
       "AI_MEMORY_SERVER_URL": f"http://{SERVER}", "AI_MEMORY_EMBEDDING_BASE_URL": os.environ["EMBEDDER_URL"],
       "AI_MEMORY_MAINTENANCE__ENABLED": "false"}
manifest = json.loads((STORE / "manifest.json").read_text())
db = sqlite3.connect(f"file:{STORE / 'db' / 'memory.sqlite'}?mode=ro&immutable=1", uri=True)
db.row_factory = sqlite3.Row


def rows(sql, *args):
    return db.execute(sql, args).fetchall()


def flat(table, prefix=""):
    out = {}
    for k, v in table.items():
        out |= flat(v, f"{prefix}{k}.") if isinstance(v, dict) else {prefix + k: v}
    return out


def vector(blob):
    v = array.array("f", blob)  # little-endian float32 (crates/ai-memory-store/src/reader.rs, f32_vec_to_bytes)
    if sys.byteorder == "big":
        v.byteswap()
    return v


def cosine(a, b):
    norms = math.sqrt(sum(x * x for x in a) * sum(y * y for y in b))
    return sum(x * y for x, y in zip(a, b)) / norms if norms else math.nan


def in_wiki(p):
    """Whether the page's markdown, the store's source of truth, ends with the body the database holds."""
    f = STORE / "wiki" / str(uuid.UUID(bytes=p["workspace_id"])) / str(uuid.UUID(bytes=p["project_id"])) / p["path"]
    return f.is_file() and f.read_bytes().endswith(p["body"].encode())


def embedded(text, model):
    """The embedder's vector for a page body, cut as ai-memory cuts it: at a character, ended with an ellipsis."""
    raw = text.encode()
    if len(raw) > MAX_INPUT:
        text = raw[: MAX_INPUT - len("…".encode())].decode(errors="ignore") + "…"
    conn = http.client.HTTPConnection(EMBEDDER.netloc, timeout=600)
    conn.request("POST", EMBEDDER.path.rstrip("/") + "/embeddings",
                 json.dumps({"input": text, "model": model}), {"Content-Type": "application/json"})
    r = conn.getresponse()
    data = r.read()
    if r.status != 200:
        sys.exit(f"check: POST /embeddings: {r.status} {data[:300]!r}")
    return json.loads(data)["data"][0]["embedding"]


def status():
    r = subprocess.run([BIN, "--data-dir", str(CHECK / "data"), "status", "--json"], env=env, cwd=CHECK / "home",
                       stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=600)
    if r.returncode:
        sys.exit(f"check: `ai-memory status` exited with {r.returncode}: {r.stderr.strip()[-2000:]}")
    return json.loads(r.stdout)


def search(workspace, project, query):
    """memory_query over MCP, as an agent calls it: Streamable HTTP, stateless, explained."""
    args = {"workspace": workspace, "project": project, "query": query, "limit": 10, "explain": True}
    body = {"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "memory_query", "arguments": args}}
    conn = http.client.HTTPConnection(SERVER, timeout=600)
    conn.request("POST", "/mcp", json.dumps(body),
                 {"Content-Type": "application/json", "Accept": "application/json, text/event-stream"})
    r = conn.getresponse()
    data = r.read().decode()
    answer = json.loads(data) if r.status == 200 else {}
    if not answer.get("result") or answer["result"].get("isError"):
        sys.exit(f"check: memory_query in {workspace}/{project}: {r.status} {data[:300]}")
    return json.loads(answer["result"]["content"][0]["text"])


config = flat(tomllib.loads((STORE / "config.toml").read_text()))
listed = {e["key"]: e["value"] for e in manifest["config"] if not e["key"].startswith(("AI_MEMORY_", "hook "))}
embedder = manifest["embedder"]
identity = (embedder["provider"], config["embedding_model"], embedder["dim"])
config_ok = (config == listed and config["embedding_provider"] == embedder["provider"]
             and config["embedding_dim"] == embedder["dim"] and config["embedding_query_prefix"] == embedder["query_prefix"])
integrity = [r[0] for r in rows("PRAGMA integrity_check")]
print("integrity", integrity, "| config.toml matches the manifest", config_ok)

sessions = rows("SELECT s.id, p.name AS project, s.ended_at FROM sessions s JOIN projects p ON p.id = s.project_id")
by_project = collections.Counter(s["project"] for s in sessions)
listed_projects = sum((collections.Counter(found) for found in manifest["projects"].values()), collections.Counter())
pages = rows("SELECT p.id, p.workspace_id, p.project_id, w.name AS workspace, pr.name AS project, p.path, p.body,"
             " p.body_sha256, p.frontmatter_json, p.is_latest FROM pages p"
             " JOIN projects pr ON pr.id = p.project_id JOIN workspaces w ON w.id = p.workspace_id")
latest = sorted((p for p in pages if p["is_latest"]), key=lambda p: (p["workspace"], p["project"], p["path"]))
if not latest:
    sys.exit("check: the store has no pages")
session_pages = {p["path"]: p for p in latest if p["path"].startswith("sessions/")}
sids = [(str(uuid.UUID(bytes=s["id"])), s["ended_at"]) for s in sessions]
with_page = sum(f"sessions/{sid}.md" in session_pages for sid, _ in sids)
still_open = sum(ended is None for _, ended in sids)
ephemeral = sum(ended is not None and f"sessions/{sid}.md" not in session_pages for sid, ended in sids)
m = manifest["sessions"]
sessions_ok = (len(sessions) == m["replayed"] and by_project == listed_projects and with_page == m["with_page"]
               and still_open == len(m["ignored_end"]) and ephemeral == len(m["ephemeral"]))
print("sessions", len(sessions), "manifest", m["replayed"], "| by project matches", by_project == listed_projects,
      "| with a page", with_page, "manifest", m["with_page"], "| SessionEnd ignored", still_open, "manifest",
      len(m["ignored_end"]), "| ephemeral", ephemeral, "manifest", len(m["ephemeral"]))

observations = {r[0]: r[1] for r in rows("SELECT kind, COUNT(*) FROM observations GROUP BY kind")}
by_folder = collections.Counter(p["path"].split("/")[0] for p in latest)
bodies_ok = all(hashlib.sha256(p["body"].encode()).digest() == p["body_sha256"] for p in pages)
unwritten = [p["path"] for p in latest if not in_wiki(p)]
m = manifest["pages"]
pages_ok = (observations == manifest["observations"]["by_kind"] and sum(observations.values()) == manifest["observations"]["total"]
            and len(latest) == m["latest"] and len(pages) == m["all"] and by_folder == m["by_folder"] and bodies_ok
            and not unwritten)
print("observations", sum(observations.values()), "manifest", manifest["observations"]["total"], "| by kind matches",
      observations == manifest["observations"]["by_kind"], "| pages", len(latest), "latest of", len(pages), "manifest",
      m["latest"], "of", m["all"], "| by folder matches", by_folder == m["by_folder"], "| bodies match their sha256",
      bodies_ok, "| latest pages not in the wiki", unwritten)

jobs = {r[0]: r[1] for r in rows("SELECT state, COUNT(*) FROM session_consolidation_jobs GROUP BY state")}
rule_based = sum(p["body"].rstrip().endswith(SYNTH_FOOTER) for p in session_pages.values())
consolidated = sum(json.loads(p["frontmatter_json"] or "{}").get("consolidated") is True for p in session_pages.values())
proposals = {r[0]: r[1] for r in rows("SELECT status, COUNT(*) FROM auto_improve_proposals GROUP BY status")}
m = manifest["consolidation"]
work_ok = (jobs == m["jobs"] and rule_based == m["rule_based_latest_session_pages"]
           and consolidated == m["consolidated_latest_session_pages"] and proposals == manifest["auto_improve"]["proposals"])
print("consolidation jobs", jobs, "| session pages rule-based", rule_based, "consolidated", consolidated,
      "| auto-improve proposals", proposals, "| all match", work_ok)

vectors = {r["page_id"]: r for r in rows("SELECT page_id, vector, provider, model, dim FROM page_embeddings")}
# Follows ai-memory's status rule: a latest page with no vector and a body of only spaces (SQL's TRIM) is unembeddable.
missing = [p["path"] for p in latest if p["id"] not in vectors and p["body"].strip(" ")]
unembeddable = sum(p["id"] not in vectors and not p["body"].strip(" ") for p in latest)
norms = [math.sqrt(sum(x * x for x in vector(r["vector"]))) for r in vectors.values()]
same = all((r["provider"], r["model"], r["dim"]) == identity and len(r["vector"]) == 4 * r["dim"] for r in vectors.values())
vectors_ok = (not missing and unembeddable == manifest["pages"]["unembeddable"] and same
              and all(abs(n - 1) <= 1e-3 for n in norms))
print("vectors", len(vectors), "| latest pages without one", missing, "| unembeddable", unembeddable, "manifest",
      manifest["pages"]["unembeddable"], f"| norms {min(norms, default=0):.5f} to {max(norms, default=0):.5f}",
      "| all", identity, same)

# Five latest pages spread over the store, embedded again as ai-memory embedded them: the body, with no prefix.
sample = [p for p in latest if p["id"] in vectors]
picks = sorted({len(sample) * k // 4 for k in range(4)} | {len(sample) - 1}) if sample else []
cosines = []
for p in (sample[i] for i in picks):
    cosines.append(cosine(embedded(p["body"], config["embedding_model"]), vector(vectors[p["id"]]["vector"])))
    print(f"page {p['project']}/{p['path']} ({len(p['body'].encode())} bytes): cosine {cosines[-1]:.6f}")
embedded_ok = bool(cosines) and all(c >= MIN_COSINE for c in cosines)

shutil.rmtree(CHECK, ignore_errors=True)
shutil.copytree(STORE, CHECK / "data", ignore=shutil.ignore_patterns("manifest.json"))
(CHECK / "home").mkdir()
log = CHECK / "serve.log"
started = time.monotonic()
server = subprocess.Popen([BIN, "--data-dir", str(CHECK / "data"), "serve", "--transport", "http", "--bind", SERVER],
                          env=env, cwd=CHECK / "home", stdin=subprocess.DEVNULL, stdout=log.open("w"),
                          stderr=subprocess.STDOUT)
try:
    for _ in range(600):
        if server.poll() is not None:
            sys.exit(f"check: ai-memory serve exited with {server.returncode}:\n{log.read_text()[-3000:]}")
        try:
            urllib.request.urlopen(f"http://{SERVER}/healthz", timeout=2).close()
            break
        except OSError:
            time.sleep(0.5)
    else:
        sys.exit("check: ai-memory serve did not answer /healthz in 5 minutes")
    before = status()
    found = {}
    for scope in sorted({(p["workspace"], p["project"]) for p in latest}):
        for q in QUERIES:
            hits = search(*scope, q)
            found[f"{scope[1]}: {q}"] = (len(hits["hits"]),
                                         sum("vector_rank" in (h.get("score_details") or {}) for h in hits["hits"]),
                                         "vector" in (hits.get("streams_active") or []))
    after = status()
finally:
    server.terminate()
    try:
        server.wait(60)
    except subprocess.TimeoutExpired:
        server.kill()
seconds = round(time.monotonic() - started)
served = log.read_text()

counts = {"pages_latest": len(latest), "pages_all": len(pages), "sessions": len(sessions),
          "observations": sum(observations.values())}
d, llm, emb = after["derived"], after["providers"]["llm"], after["providers"]["embedding"]
maintenance_off = "maintenance disabled" in served
backfilled = "startup embedding backfill completed" in served
serve_ok = (after["version"] == manifest["versions"]["ai-memory"]
            and all(s["counts"][k] == v for s in (before, after) for k, v in counts.items())
            and d["latest_pages_missing_embeddings"] == 0 and d["embedding_rows"] == len(vectors)
            and d["pages_fts_rows"] == d["pages_rows"] and d["observations_fts_rows"] == d["observations_rows"]
            and llm["status"] == "disabled" and (emb["provider"], emb["model"], emb["dim"]) == identity
            and maintenance_off and not backfilled)
print("served: ai-memory", after["version"], "manifest", manifest["versions"]["ai-memory"], "| counts",
      {k: after["counts"][k] for k in counts}, "| missing embeddings", d["latest_pages_missing_embeddings"],
      "| embedding rows", d["embedding_rows"], "| fts rows", d["pages_fts_rows"], "of", d["pages_rows"], "pages,",
      d["observations_fts_rows"], "of", d["observations_rows"], "observations | llm", llm["status"], "| embedder",
      (emb["provider"], emb["model"], emb["dim"]), "| maintenance off", maintenance_off, "| backfilled", backfilled,
      "| served in", seconds, "s")
print("searches (hits, hits from the vector stream, vector stream on):", found)
searches_ok = all(hits and from_vector and on for hits, from_vector, on in found.values())

ok = (integrity == ["ok"] and config_ok and sessions_ok and pages_ok and work_ok and vectors_ok and embedded_ok
      and serve_ok and searches_ok)
print("OK" if ok else "FAILED")
sys.exit(0 if ok else 1)
