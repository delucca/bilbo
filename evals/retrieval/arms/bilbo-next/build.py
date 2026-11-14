"""Build bilbo's stores from the sessions of /corpus with the bilbo binary of the image: `bilbo remember` for every
repository, then `bilbo reindex`, in one store of every repository (scope all) or one store a repository (scope repo).
Every `bilbo` run has its own HOME, store, state and cache folders under /work/store/stores/<name>, the same paths the
check and the search use again, since the catalog and the state folder record them. /work/store/entries.jsonl maps each
session file to its entry id; manifest.json is written last."""

import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

CORPUS = Path("/corpus")
WORK = Path("/work")
STORE = WORK / "store"
STAGED = Path("/tmp/staged")
BILBO = "/usr/local/bin/bilbo"
SESSION_ID = re.compile(r"^.+--([0-9a-f-]{36})\.jsonl$")
REMEMBER = re.compile(r"^new (\d+), appended (\d+), new generation (\d+), unchanged (\d+), refused (\d+)$")
SEARCHED = re.compile(r"^bilbo: searched ([\d,]+) entr(?:y|ies)$", re.M)
REINDEX = re.compile(r"^entries (\d+), chunks (\d+), unread (\d+), failed (\d+)\nindex bytes (\d+), milliseconds (\d+)$")


def log(msg):
    print(f"bilbo-next: {msg}", file=sys.stderr, flush=True)


def stamp(entry):
    return datetime.fromisoformat(entry["timestamp"]) if entry.get("timestamp") else None


def sessions():
    """Every session as (repo, path), in the order they started across all repos, as the bm25 arm orders them. A
    transcript with no timestamp (Cursor's) comes last, by path."""
    found, other = [], []
    for path in sorted(CORPUS.glob("*/*.jsonl")):
        start = next((t for t in map(stamp, (json.loads(line) for line in path.open() if line.strip())) if t), None)
        (found if start else other).append((start, path.parent.name, path))
    found.sort()
    limit = os.environ.get("BILBO_EVAL_SESSIONS")
    return [(repo, path) for _, repo, path in found + other][: int(limit) if limit else None]


def folders(name):
    return STORE / "stores" / name


def environment(name):
    d = folders(name)
    return {"HOME": f"{d}/home", "BILBO_HOME": f"{d}/root", "XDG_STATE_HOME": f"{d}/state",
            "XDG_CACHE_HOME": f"{d}/cache", "BILBO_CONFIG": f"{d}/config.toml", "PATH": "/usr/bin:/bin",
            "NO_COLOR": "1"}


def bilbo(name, *args):
    done = subprocess.run([BILBO, *args], env=environment(name), capture_output=True, text=True, check=False)
    return done.returncode, done.stdout, done.stderr


def sure(what, code, out, err):
    if code:
        sys.exit(f"bilbo-next: {what} exited {code}, not 0\nstdout: {out}\nstderr: {err}")


def prepare(name):
    d = folders(name)
    for sub in ("root", "state", "cache", "home"):
        (d / sub).mkdir(parents=True, exist_ok=True)
    (d / "config.toml").write_text("follow = []\n")


def remember(name, repo, paths, limited):
    target = f"/corpus/{repo}"
    if limited:
        staged = STAGED / repo
        shutil.rmtree(staged, ignore_errors=True)
        staged.mkdir(parents=True)
        for p in paths:
            shutil.copy2(p, staged / p.name)
        target = str(staged)
    code, out, err = bilbo(name, "remember", target, "--kind", "session")
    sure(f"remember {repo}", code, out, err)
    first = out.splitlines()[0] if out else ""
    m = REMEMBER.match(first)
    if not m:
        sys.exit(f"bilbo-next: remember {repo}: unexpected output {out!r}")
    new, appended, generation, unchanged, refused = map(int, m.groups())
    if refused or appended or generation or new + unchanged != len(paths):
        sys.exit(f"bilbo-next: remember {repo}: {len(paths)} files but {first!r}; stderr {err!r}")
    return new


