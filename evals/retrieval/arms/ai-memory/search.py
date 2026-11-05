"""Search an ai-memory store unpacked in /work/store for the questions in /work/questions.jsonl, the way an agent
recalls: memory_query over MCP, in the project the hook names for the repository's main checkout, with the embedder,
no LLM and maintenance off. Both forms send their text as the query: with Claude Code, no ai-memory hook searches with
the prompt. Appends one row per question and form to /work/search.jsonl, in the order memory_query returns them.
build-store.sh --search runs it in the build service."""

import http.client
import json
import os
import re
import shutil
import sqlite3
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

STORE = Path("/work/store")
SERVE = Path("/tmp/search")  # the served copy: serving writes to its data directory
BIN = "/usr/local/bin/ai-memory"
SERVER = "127.0.0.1:7801"
WORKSPACE = "default"
LIMIT = 100  # memory_query's maximum; it defaults to 10, and pack.py cuts at the budget
env = {"HOME": str(SERVE / "home"), "PATH": "/usr/local/bin:/usr/bin:/bin", "NO_COLOR": "1",
       "AI_MEMORY_SERVER_URL": f"http://{SERVER}", "AI_MEMORY_EMBEDDING_BASE_URL": os.environ["EMBEDDER_URL"],
       "AI_MEMORY_MAINTENANCE__ENABLED": "false"}


def project_of(repo):
    """The name the hook gives a checkout with this repository's GitHub remote: owner-name, lowercased."""
    return repo.replace("__", "-", 1).lower()


def one_line(text):
    return re.sub(r"\s+", " ", re.sub(r"</?mark>", "", text or "")).strip()


def page_text(hit):
    return f"{hit['path']}: {one_line(hit['title'])}: {one_line(hit['snippet'])}"


def observation_text(hit):
    return f"{hit['kind']} observation, {hit['created_at']}: {one_line(hit['title'])}: {one_line(hit['snippet'])}"


def query(project, text):
    """memory_query over MCP, as an agent calls it: Streamable HTTP, stateless, workspace and project named."""
    args = {"workspace": WORKSPACE, "project": project, "query": text, "limit": LIMIT}
    body = {"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "memory_query", "arguments": args}}
    conn = http.client.HTTPConnection(SERVER, timeout=600)
    conn.request("POST", "/mcp", json.dumps(body),
                 {"Content-Type": "application/json", "Accept": "application/json, text/event-stream"})
    r = conn.getresponse()
    data = r.read().decode()
    answer = json.loads(data) if r.status == 200 else {}
    if not answer.get("result") or answer["result"].get("isError"):
        sys.exit(f"search: memory_query in {WORKSPACE}/{project}: {r.status} {data[:300]}")
    found = json.loads(answer["result"]["content"][0]["text"])
    # The response's own order: the project's pages, the raw observations it falls back to when no page matches, then
    # the standing pages of the _global and profile scopes.
    return ([{"text": page_text(h), "ref": h["path"], "kind": "page", "rank": h["rank"]} for h in found["hits"]] +
            [{"text": observation_text(h), "ref": h["id"], "kind": "observation", "rank": h["rank"]}
             for h in found.get("raw_hits", [])] +
            [{"text": page_text(h), "ref": h["path"], "kind": "global", "rank": h["rank"]}
             for h in found.get("global_scope_hits", [])])


rows = [json.loads(line) for line in Path("/work/questions.jsonl").read_text().splitlines() if line.strip()]
db = sqlite3.connect(f"file:{STORE / 'db' / 'memory.sqlite'}?mode=ro&immutable=1", uri=True)
named = {r[0] for r in db.execute("SELECT p.name FROM projects p JOIN workspaces w ON w.id = p.workspace_id"
                                  " WHERE w.name = ?", (WORKSPACE,))}
# The server refuses a name that is also another project's canonical or legacy key as ambiguous.
taken = {r[0] for r in db.execute("SELECT o.name FROM projects p JOIN projects o ON o.workspace_id = p.workspace_id"
                                  " AND o.id <> p.id AND (p.canonical_name = o.name OR p.legacy_name = o.name)")}
db.close()
for project in sorted({project_of(q["repo"]) for q in rows}):
    if project not in named or project in taken:
        sys.exit(f"search: the store has no project {project} that memory_query can name")

shutil.rmtree(SERVE, ignore_errors=True)
shutil.copytree(STORE, SERVE / "data", ignore=shutil.ignore_patterns("manifest.json"))
(SERVE / "home").mkdir()
log = SERVE / "serve.log"
server = subprocess.Popen([BIN, "--data-dir", str(SERVE / "data"), "serve", "--transport", "http", "--bind", SERVER],
                          env=env, cwd=SERVE / "home", stdin=subprocess.DEVNULL, stdout=log.open("w"),
                          stderr=subprocess.STDOUT)
try:
    for _ in range(600):
        if server.poll() is not None:
            sys.exit(f"search: ai-memory serve exited with {server.returncode}:\n{log.read_text()[-3000:]}")
        try:
            urllib.request.urlopen(f"http://{SERVER}/healthz", timeout=2).close()
            break
        except OSError:
            time.sleep(0.5)
    else:
        sys.exit("search: ai-memory serve did not answer /healthz in 5 minutes")
    with open("/work/search.jsonl", "a") as out:
        for i, q in enumerate(rows, 1):
            for form in ("query", "prompt"):
                hits = query(project_of(q["repo"]), q[form])
                # A query whose embedding fails still answers, from full text alone (server.rs: "degrading memory_query").
                if "degrading memory_query" in log.read_text():
                    sys.exit(f"search: {q['id']} {form}: the embedder failed, so the query ran without vectors")
                if not hits:
                    sys.exit(f"search: {q['id']} {form}: ai-memory found nothing")
                out.write(json.dumps({"id": q["id"], "form": form, "results": hits}, ensure_ascii=False) + "\n")
                out.flush()
            print(f"search: {i}/{len(rows)} {q['id']}", file=sys.stderr)
finally:
    server.terminate()
    try:
        server.wait(60)
    except subprocess.TimeoutExpired:
        server.kill()
