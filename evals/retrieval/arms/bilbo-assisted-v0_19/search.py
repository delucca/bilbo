"""Search a bilbo store unpacked in /work/store for the questions in /work/questions.jsonl the way bilbo 0.19.0 reads
for an agent, over the notes of the question's project: each project's notes are copied into a store of their own and
indexed, as if the developer's bilbo held only that project. The `query` form goes through `bilbo recall`, as the
recall skill runs it, then the agent's Read of the top READ notes in full, the rest as recall printed them; the
`prompt` form through `bilbo digest`, as the prompt hook runs it, each session's first digest, as injected. Appends one
row per question and form to /work/search.jsonl. build-store.sh --search runs it in the build service."""

import json
import os
import re
import shutil
import subprocess
import sys
import urllib.request
import uuid
from pathlib import Path

STORE = Path("/work/store")
# One project's store at a time, always at this root: a note's path reads the same whatever its project.
SCOPED = Path("/tmp/store")
BILBO = "/usr/local/bin/bilbo"
EMBEDDER = os.environ["EMBEDDER_URL"].rstrip("/")
# recall prints 10 by default; the budget, not bilbo, cuts, so ask for more than 4,000 tokens hold.
LIMIT = 50
# The recall skill prints snippets and allows Read: an agent opens the best notes whole (the eval's k).
READ = 3
# The digest gives the embedder 1.2 seconds and falls back to keywords past it; a retry asks again in a new session.
DIGEST_TRIES = 3
SENTINEL = "\u0000"
BLOCK_HEAD = re.compile(r"^(/\S+\.md):\d+\t[a-z]+\t\S+$")
DIGEST_LINE = re.compile(r"^- (/\S+\.md):\d+ \(")


def env():
    scratch = Path("/tmp/search-home")
    for d in ("home", "config", "data", "state"):
        (scratch / d).mkdir(parents=True, exist_ok=True)
    return {
        "HOME": str(scratch / "home"), "BILBO_HOME": str(SCOPED), "BILBO_CONFIG": str(SCOPED / "config"),
        "XDG_CACHE_HOME": str(SCOPED / "cache"), "XDG_CONFIG_HOME": str(scratch / "config"),
        "XDG_DATA_HOME": str(scratch / "data"), "XDG_STATE_HOME": str(scratch / "state"),
        "PATH": "/usr/bin:/bin", "NO_COLOR": "1", "LANG": "en_US.UTF-8", "LC_ALL": "en_US.UTF-8",
    }  # fmt: skip


def scope(repo, notes):
    """A store holding only this project's notes, indexed on a fresh cache after a sentinel, as the build indexes."""
    shutil.rmtree(SCOPED, ignore_errors=True)
    (SCOPED / "notes").mkdir(parents=True)
    (SCOPED / "cache").mkdir()
    shutil.copyfile(STORE / "config", SCOPED / "config")
    for name in notes:
        shutil.copyfile(STORE / "notes" / name, SCOPED / "notes" / name)
    sentinel()
    p = bilbo("index")
    m = re.search(r"^embedded (\d+), kept (\d+), dropped (\d+)$", p.stdout, re.M)
    if p.returncode != 0 or not m or m[2] != "0" or m[3] != "0":
        sys.exit(f"search: {repo}: bilbo index did not embed a fresh cache (exit {p.returncode}): {p.stdout[-300:]!r} {p.stderr[-300:]!r}")
    print(f"search: {repo}: {len(notes)} notes, {m[1]} passages embedded", file=sys.stderr)


def sentinel():
    """llama.cpp b11206 reuses the prefix a request shares with the one before it, so each query vector would depend on
    the previous search; a sentinel first makes it depend only on its own text (the store manifest's `search`)."""
    request = urllib.request.Request(EMBEDDER + "/v1/embeddings", json.dumps({"input": [SENTINEL]}).encode(),
                                     {"Content-Type": "application/json"})
    urllib.request.urlopen(request, timeout=600).read()


def bilbo(*args, stdin=None):
    return subprocess.run([BILBO, *args], input=stdin, env=env(), cwd="/tmp/search-home", capture_output=True,
                          text=True, timeout=600)


def recall(qid, text):
    sentinel()
    p = bilbo("recall", "--limit", str(LIMIT), "--", text)
    err = p.stderr.strip()
    if "not indexed" in err or "embedder unavailable" in err:
        sys.exit(f"search: {qid} query: recall did not use the vectors: {err[:300]!r}")
    if p.returncode == 1 and err.splitlines()[-1:] == ["bilbo: no notes match"]:
        return []
    if p.returncode != 0 or err:
        sys.exit(f"search: {qid} query: recall exit {p.returncode}: {err[:300]!r}")
    hits = []
    for block in p.stdout.strip().split("\n\n"):
        m = BLOCK_HEAD.match(block.split("\n", 1)[0])
        if not m:
            sys.exit(f"search: {qid} query: a recall block does not open with path, kind and created: {block[:200]!r}")
        hit = {"text": block, "ref": m[1], "rank": len(hits) + 1}
        if len(hits) < READ:
            hit |= {"text": f"{m[1]}\n{Path(m[1]).read_text()}", "read": True}
        hits.append(hit)
    return hits


def digest(qid, text):
    for _ in range(DIGEST_TRIES):
        sentinel()
        p = bilbo("digest", stdin=json.dumps({"session_id": f"eval-{uuid.uuid4()}", "prompt": text}))
        err = p.stderr.strip()
        if p.returncode != 0:
            sys.exit(f"search: {qid} prompt: digest exit {p.returncode}: {err[:300]!r}")
        if err:
            print(f"search: {qid} prompt: digest said {err[:300]!r}; asking again", file=sys.stderr)
            continue
        hits = []
        for line in p.stdout.splitlines():
            m = DIGEST_LINE.match(line)
            if m:
                hits.append({"text": line[2:], "ref": m[1], "rank": len(hits) + 1})
            elif not (line.startswith("<!-- bilbo digest: ") or line.startswith("Notes that may bear on this prompt")
                      or re.match(r"^\(\d+ more passed; run bilbo recall for them\)$", line)):
                sys.exit(f"search: {qid} prompt: an unexpected digest line: {line[:200]!r}")
        return hits
    sys.exit(f"search: {qid} prompt: the digest did not rank by meaning in {DIGEST_TRIES} tries")


rows = [json.loads(line) for line in Path("/work/questions.jsonl").read_text().splitlines() if line.strip()]
projects = json.loads((STORE / "manifest.json").read_text())["repos"]
missing = sorted({q["repo"] for q in rows} - projects.keys())
if missing:
    sys.exit(f"search: the store has no project {', '.join(missing)}")
rows.sort(key=lambda q: q["repo"])
with open("/work/search.jsonl", "a") as out:
    for i, q in enumerate(rows, 1):
        if i == 1 or q["repo"] != rows[i - 2]["repo"]:
            scope(q["repo"], projects[q["repo"]]["notes"])
        for form, hits in (("query", recall(q["id"], q["query"])), ("prompt", digest(q["id"], q["prompt"]))):
            out.write(json.dumps({"id": q["id"], "form": form, "results": hits}, ensure_ascii=False) + "\n")
            out.flush()
        print(f"search: {i}/{len(rows)} {q['id']}", file=sys.stderr)