def entry_of(name, repo, path):
    session = SESSION_ID.match(path.name)
    if not session:
        sys.exit(f"bilbo-next: {repo}/{path.name} has no session id in its name")
    code, out, err = bilbo(name, "read", session.group(1), "--budget", "256")
    if code == 2:
        sys.exit(f"bilbo-next: {repo}/{path.name}: the session id names several entries: {err.strip()}")
    sure(f"read {session.group(1)}", code, out, err)
    head = out.split("\n", 1)[0]
    if not re.fullmatch(r"bilbo:[0-9A-Z]{26}", head):
        sys.exit(f"bilbo-next: {repo}/{path.name}: read printed {head!r}, not an entry id")
    return head.removeprefix("bilbo:")


def main():
    plan = sessions()
    limited = bool(os.environ.get("BILBO_EVAL_SESSIONS"))
    scope = os.environ["BILBO_EVAL_SCOPE"]
    if scope not in ("all", "repo"):
        sys.exit(f"bilbo-next: BILBO_EVAL_SCOPE is {scope!r}, not all or repo")
    by_repo = {}
    for repo, path in plan:
        by_repo.setdefault(repo, []).append(path)
    for paths in by_repo.values():
        paths.sort()
    groups = {"all": sorted(by_repo)} if scope == "all" else {repo: [repo] for repo in sorted(by_repo)}
    log(f"{len(plan)} sessions of {len(by_repo)} repositories in {len(groups)} store(s)")

    STORE.mkdir(exist_ok=True)
    started = time.monotonic()
    entries, stores = [], {}
    for name, repos in groups.items():
        prepare(name)
        for repo in repos:
            new = remember(name, repo, by_repo[repo], limited)
            log(f"{repo}: {len(by_repo[repo])} sessions, {new} new")
        for repo in repos:
            for path in by_repo[repo]:
                entries.append({"store": name, "id": entry_of(name, repo, path), "repo": repo, "session": path.name})
        code, out, err = bilbo(name, "reindex")
        sure("reindex", code, out, err)
        m = REINDEX.match(out.strip())
        if not m:
            sys.exit(f"bilbo-next: reindex {name}: unexpected output {out!r}")
        count, chunks, unread, failed, size, _ = map(int, m.groups())
        want = sum(len(by_repo[r]) for r in repos)
        if unread or failed or count != want:
            sys.exit(f"bilbo-next: reindex {name}: {want} sessions but {out.strip()!r}; stderr {err!r}")
        code, out, err = bilbo(name, "recall", "--budget", "4000", "--", "user", "assistant")
        sure(f"recall in {name}", code, out, err)
        (folders(name) / "state" / "bilbo" / "queries.log").unlink(missing_ok=True)
        found = SEARCHED.search(err)
        indexed = int(found.group(1).replace(",", "")) if found else 0
        if not 0 < indexed <= count:
            sys.exit(f"bilbo-next: recall in {name} searched {indexed} of {count} entries; stderr {err!r}")
        stores[name] = {"repos": repos, "entries": count, "indexed": indexed, "chunks": chunks, "index_bytes": size}
        log(f"{name}: {count} entries, {chunks} chunks, index {size} bytes")
    seen = {(e["store"], e["id"]) for e in entries}
    if len(seen) != len(entries):
        sys.exit("bilbo-next: two sessions share an entry id in one store")

    tmp = STORE / "entries.jsonl.part"
    tmp.write_text("".join(json.dumps(e) + "\n" for e in entries))
    tmp.rename(STORE / "entries.jsonl")
    inputs = (WORK / "key.txt").read_text().splitlines()
    version = subprocess.run([BILBO, "--version"], capture_output=True, text=True, check=True).stdout.strip()
    manifest = {
        "arm": "bilbo-next",
        "key": inputs[0],
        "inputs": inputs[1:],
        "scope": scope,
        "remembered": str(STAGED if limited else CORPUS),
        "bilbo": version,
        "binary_sha256": hashlib.sha256(Path(BILBO).read_bytes()).hexdigest(),
        "sessions": len(plan),
        "entries": len(entries),
        "stores": stores,
        "ingest_seconds": round(time.monotonic() - started),
        "finished": datetime.now(timezone.utc).isoformat(timespec="seconds"),
    }
    (STORE / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    log(f"done: {STORE}")


if __name__ == "__main__":
    main()
