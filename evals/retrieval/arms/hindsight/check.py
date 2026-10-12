"""Check a Hindsight store unpacked in /work/store. As packed: the backup's own manifest lists the tables and row counts the
build's manifest records. Restored into the fresh database the way the eval restores it (the migration with the embedder's
dimension, the rows, the per-bank vector indexes): the schema version and the indexes are right, every row count matches,
and up to 20 memories per bank embedded again point the way their stored vectors do.
Then Hindsight is served the way the eval serves it, with the embedder, no LLM and no worker: it must report the same
counts, documents and pages as the manifest, and three facts per bank searched by their own text must come back.
build-store.sh --check runs it in the build service."""

import asyncio
import json
import math
import os
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import zipfile
from pathlib import Path

STORE = Path("/work/store")
LIVE = Path("/tmp/live")
SERVER = "127.0.0.1:8888"
EMBEDDER_URL = os.environ["EMBEDDER_URL"]
DATABASE_URL = os.environ["DATABASE_URL"]
QUERIES = ("what was decided about the build", "which bug was fixed", "how are tests run", "what did the user prefer")
MIN_COSINE_APPROX = 0.97  # a fact whose text is only approximated: the stored names differ from the extracted ones, which sets the last-token pooling a little off
MIN_COSINE = 0.99  # as for dense, Graphiti and ai-memory; the stored vectors came back at 0.9988 to 1.0
SAMPLE = 10  # observations and facts per bank
manifest = json.loads((STORE / "manifest.json").read_text())


def bank_path(bank):
    return f"/v1/default/banks/{urllib.parse.quote(bank, safe='')}"


def sql(query, *args):
    import asyncpg

    async def run():
        c = await asyncpg.connect(DATABASE_URL)
        try:
            return [dict(r) for r in await c.fetch(query, *args)]
        finally:
            await c.close()

    return asyncio.run(run())


def admin(*args):
    r = subprocess.run(["hindsight-admin", *args], env=os.environ | {"HINDSIGHT_API_DATABASE_URL": DATABASE_URL}, stdin=subprocess.DEVNULL,
                       capture_output=True, text=True, timeout=3600)
    if r.returncode:
        sys.exit(f"check: `hindsight-admin {args[0]}` exited with {r.returncode}: {(r.stdout + r.stderr).strip()[-2000:]}")


def api(method, path, body=None):
    request = urllib.request.Request(f"http://{SERVER}{path}", json.dumps(body).encode() if body is not None else None, {"Content-Type": "application/json"}, method=method)
    try:
        with urllib.request.urlopen(request, timeout=600) as r:
            return json.loads(r.read() or b"null")
    except urllib.error.HTTPError as e:
        sys.exit(f"check: {method} {path}: {e.code} {e.read()[:500]!r}")


