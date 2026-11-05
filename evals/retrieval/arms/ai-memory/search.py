"""Search an ai-memory store unpacked in /work/store for the questions in /work/questions.jsonl, the way an agent
recalls: memory_query over MCP, with the embedder, no LLM and maintenance off, scoped to every ai-memory project that
holds a session of the question's repository (the build's manifest lists them), which leaves out the `_global` scope,
then memory_read_page for the top READ pages, as its MCP instructions tell an agent to read a page past its snippet.
Both forms send their text as the query: with Claude Code, no ai-memory hook searches with the prompt. Appends one row
per question and form to /work/search.jsonl, in the order memory_query returns them, the read pages whole and the rest
as snippets. build-store.sh --search runs it in the build service."""

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
READ = 3  # the eval's k: pages an agent opens whole
MAX_SCOPES = 25  # memory_query refuses more (MAX_QUERY_SCOPES, crates/ai-memory-mcp/src/server.rs:576)
env = {"HOME": str(SERVE / "home"), "PATH": "/usr/local/bin:/usr/bin:/bin", "NO_COLOR": "1",
       "AI_MEMORY_SERVER_URL": f"http://{SERVER}", "AI_MEMORY_EMBEDDING_BASE_URL": os.environ["EMBEDDER_URL"],
       "AI_MEMORY_MAINTENANCE__ENABLED": "false"}


def one_line(text):
    return re.sub(r"\s+", " ", re.sub(r"</?mark>", "", text or "")).strip()


def page_text(hit):
    return f"{hit['path']}: {one_line(hit['title'])}: {one_line(hit['snippet'])}"


def observation_text(hit):
    return f"{hit['kind']} observation, {hit['created_at']}: {one_line(hit['title'])}: {one_line(hit['snippet'])}"


def mcp(tool, args):
    """One MCP tool call, as an agent makes it: Streamable HTTP, stateless."""
    body = {"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": args}}
    conn = http.client.HTTPConnection(SERVER, timeout=600)
    conn.request("POST", "/mcp", json.dumps(body),
                 {"Content-Type": "application/json", "Accept": "application/json, text/event-stream"})
    r = conn.getresponse()
    data = r.read().decode()
    answer = json.loads(data) if r.status == 200 else {}
    if not answer.get("result") or answer["result"].get("isError"):
        sys.exit(f"search: {tool} {json.dumps(args)[:200]}: {r.status} {data[:300]}")
    return json.loads(answer["result"]["content"][0]["text"])


def call(projects, text):
    """memory_query with the projects named as scopes."""
    found = mcp("memory_query", {"scopes": [{"workspace": WORKSPACE, "project": p} for p in projects], "query": text,
                                 "limit": LIMIT})
    if found.get("global_scope_hits"):
        sys.exit(f"search: memory_query with scopes returned _global hits for {','.join(projects)}")
    return found["hits"], found.get("raw_hits", [])


def read_page(hit):
    """memory_read_page by the hit's path in the project that holds it. A scoped hit does not name its project; an agent
    would try the scopes in turn, and the page's id in the store says which one answers."""
    project = page_project.get(hit["id"].replace("-", "").lower())
    if not project:
        sys.exit(f"search: page {hit['id']} ({hit['path']}) is in no project of the store")
    page = mcp("memory_read_page", {"path": hit["path"], "workspace": WORKSPACE, "project": project})
    return f"{page['path']}: {one_line(page['title'])}\n{page['body']}"


def query(projects, text):
    """The response's own order: the scopes' pages by rank, else the raw observations it falls back to. Past
    MAX_SCOPES projects, one call per chunk, merged as the server merges its scopes (rank, then path), so the result is
    the one call the server would answer with a larger cap."""
    pages, raw = [], []
    for i in range(0, len(projects), MAX_SCOPES):
        p, o = call(projects[i:i + MAX_SCOPES], text)
        pages += p
        raw += o
    pages = sorted(pages, key=lambda h: (h["rank"], h["path"]))[:LIMIT]
    raw = [] if pages else sorted(raw, key=lambda h: h["rank"])[:LIMIT]
    return ([{"text": read_page(h), "ref": h["path"], "kind": "page", "rank": h["rank"], "read": True}
             for h in pages[:READ]] +
            [{"text": page_text(h), "ref": h["path"], "kind": "page", "rank": h["rank"]} for h in pages[READ:]] +
            [{"text": observation_text(h), "ref": h["id"], "kind": "observation", "rank": h["rank"]} for h in raw])


rows = [json.loads(line) for line in Path("/work/questions.jsonl").read_text().splitlines() if line.strip()]
db = sqlite3.connect(f"file:{STORE / 'db' / 'memory.sqlite'}?mode=ro&immutable=1", uri=True)
named = {r[0] for r in db.execute("SELECT p.name FROM projects p JOIN workspaces w ON w.id = p.workspace_id"
                                  " WHERE w.name = ?", (WORKSPACE,))}
# The server refuses a name that is also another project's canonical or legacy key as ambiguous.
taken = {r[0] for r in db.execute("SELECT o.name FROM projects p JOIN projects o ON o.workspace_id = p.workspace_id"
                                  " AND o.id <> p.id AND (p.canonical_name = o.name OR p.legacy_name = o.name)")}
page_project = {r[0].hex(): r[1] for r in db.execute(
    "SELECT g.id, p.name FROM pages g JOIN projects p ON p.id = g.project_id JOIN workspaces w ON w.id = g.workspace_id"
    " WHERE w.name = ? AND g.is_latest = 1", (WORKSPACE,))}
db.close()
held = json.loads((STORE / "manifest.json").read_text())["projects"]
scopes = {}
for repo in sorted({q["repo"] for q in rows}):
    projects = sorted(held.get(repo, {}))
    absent = [p for p in projects if p not in named]
    if absent or not projects:
        sys.exit(f"search: the store has no project {', '.join(absent) or 'at all'} for {repo}")
    # A real user cannot name these either; build.py's --check leaves them out too.
    for p in projects:
        if p in taken:
            print(f"search: {repo}: {p} is another project's key, so memory_query cannot name it; left out",
                  file=sys.stderr)
    scopes[repo] = [p for p in projects if p not in taken]
    if not scopes[repo]:
        sys.exit(f"search: memory_query can name no project of {repo}")

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
                hits = query(scopes[q["repo"]], q[form])
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
