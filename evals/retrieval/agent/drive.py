"""Answer each question in /work/questions.jsonl with a real Claude Code agent on the arm's own memory surface, and keep
what it read. The agent gets the question's `prompt`, the tool as Claude Code would have it (bilbo's recall skill and
CLI, ai-memory's MCP server, claude-mem's mem_search and its skill, Claude Code's own auto memory) and nothing else,
and a reading budget: budget.py refuses its calls once the memory it read reaches BUDGET tokens. Scope is the eval's,
one project per search: the arm's search.py sets it up, and a proxy pins it on every MCP call, so the agent chooses the
words, the calls and what it opens, not the projects. Appends one row per question to /work/search.jsonl, form
`query`, the results being what the agent read in order, and keeps each transcript in /work/agent/.
build-store.sh --search --driver runs it in the build service, with the arm's search.py at /work/search.py."""

import http.client
import http.server
import json
import os
import re
import shutil
import sqlite3
import subprocess
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, "/work")
import budget  # noqa: E402
import search  # noqa: E402  (the arm's own search.py)

ARM = os.environ["BILBO_EVAL_ARM"]
MODEL = os.environ.get("BILBO_EVAL_AGENT_MODEL") or "claude-sonnet-5-5"
BUDGET = 4000
MAX_TURNS = 30
TIMEOUT = 900
DRIVER = Path("/work/driver")
CLAUDE = DRIVER / "claude"
LOGS = Path("/work/agent")
SCRATCH = Path("/tmp/agent")
BYTES_PER_TOKEN = 4

SYSTEM = """This session runs in the {repo} repository, for a developer who asks about earlier work on it. The \
repository's code is not available here: answer from the project's memory, {memory}. You can read about \
{budget:,} tokens of tool output in this session; past that, tool calls are refused, so read what matters most first. \
Answer briefly, as well as the memory allows."""


def text_of(content):
    if isinstance(content, str):
        return content
    return "\n".join(c.get("text", "") for c in content or [] if c.get("type") == "text")


def claude_key(path):
    """The folder Claude Code keeps a working directory's sessions and auto memory in."""
    return re.sub(r"[^a-zA-Z0-9]", "-", str(path))


def fresh_home(qid):
    home = SCRATCH / "home" / qid
    shutil.rmtree(home, ignore_errors=True)
    (home / ".claude").mkdir(parents=True)
    # Onboarding and trust are a person's steps; a fresh home would otherwise stop at them.
    (home / ".claude.json").write_text(json.dumps({"hasCompletedOnboarding": True, "bypassPermissionsModeAccepted": True}))
    return home