def embedded(texts):
    request = urllib.request.Request(EMBEDDER_URL.rstrip("/") + "/embeddings", json.dumps({"input": texts, "model": "embedder"}).encode(), {"Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=600) as r:
        return [d["embedding"] for d in json.load(r)["data"]]


def cosine(a, b):
    norms = math.sqrt(sum(x * x for x in a) * sum(y * y for y in b))
    return sum(x * y for x, y in zip(a, b)) / norms if norms else math.nan


def spread(items, n):
    """Up to n items spread over the list, the first and the last included."""
    if len(items) <= n:
        return items
    return [items[k * (len(items) - 1) // (n - 1)] for k in range(n)]


def dated(row):
    """A fact as Hindsight embeds it: its text, then when it happened (API engine/retain/embedding_processing.py:15-45)."""
    when = row["occurred_start"] or row["mentioned_at"]
    if when is None:
        return row["text"]
    if row["occurred_end"] and row["occurred_end"] != row["occurred_start"]:
        return f"{row['text']} (happened from {when.strftime('%B %Y')} to {row['occurred_end'].strftime('%B %Y')})"
    return f"{row['text']} (happened in {when.strftime('%B %Y')})"


def written(when):
    """A date as Hindsight writes it into text_signals (API engine/memories/pg/writes.py:99-113)."""
    return when.strftime("%B %d %Y").lstrip("0").replace(" 0", " ")


def segment(prefix, pool, norm=lambda x: x):
    """An order of the unit's entity names whose space-joined text is the text_signals prefix, or None. `norm` is applied to
    both sides before matching; the names come back as stored."""
    if not prefix:
        return []
    for name in sorted(pool, key=len, reverse=True):
        n = norm(name)
        if n and (prefix == n or prefix.startswith(n + " ")):
            rest = segment(prefix[len(n):].lstrip(" "), [m for m in pool if m is not name], norm)
            if rest is not None:
                return [name] + rest
    return None


def fold(text):
    """Backticks and case, which entity resolution can change between an extracted name and its stored one."""
    return text.replace("`", "").casefold()


def hybrid(signals, names):
    """The text_signals words, with the words that spell one multi-word stored name (folded) grouped into it and every other word
    on its own, in the spelling of text_signals."""
    words = signals.split()
    folded = [fold(w) for w in words]
    known = sorted({tuple(fold(n).split()) for n in names if " " in n.strip()}, key=len, reverse=True)
    out, i = [], 0
    while i < len(words):
        for group in known:
            if tuple(folded[i:i + len(group)]) == group:
                out.append(" ".join(words[i:i + len(group)]))
                i += len(group)
                break
        else:
            out.append(words[i])
            i += 1
    return out


def signal_names(row):
    """text_signals without its trailing date tokens: the entity names Hindsight extracted, joined by spaces."""
    prefix = row["text_signals"] or ""
    start, end = row["occurred_start"], row["occurred_end"]
    for when in reversed([w for w in (start, end if end and end != start else None) if w]):
        prefix = prefix.removesuffix(" " + written(when)).removesuffix(written(when))
    return prefix.strip()


banks = manifest["banks"]
recall = manifest["recall"]
ok = True


def report(name, passed, detail=""):
    global ok
    ok &= bool(passed)
    print(f"{name}: {'ok' if passed else 'FAILED'} {detail}".rstrip(), flush=True)


# 1. The archive.
with zipfile.ZipFile(STORE / "hindsight.zip") as z:
    packed = {name: t["rows"] for name, t in json.loads(z.read("manifest.json"))["tables"].items()}
report("archive", packed == manifest["store"]["tables"], f"{len(packed)} tables, {sum(packed.values())} rows")

# 2. The restore.
for directory in (LIVE / "home", LIVE / "hooks"):
    directory.mkdir(parents=True, exist_ok=True)
started = time.monotonic()
admin("run-db-migration", "--embedding-dimension", "1024")
admin("restore", str(STORE / "hindsight.zip"), "--yes")
admin("repair-bank", "--all")
rows = {name: sql(f'SELECT count(*) AS n FROM public."{name}"')[0]["n"] for name in packed}
version = sql("SELECT version_num FROM alembic_version")[0]["version_num"]
indexes = sql("SELECT indexname FROM pg_indexes WHERE schemaname = 'public' AND indexname LIKE 'idx_mu_emb_%'")
dims = {r["d"] for r in sql("SELECT DISTINCT vector_dims(embedding) AS d FROM memory_units")}
report("restore", rows == packed and version == manifest["versions"]["alembic"] and len(indexes) == 3 * len(banks) and dims <= {1024},
       f"in {round(time.monotonic() - started)} s: rows match {rows == packed}, alembic {version}, vector indexes {len(indexes)} for {len(banks)} banks, dims {sorted(dims)}")

# 3. The server, with no LLM and no worker.
env = {k: v for k, v in os.environ.items() if not k.startswith("BILBO_EVAL_")} | {
    "HINDSIGHT_API_DATABASE_URL": DATABASE_URL, "HINDSIGHT_API_HOST": "127.0.0.1", "HINDSIGHT_API_PORT": SERVER.split(":")[1],
    "HINDSIGHT_API_LLM_PROVIDER": "none", "HINDSIGHT_API_WORKER_ENABLED": "false", "HINDSIGHT_API_SKIP_LLM_VERIFICATION": "true",
    "HINDSIGHT_API_EMBEDDINGS_PROVIDER": "openai", "HINDSIGHT_API_EMBEDDINGS_OPENAI_BASE_URL": EMBEDDER_URL, "HINDSIGHT_API_EMBEDDINGS_OPENAI_MODEL": "embedder",
    "HINDSIGHT_API_EMBEDDINGS_OPENAI_API_KEY": "none", "HINDSIGHT_API_EMBEDDINGS_QUERY_PREFIX": recall["query_prefix"],
    "HINDSIGHT_API_BANK_STATS_CACHE_TTL_SECONDS": "0", "HINDSIGHT_API_MENTAL_MODEL_REFRESH_TICK_SECONDS": "0",
} | {f"HINDSIGHT_API_{k}": str(v) for k, v in recall["thresholds"].items()}
log = LIVE / "serve.log"
server = subprocess.Popen(["hindsight-api"], env=env, cwd=LIVE, stdin=subprocess.DEVNULL, stdout=log.open("w"), stderr=subprocess.STDOUT)
try:
    started = time.monotonic()
    while True:
        if server.poll() is not None:
            sys.exit(f"check: hindsight-api exited with {server.returncode}:\n{log.read_text()[-3000:]}")
        try:
            urllib.request.urlopen(f"http://{SERVER}/health", timeout=5).close()
            break
        except OSError:
            if time.monotonic() - started > 600:
                sys.exit("check: hindsight-api did not answer /health in 10 minutes")
            time.sleep(1)
    served = api("GET", "/version")["api_version"]
    report("serve", served == manifest["versions"]["hindsight"], f"hindsight {served}, manifest {manifest['versions']['hindsight']}, up in {round(time.monotonic() - started)} s")

    # 4. The counts of each bank.
    for bank, b in banks.items():
        s = api("GET", f"{bank_path(bank)}/stats?refresh=true")
        want = {}
        for statuses in b["operations"].values():
            for status, n in statuses.items():
                want[status] = want.get(status, 0) + n
        documents = sorted(d["id"] for d in api("GET", f"{bank_path(bank)}/documents?limit=1000")["items"])
        tree = {n["name"]: n["trigger"]["refresh_cron"] for n in api("GET", f"{bank_path(bank)}/knowledge-base/tree")["roots"] if n["kind"] == "page"}
        counts = (s["nodes_by_fact_type"], s["total_observations"], s["total_links"], s["total_documents"])
        report(f"counts {bank}",
               counts == (b["nodes_by_fact_type"], b["observations"], b["links"], len(b["documents"])) and documents == b["documents"]
               and s["operations_by_status"] == want and set(want) <= {"completed"} and not s["failed_operations"] and not s["failed_consolidation"] and not s["pending_consolidation"]
               and tree == {name: p["cron"] for name, p in manifest["pages"]["banks"][bank].items()},
               f"facts {s['nodes_by_fact_type']}, observations {s['total_observations']}, links {s['total_links']}, documents {len(documents)}, operations {s['operations_by_status']}, pages {len(tree)}")
    filled = sql("SELECT count(*) AS n FROM mental_models WHERE content IS NOT NULL AND content <> ''")[0]["n"]
    report("pages with content", filled == manifest["pages"]["pages_with_content"], f"{filled}, manifest {manifest['pages']['pages_with_content']}")

    # 5. Vectors: memories spread over each bank, embedded again as Hindsight embedded them. A fact is its text, when it happened
    # and its entities in extraction order, which text_signals keeps (API engine/memories/pg/writes.py:99-113).
    exact, approx, misplaced = [], [], 0
    for bank in banks:
        units = sql("SELECT id, fact_type, text, text_signals, occurred_start, occurred_end, mentioned_at, embedding::text AS embedding FROM memory_units WHERE bank_id = $1 ORDER BY fact_type, mentioned_at, id", bank)
        sample = spread([u for u in units if u["fact_type"] == "observation"], SAMPLE) + spread([u for u in units if u["fact_type"] != "observation"], SAMPLE)
        for u in sample:
            stored = json.loads(u["embedding"])
            rebuilt = True
            if u["fact_type"] == "observation":
                texts = [u["text"]]
            else:
                names = [r["canonical_name"] for r in sql("SELECT e.canonical_name FROM unit_entities ue JOIN entities e ON e.id = ue.entity_id WHERE ue.unit_id = $1", u["id"])]
                signals = signal_names(u)
                order = segment(signals, names)
                folded = None if order is not None else segment(fold(signals), names, fold)
                if order is not None:
                    texts = [f"{dated(u)} [{', '.join(order)}]" if order else dated(u)]
                else:
                    # Not rebuilt exactly: an extracted name that entity resolution stored under another canonical name, or in another
                    # case or with backticks. The best of the grouped text_signals words, the folded split, the plain words, the
                    # canonical names sorted, and none.
                    rebuilt = False
                    hy = hybrid(signals, names)
                    texts = [f"{dated(u)} [{', '.join(hy)}]" if hy else dated(u)]
                    if folded:
                        texts.append(f"{dated(u)} [{', '.join(folded)}]")
                    texts += [dated(u), f"{dated(u)} [{', '.join(signals.split())}]"] + ([f"{dated(u)} [{', '.join(sorted(names))}]"] if names else [])
            new = max(embedded(texts), key=lambda v: cosine(v, stored))
            (exact if rebuilt else approx).append(cosine(new, stored))
            nearest = max(sample, key=lambda o: cosine(new, json.loads(o["embedding"])))
            misplaced += nearest["id"] != u["id"]
    span = lambda c: f"{min(c):.5f} to {max(c):.5f}" if c else "none"
    report("vectors", bool(exact + approx) and all(c >= MIN_COSINE for c in exact) and all(c >= MIN_COSINE_APPROX for c in approx) and not misplaced,
           f"{len(exact)} exactly rebuilt (observations and facts) cosine {span(exact)} (at least {MIN_COSINE}); {len(approx)} facts rebuilt approximately "
           f"cosine {span(approx)} (at least {MIN_COSINE_APPROX}); nearest is not itself for {misplaced}")

    # 6. Recall: three facts per bank, each searched by its own text, must come back in the first five.
    stamp = recall["last_session_time"]
    for bank in banks:
        units = sql("SELECT id, fact_type, text FROM memory_units WHERE bank_id = $1 ORDER BY fact_type, mentioned_at, id", bank)
        facts = [u for u in units if u["fact_type"] != "observation"] or units
        found = []
        for u in spread(facts, 3):
            answer = api("POST", f"{bank_path(bank)}/memories/recall", {"query": u["text"], "budget": "mid", "query_timestamp": stamp})
            found.append(str(u["id"]) in [r["id"] for r in answer["results"][:5]])
        hits = [len(api("POST", f"{bank_path(bank)}/memories/recall", {"query": q, "budget": "mid", "query_timestamp": stamp})["results"]) for q in QUERIES]
        report(f"recall {bank}", bool(found) and all(found), f"facts found by their own text {found}; results for the four questions {hits}")
finally:
    server.terminate()
    try:
        server.wait(60)
    except subprocess.TimeoutExpired:
        server.kill()

print("OK" if ok else "FAILED")
sys.exit(0 if ok else 1)
