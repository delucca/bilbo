"""Search a claude-mem store unpacked in /work/store for the questions in /work/questions.jsonl, served by claude-mem's
worker with the embedder and no LLM, as a later session of the question's project would reach it. The query form is
the MCP tool mem_search in auto mode, the one claude-mem's instructions tell an agent to call; the prompt form is the
SessionStart injection, since the hook that sees the prompt searches with it only when CLAUDE_MEM_SEMANTIC_INJECT is
on, which it is not by default. Both scope to every project key the build filed the repository's sessions under.
The worker's clock reads the repository's last event (clock.js), so mem_search's 90-day window, which it cannot widen,
ends where a session right after the last one would see it end. Appends one row per question and form to
/work/search.jsonl. build-store.sh --search runs it in the build service."""

import json
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
SERVE = Path("/tmp/search")
CLOCK = SERVE / "clock"
PORT = 37777
EMBEDDER = os.environ["EMBEDDER_URL"]
# mem_search's ceilings (mcp-server.ts inputSchema); its defaults are 20 and 3, and pack.py cuts at the budget.
LIMIT, MAX_DETAILS = 20, 5
BACKFILLED = ("Backfill check complete for all projects", "Backfill check ended before every project finished")
LLM_ATTEMPTS = ("Generator auto-starting", "Starting SDK query")
manifest = json.loads((STORE / "manifest.json").read_text())


def api(path, body=None, timeout=600):
    data = None if body is None else json.dumps(body).encode()
    request = urllib.request.Request(f"http://127.0.0.1:{PORT}{path}", data, {"Content-Type": "application/json"},
                                     method="GET" if body is None else "POST")
    with urllib.request.urlopen(request, timeout=timeout) as r:
        raw = r.read()
    return json.loads(raw) if r.headers.get_content_type() == "application/json" else raw.decode()


def logged():
    return "".join(p.read_text(errors="replace") for p in sorted((SERVE / "data" / "logs").glob("*.log")))


def scope(repo, db):
    """The repository's project keys, the one with the most sessions last (the primary, as the SessionStart hook
    orders them), and the time of its last event, in epoch milliseconds."""
    keys = manifest["projects"].get(repo)
    if not keys:
        sys.exit(f"search: the manifest lists no project for {repo}")
    keys = sorted(keys, key=lambda k: (keys[k], k))
    marks = ",".join("?" * len(keys))
    last = max(v or 0 for (v,) in db.execute(
        f"SELECT MAX(created_at_epoch) FROM observations WHERE project IN ({marks}) UNION ALL "
        f"SELECT MAX(created_at_epoch) FROM session_summaries WHERE project IN ({marks}) UNION ALL "
        f"SELECT MAX(p.created_at_epoch) FROM user_prompts p JOIN sdk_sessions s ON s.id = p.session_db_id "
        f"WHERE s.project IN ({marks})", keys * 3))
    if not last:
        sys.exit(f"search: {repo} has no rows under {keys}")
    return keys, last


def split_search(text):
    """mem_search's rendering (progressive-search.ts renderProgressiveSearchResult), one result per detail or, when
    no title matched the query, per index line. The protocol lines around them carry no memory."""
    if "\n### [" in text:
        return [part.strip() for part in text.split("\n### ")[1:] if part.strip()]
    return [line[2:].strip() for line in text.splitlines() if line.startswith("- [")]