def run_agent(q, *, home, cwd, env, memory, tools, mcp=None, reads, roots=(), commands=(), initial="",
              settings=None):
    """One claude -p session on q's prompt; returns its row for search.jsonl."""
    cwd.mkdir(parents=True, exist_ok=True)
    state = SCRATCH / f"state-{q['id']}.json"
    state.write_text(json.dumps({"budget": BUDGET, "initial": len(initial.encode()) / BYTES_PER_TOKEN,
                                 "reads": list(reads), "roots": [os.path.realpath(r) for r in roots],
                                 "commands": list(commands)}))
    hook = {"type": "command", "command": f"{sys.executable} {DRIVER / 'budget.py'}"}
    config = {"hooks": {"PreToolUse": [{"matcher": "*", "hooks": [hook]}]}, "autoMemoryEnabled": False,
              "includeCoAuthoredBy": False} | (settings or {})
    argv = [str(CLAUDE), "-p", q["prompt"], "--model", MODEL, "--output-format", "json", "--max-turns", str(MAX_TURNS),
            "--permission-mode", "bypassPermissions", "--tools", ",".join(tools), "--strict-mcp-config",
            "--setting-sources", "user", "--settings", json.dumps(config),
            "--append-system-prompt", SYSTEM.format(repo=q["repo"].replace("__", "/"), memory=memory, budget=BUDGET)]
    if mcp:
        argv += ["--mcp-config", json.dumps({"mcpServers": mcp})]
    full_env = env | {"HOME": str(home), "BILBO_AGENT_STATE": str(state), "DISABLE_AUTOUPDATER": "1",
                      "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1", "DISABLE_TELEMETRY": "1",
                      "CLAUDE_CODE_OAUTH_TOKEN": os.environ["CLAUDE_CODE_OAUTH_TOKEN"]}
    started = time.monotonic()
    p = subprocess.run(argv, cwd=cwd, env=full_env, stdin=subprocess.DEVNULL, capture_output=True, text=True,
                       timeout=TIMEOUT)
    try:
        result = json.loads(p.stdout)
    except ValueError:
        sys.exit(f"drive: {q['id']}: claude exit {p.returncode}: {p.stdout[-500:]!r} {p.stderr[-500:]!r}")
    if result.get("is_error") and result.get("subtype") != "error_max_turns":
        sys.exit(f"drive: {q['id']}: claude failed: {json.dumps(result)[:800]}")
    found = list(home.glob(f".claude/projects/*/{result['session_id']}.jsonl"))
    if len(found) != 1:
        sys.exit(f"drive: {q['id']}: no transcript for session {result['session_id']}")
    LOGS.mkdir(exist_ok=True)
    shutil.copyfile(found[0], LOGS / f"{q['id']}.jsonl")

    calls, results, by_id = [], [], {}
    if initial.strip():
        results.append({"text": initial, "ref": "loaded at start", "tool": "startup"})
    for line in found[0].read_text().splitlines():
        entry = json.loads(line)
        for c in (entry.get("message") or {}).get("content") or []:
            if not isinstance(c, dict):
                continue
            if c.get("type") == "tool_use":
                by_id[c["id"]] = len(calls)
                calls.append({"tool": c["name"], "input": c["input"]})
            elif c.get("type") == "tool_result" and c.get("tool_use_id") in by_id:
                call = calls[by_id[c["tool_use_id"]]]
                text = text_of(c.get("content"))
                call |= {"error": bool(c.get("is_error")), "tokens": round(len(text.encode()) / BYTES_PER_TOKEN)}
                if not c.get("is_error") and budget.reads_memory({"name": call["tool"], "input": call["input"]}, reads):
                    results.append({"text": text, "ref": ref_of(call), "tool": call["tool"]})
                elif c.get("is_error"):
                    call["said"] = text[:300]
    return {"id": q["id"], "form": "query", "results": results,
            "agent": {"model": MODEL, "calls": calls, "answer": result.get("result", ""),
                      "turns": result.get("num_turns"), "stop": result.get("subtype"),
                      "seconds": round(time.monotonic() - started, 1), "usage": result.get("usage")}}


def ref_of(call):
    args = call.get("input") or {}
    return args.get("file_path") or args.get("path") or args.get("command") or json.dumps(args, ensure_ascii=False)


class Proxy(threading.Thread):
    """A reverse proxy on 127.0.0.1:<port> to <upstream>; rewrite(path, body) may change a JSON request body."""

    def __init__(self, port, upstream, rewrite):
        super().__init__(daemon=True)
        proxy = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def forward(self):
                size = int(self.headers.get("Content-Length") or 0)
                body = self.rfile.read(size) if size else None
                if body and "json" in (self.headers.get("Content-Type") or ""):
                    body = json.dumps(proxy.rewrite(self.path, json.loads(body))).encode()
                conn = http.client.HTTPConnection(proxy.upstream, timeout=600)
                headers = {k: v for k, v in self.headers.items() if k.lower() not in ("content-length", "host")}
                conn.request(self.command, self.path, body, headers | {"Host": proxy.upstream})
                r = conn.getresponse()
                data = r.read()
                self.send_response(r.status)
                for k, v in r.getheaders():
                    if k.lower() not in ("content-length", "transfer-encoding", "connection"):
                        self.send_header(k, v)
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            do_GET = do_POST = do_DELETE = forward

        self.upstream, self.rewrite = upstream, rewrite
        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", port), Handler)

    def run(self):
        self.server.serve_forever()


