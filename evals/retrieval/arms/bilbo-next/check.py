"""Check the bilbo stores unpacked in /work/store: the manifest's sessions are the corpus's (as many as the limit says),
entries.jsonl has one row a session with distinct ids in a store, and every store answers `bilbo recall`: exit 0, its
stderr counts the entries the build's own recall counted (`indexed` in the manifest), every header is followed by its
lines, and `bilbo read` takes the first hit's ref; a query of stopwords only exits 1 with `bilbo: nothing matched`. build-store.sh --check runs it in the build
service."""

import json
import re
import subprocess
import sys
from datetime import datetime
from pathlib import Path

STORE = Path("/work/store")
CORPUS = Path("/corpus")
BILBO = "/usr/local/bin/bilbo"
HEADER = re.compile(r"^=== (bilbo:[0-9A-Z]{26}#r\d+(?:-\d+)?)\t([a-z]+)\t(\d{4}-\d{2}-\d{2})\t(\d+) lines?(?:\t[^\t\n]+)*$")
SEARCHED = re.compile(r"^bilbo: searched ([\d,]+) entr(?:y|ies)$")


def stamp(path):
    for line in path.open(encoding="utf-8"):
        entry = json.loads(line) if line.strip() else {}
        if entry.get("timestamp"):
            return datetime.fromisoformat(entry["timestamp"])
    return None


def chosen(manifest):
    """The sessions the build chose: the order of the bm25 arm, cut at the limit in the manifest's inputs."""
    found, other = [], []
    for path in sorted(CORPUS.glob("*/*.jsonl")):
        start = stamp(path)
        (found if start else other).append((start, path.parent.name, path.name))
    found.sort()
    limit = next(i.split()[1] for i in manifest["inputs"] if i.startswith("sessions "))
    return [(repo, name) for _, repo, name in found + other][: None if limit == "all" else int(limit)]


def environment(name, store=STORE):
    d = store / "stores" / name
    return {"HOME": f"{d}/home", "BILBO_HOME": f"{d}/root", "XDG_STATE_HOME": f"{d}/state",
            "XDG_CACHE_HOME": f"{d}/cache", "BILBO_CONFIG": f"{d}/config.toml", "PATH": "/usr/bin:/bin", "NO_COLOR": "1"}


def run(name, *args, binary=BILBO, store=STORE):
    done = subprocess.run([binary, *args], env=environment(name, store), capture_output=True, check=False)
    return done.returncode, done.stdout.decode(), done.stderr.decode()


def headers(stdout):
    lines = stdout.split("\n")
    if lines.pop() != "":
        return None
    found, i = [], 0
    while i < len(lines):
        m = HEADER.match(lines[i])
        if not m or i + int(m.group(4)) >= len(lines):
            return None
        found.append(m.group(1))
        i += 1 + int(m.group(4))
    return found


def main(binary=BILBO, store=STORE):
    manifest = json.loads((store / "manifest.json").read_text())
    rows = [json.loads(line) for line in (store / "entries.jsonl").open(encoding="utf-8")]
    problems = []

    def bad(msg):
        problems.append(msg)
        print("check:", msg)

    want = chosen(manifest)
    got = [(r["repo"], r["session"]) for r in rows]
    print("sessions", manifest["sessions"], "of", len(want), "| entries", len(rows), "| entries as the manifest counts",
          manifest["entries"])
    if manifest["sessions"] != len(want) or manifest["entries"] != len(rows):
        bad("the manifest's sessions or entries differ from the corpus and the limit")
    if sorted(got) != sorted(want) or len(set(got)) != len(got):
        bad("entries.jsonl is not one row for each chosen session")
    if manifest["scope"] not in ("all", "repo"):
        bad(f"scope {manifest['scope']!r}")
    by_store = {}
    for r in rows:
        by_store.setdefault(r["store"], []).append(r)
    if sorted(by_store) != sorted(manifest["stores"]):
        bad("the stores of entries.jsonl are not the manifest's")
    if manifest["scope"] == "repo" and any({r["repo"] for r in rs} != {name} for name, rs in by_store.items()):
        bad("a store of scope repo holds another repository")
    for name, rs in by_store.items():
        ids = [r["id"] for r in rs]
        if len(set(ids)) != len(ids) or not all(re.fullmatch(r"[0-9A-Z]{26}", i) for i in ids):
            bad(f"{name}: entry ids are not 26 distinct characters")
        if manifest["stores"][name]["entries"] != len(rs):
            bad(f"{name}: the manifest counts {manifest['stores'][name]['entries']} entries, entries.jsonl {len(rs)}")
        if not manifest["stores"][name]["chunks"]:
            bad(f"{name}: no chunk")


    for name in sorted(manifest["stores"]):
        code, out, err = run(name, "recall", "--budget", "4000", "--", "user", "assistant", binary=binary, store=store)
        count = [m for m in map(SEARCHED.match, err.splitlines()) if m]
        found = headers(out) if code == 0 else None
        ok = (code == 0 and err.splitlines() == [m.string for m in count] and len(count) == 1
              and int(count[0].group(1).replace(",", "")) == manifest["stores"][name]["indexed"] and bool(found))
        if not ok:
            bad(f"{name}: recall exited {code}; stderr {err.strip()[:300]!r}; hits {found and len(found)}")
            continue
        read_code, _, read_err = run(name, "read", found[0], "--budget", "256", binary=binary, store=store)
        if read_code != 0:
            bad(f"{name}: read {found[0]} exited {read_code}: {read_err.strip()[:300]}")
        code, out, err = run(name, "recall", "--", "the", "and", "of", "to", binary=binary, store=store)
        if code != 1 or out or not err.splitlines() or err.splitlines()[-1] != "bilbo: nothing matched":
            bad(f"{name}: a query of stopwords exited {code}; stderr {err.strip()[:300]!r}")
        kept = manifest["stores"][name]
        print(f"store {name}: {kept['entries']} entries, {kept['indexed']} searched, {kept['entries'] - kept['indexed']} "
              f"with no chunk, {len(found)} hits for the probe, first {found[0]}")

    print("FAILED" if problems else "OK")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
