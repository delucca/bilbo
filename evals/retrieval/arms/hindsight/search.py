"""Search a Hindsight store unpacked in /work/store for the questions in /work/questions.jsonl with Hindsight's recall,
the way its MCP server's recall tool calls it: every fact type, budget high, in the bank of the question's repository,
with query_timestamp at the corpus's last session. Restored and served as check.py does, with the embedder, no LLM and
no worker. Appends one row per question and form to /work/search.jsonl. build-store.sh --search runs it in the build
service."""

import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

STORE = Path("/work/store")
LIVE = Path("/tmp/live")
SERVER = "127.0.0.1:8888"
EMBEDDER_URL = os.environ["EMBEDDER_URL"]
DATABASE_URL = os.environ["DATABASE_URL"]
BUDGET = "high"  # the MCP recall tool's default (API mcp_tools.py:1262); the REST default is mid
MAX_TOKENS = 8192  # of the facts' text; the MCP tool defaults to 4096 in Hindsight's own count, and pack.py cuts at the budget
manifest = json.loads((STORE / "manifest.json").read_text())
recall = manifest["recall"]


def admin(*args):
    r = subprocess.run(["hindsight-admin", *args], env=os.environ | {"HINDSIGHT_API_DATABASE_URL": DATABASE_URL}, stdin=subprocess.DEVNULL,
                       capture_output=True, text=True, timeout=3600)
    if r.returncode:
        sys.exit(f"search: `hindsight-admin {args[0]}` exited with {r.returncode}: {(r.stdout + r.stderr).strip()[-2000:]}")


def api(method, path, body=None):
    request = urllib.request.Request(f"http://{SERVER}{path}", json.dumps(body).encode() if body is not None else None, {"Content-Type": "application/json"}, method=method)
    try:
        with urllib.request.urlopen(request, timeout=600) as r:
            return json.loads(r.read() or b"null")
    except urllib.error.HTTPError as e:
        sys.exit(f"search: {method} {path}: {e.code} {e.read()[:500]!r}")


def day(when):
    return when[:10] if when else ""


def render(r):
    """The fact with the date the recall tool sends beside it: when it happened, else when it was mentioned."""
    text = " ".join(r["text"].split())
    start, end = day(r.get("occurred_start")), day(r.get("occurred_end"))
    if start:
        when = f"happened {start} to {end}" if end and end != start else f"happened {start}"
    elif r.get("mentioned_at"):
        when = f"mentioned {day(r['mentioned_at'])}"
    else:
        return text
    return f"{text} [{when}]"


for directory in (LIVE / "home", LIVE / "hooks"):
    directory.mkdir(parents=True, exist_ok=True)
admin("run-db-migration", "--embedding-dimension", "1024")
admin("restore", str(STORE / "hindsight.zip"), "--yes")
admin("repair-bank", "--all")

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
            sys.exit(f"search: hindsight-api exited with {server.returncode}:\n{log.read_text()[-3000:]}")
        try:
            urllib.request.urlopen(f"http://{SERVER}/health", timeout=5).close()
            break
        except OSError:
            if time.monotonic() - started > 600:
                sys.exit("search: hindsight-api did not answer /health in 10 minutes")
            time.sleep(1)

    banks = {}
    for bank, b in manifest["banks"].items():
        banks.setdefault(b["repo"], []).append(bank)
    rows = [json.loads(line) for line in Path("/work/questions.jsonl").read_text().splitlines() if line.strip()]
    with open("/work/search.jsonl", "a") as out:
        for i, q in enumerate(rows, 1):
            if len(banks.get(q["repo"], [])) != 1:
                sys.exit(f"search: {q['id']}: the store has {len(banks.get(q['repo'], []))} banks for {q['repo']}, not one")
            path = f"/v1/default/banks/{urllib.parse.quote(banks[q['repo']][0], safe='')}/memories/recall"
            for form in ("query", "prompt"):
                answer = api("POST", path, {"query": q[form], "budget": BUDGET, "max_tokens": MAX_TOKENS, "query_timestamp": recall["last_session_time"]})
                hits = [{"text": render(r), "ref": r["id"], "type": r.get("type")} for r in answer["results"]]
                if not hits:
                    sys.exit(f"search: {q['id']} {form}: Hindsight recalled nothing")
                out.write(json.dumps({"id": q["id"], "form": form, "results": hits}, ensure_ascii=False) + "\n")
                out.flush()
            print(f"search: {i}/{len(rows)} {q['id']}", file=sys.stderr)
finally:
    server.terminate()
    try:
        server.wait(60)
    except subprocess.TimeoutExpired:
        server.kill()