def drive_bilbo(rows):
    projects = json.loads((search.STORE / "manifest.json").read_text())["repos"]
    bin_dir = SCRATCH / "bin"
    bin_dir.mkdir(parents=True, exist_ok=True)
    # The sentinel before each search, as search.py sends it, so a query's vector depends on its own text alone.
    (bin_dir / "bilbo").write_text(f"#!{sys.executable}\nimport os, sys\nsys.path.insert(0, '/work')\nimport search\n"
                                   f"search.sentinel()\nos.execv({search.BILBO!r}, [{search.BILBO!r}, *sys.argv[1:]])\n")
    (bin_dir / "bilbo").chmod(0o755)
    rows.sort(key=lambda q: q["repo"])
    for i, q in enumerate(rows):
        if i == 0 or q["repo"] != rows[i - 1]["repo"]:
            search.scope(q["repo"], projects[q["repo"]]["notes"])
        home = fresh_home(q["id"])
        shutil.copytree(DRIVER / "skills" / "recall", home / ".claude" / "skills" / "recall")
        env = search.env() | {"PATH": f"{bin_dir}:/usr/bin:/bin", "EMBEDDER_URL": search.EMBEDDER}
        yield run_agent(q, home=home, cwd=SCRATCH / "cwd" / q["repo"], env=env, memory="its bilbo notes",
                        tools=["Bash", "Read", "Skill"], reads=["Bash", "Read"], roots=[search.SCOPED / "notes"],
                        commands=[r"bilbo recall( .*)?", r"command -v bilbo"])


def drive_ai_memory(rows):
    scopes = search.load(rows)
    db = sqlite3.connect(f"file:{search.STORE / 'db' / 'memory.sqlite'}?mode=ro&immutable=1", uri=True)
    holder = {}
    for path, project in db.execute("SELECT g.path, p.name FROM pages g JOIN projects p ON p.id = g.project_id JOIN "
                                    "workspaces w ON w.id = g.workspace_id WHERE w.name = ? AND g.is_latest = 1",
                                    (search.WORKSPACE,)):
        holder.setdefault(path, []).append(project)
    db.close()
    current = {}

    def primary(repo):
        named = repo.replace("__", "-")
        return named if named in scopes[repo] else scopes[repo][0]

    def rewrite(path, body):
        """Every tool call in the question's projects, as search.py scopes memory_query; any other scope is dropped."""
        if body.get("method") != "tools/call":
            return body
        repo, args = current["repo"], dict(body["params"].get("arguments") or {})
        for k in ("workspace", "project", "scopes", "global", "cwd"):
            args.pop(k, None)
        name = body["params"]["name"]
        if name == "memory_query":
            if len(scopes[repo]) > search.MAX_SCOPES:
                sys.exit(f"drive: {repo} has more projects than memory_query takes")
            args["scopes"] = [{"workspace": search.WORKSPACE, "project": p} for p in scopes[repo]]
        else:
            held = [p for p in holder.get(args.get("path") or "", []) if p in scopes[repo]]
            args |= {"workspace": search.WORKSPACE, "project": held[0] if held else primary(repo)}
        return body | {"params": body["params"] | {"arguments": args}}

    Proxy(7802, search.SERVER, rewrite).start()
    read_only = ["memory_query", "memory_read_page", "memory_recent", "memory_briefing", "memory_status",
                 "memory_read_session_observations"]
    with search.serve() as log:
        for q in rows:
            current["repo"] = q["repo"]
            row = run_agent(q, home=fresh_home(q["id"]), cwd=SCRATCH / "cwd" / q["repo"], env=search.env,
                            memory=f"ai-memory's MCP tools (workspace `{search.WORKSPACE}`, project "
                                   f"`{primary(q['repo'])}`)",
                            tools=[], mcp={"ai-memory": {"type": "http", "url": "http://127.0.0.1:7802/mcp"}},
                            reads=[f"mcp__ai-memory__{t}" for t in read_only])
            if "degrading memory_query" in log.read_text():
                sys.exit(f"drive: {q['id']}: the embedder failed, so a query ran without vectors")
            yield row