def main():
    shutil.rmtree(SERVE, ignore_errors=True)
    (SERVE / "home").mkdir(parents=True)
    data = SERVE / "data"
    shutil.copytree(STORE, data, ignore=shutil.ignore_patterns("manifest.json"))
    db = sqlite3.connect(data / "claude-mem.db")
    rows = [json.loads(line) for line in Path("/work/questions.jsonl").read_text().splitlines() if line.strip()]
    scopes = {repo: scope(repo, db) for repo in {q["repo"] for q in rows}}
    db.close()
    # clock.js never moves back, so the projects go in the order of their last event.
    rows.sort(key=lambda q: (scopes[q["repo"]][1], q["repo"]))
    CLOCK.write_text(str(scopes[rows[0]["repo"]][1]))

    env = {"HOME": str(SERVE / "home"), "PATH": "/usr/local/bin:/usr/bin:/bin", "CLAUDE_MEM_DATA_DIR": str(data),
           "CLAUDE_MEM_WORKER_PORT": str(PORT), "CLAUDE_MEM_WORKER_AUTOSTART": "false", "CLAUDE_MEM_TELEMETRY": "0",
           "OPENAI_BASE_URL": EMBEDDER, "OPENAI_API_KEY": "local",
           "BUN_OPTIONS": "--preload /app/clock.js", "BILBO_CLOCK_FILE": str(CLOCK)}
    env |= {k: v for k, v in os.environ.items() if k.startswith("UV_")}
    worker = subprocess.Popen(["bun", "/opt/claude-mem/plugin/scripts/worker-service.cjs", "--daemon"], env=env,
                              cwd=SERVE / "home", stdin=subprocess.DEVNULL, stdout=(SERVE / "worker.out").open("w"),
                              stderr=subprocess.STDOUT, start_new_session=True)
    asked_to_stop = False
    try:
        until = time.monotonic() + 600
        while not any(m in logged() for m in BACKFILLED):
            if worker.poll() is not None:
                sys.exit(f"search: the worker exited with {worker.returncode}:\n"
                         f"{(SERVE / 'worker.out').read_text()[-3000:]}")
            if time.monotonic() > until:
                sys.exit("search: the worker did not end its backfill check in 10 minutes")
            time.sleep(0.5)
        if api("/api/chroma/status?deep=1").get("status") != "healthy":
            sys.exit("search: Chroma is not healthy")

        with open("/work/search.jsonl", "a") as out:
            for i, q in enumerate(rows, 1):
                keys, last = scopes[q["repo"]]
                CLOCK.write_text(str(last))
                primary, projects = keys[-1], ",".join(keys)
                found = api("/api/mem-search", {"mode": "auto", "query": q["query"], "project": primary,
                                                "limit": LIMIT, "maxDetails": MAX_DETAILS,
                                                "searchScope": projects, "projects": projects})
                text = found["content"][0]["text"]
                if found.get("isError"):
                    sys.exit(f"search: {q['id']}: mem_search failed: {text}")
                hits = [{"text": t, "ref": t.split("]", 1)[0].lstrip("[")} for t in split_search(text)]
                out.write(json.dumps({"id": q["id"], "form": "query", "results": hits}, ensure_ascii=False) + "\n")

                params = urllib.parse.urlencode({"projects": projects, "platformSource": "claude"})
                context = api(f"/api/context/inject?{params}").strip()
                if not context:
                    sys.exit(f"search: {q['id']}: the SessionStart injection for {projects} is empty")
                out.write(json.dumps({"id": q["id"], "form": "prompt", "results": [{"text": context}]},
                                     ensure_ascii=False) + "\n")
                out.flush()
                print(f"search: {i}/{len(rows)} {q['id']} ({len(hits)} results)", file=sys.stderr)
        tried = [m for m in LLM_ATTEMPTS if m in logged()]
        if tried:
            sys.exit(f"search: the worker tried to start the observer: {tried}")
        asked_to_stop = True
        try:
            api("/api/admin/shutdown", {}, timeout=10)
        except OSError:
            pass  # the shutdown ends the connection
    except OSError:
        print(f"search: the worker's output:\n{(SERVE / 'worker.out').read_text()[-3000:]}\n"
              f"search: its log:\n{logged()[-6000:]}", file=sys.stderr)
        raise
    finally:
        if not asked_to_stop and worker.poll() is None:
            worker.terminate()
        try:
            worker.wait(120 if asked_to_stop else 30)
        except subprocess.TimeoutExpired:
            worker.kill()
        subprocess.run(["pkill", "-9", "-f", "chroma-mcp"])


main()