def drive_claude_mem(rows):
    scopes = search.load(rows)
    current = {}

    def rewrite(path, body):
        """mem_search in the question's project keys, as search.py passes them."""
        if not path.startswith("/api/mem-search"):
            return body
        keys = scopes[current["repo"]][0]
        body = {k: v for k, v in body.items() if k not in ("project", "projects", "searchScope")}
        if "continuation" not in body:
            body |= {"project": keys[-1], "projects": ",".join(keys)}
        return body | {"searchScope": ",".join(keys)}

    Proxy(search.PORT + 1, f"127.0.0.1:{search.PORT}", rewrite).start()
    with search.serve(scopes[rows[0]["repo"]][1]) as (data, worker_env):
        mcp_env = worker_env | {"CLAUDE_MEM_WORKER_PORT": str(search.PORT + 1)}
        mcp_env.pop("BUN_OPTIONS")
        for q in rows:
            keys, last = scopes[q["repo"]]
            search.CLOCK.write_text(str(last))
            current["repo"] = q["repo"]
            home = fresh_home(q["id"])
            shutil.copytree("/opt/claude-mem/plugin/skills/mem-search", home / ".claude" / "skills" / "mem-search")
            server = {"type": "stdio", "command": "bun", "args": ["/opt/claude-mem/plugin/scripts/mcp-server.cjs"],
                      "env": mcp_env | {"HOME": str(home)}}
            yield run_agent(q, home=home, cwd=SCRATCH / "cwd" / q["repo"], env={"PATH": worker_env["PATH"]},
                            memory=f"claude-mem's mem_search (project `{keys[-1]}`)", tools=["Skill"],
                            mcp={"mcp-search": server}, reads=["mcp__mcp-search__mem_search"])


def drive_auto_memory(rows):
    for q in rows:
        key = search.keys_of(q["repo"])[0]
        home = fresh_home(q["id"])
        # Claude Code keys auto memory by the working directory, which here is not the developer's path: the files go
        # under this session's key, unchanged. The archive's epoch mtimes would make Claude Code call every file
        # decades stale, so they get the time of the copy.
        # A Windows root (superset-windows-arm64) is not a POSIX path: its last folder is the checkout's name.
        name = re.split(r"[\\/]", search.roots.get(q["repo"], q["repo"]).rstrip("\\/"))[-1]
        cwd = SCRATCH / "checkouts" / re.sub(r"[^A-Za-z0-9._-]", "-", name)
        memory = home / ".claude" / "projects" / claude_key(cwd) / "memory"
        shutil.copytree(search.STORE / "memory" / key, memory, copy_function=shutil.copyfile)
        index = search.loaded_index(key)
        initial = f"{memory / 'MEMORY.md'}:\n{index}" if index.strip() else ""
        yield run_agent(q, home=home, cwd=cwd, env={"PATH": "/usr/bin:/bin"}, memory="your auto memory",
                        tools=["Read", "Glob", "Grep"], reads=["Read", "Glob", "Grep"], roots=[memory],
                        initial=initial, settings={"autoMemoryEnabled": True})


DRIVERS = {"bilbo-agent-v0_19": drive_bilbo, "bilbo-assisted-v0_19": drive_bilbo, "ai-memory": drive_ai_memory,
           "claude-mem": drive_claude_mem, "auto-memory": drive_auto_memory}


def main():
    if ARM not in DRIVERS:
        sys.exit(f"drive: no agent driver for {ARM}")
    rows = [json.loads(line) for line in Path("/work/questions.jsonl").read_text().splitlines() if line.strip()]
    SCRATCH.mkdir(parents=True, exist_ok=True)
    with open("/work/search.jsonl", "a") as out:
        for i, row in enumerate(DRIVERS[ARM](rows), 1):
            out.write(json.dumps(row, ensure_ascii=False) + "\n")
            out.flush()
            a = row["agent"]
            print(f"drive: {i}/{len(rows)} {row['id']}: {len(a['calls'])} calls, {len(row['results'])} read, "
                  f"{a['seconds']}s, {a['stop']}", file=sys.stderr)


if __name__ == "__main__":
    main()
