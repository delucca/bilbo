"""Give each question in /work/questions.jsonl the auto memory of its repository, from the store unpacked in
/work/store, as an agent reads it: for every project key of the repository, the main checkout's first, MEMORY.md as
Claude Code loads it; then the topic files, ranked for the form's text by BM25 over what the agent sees before it opens
one (the file's name and the lines of the loaded MEMORY.md that link it). Each file opens with its full memory path.
Appends one row per question and form to /work/search.jsonl. build-store.sh --search runs it in the build service."""

import json
import math
import re
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, "/app")
from build import checkout_roots, key_of  # noqa: E402

STORE = Path("/work/store")
LINES, CHARS = 200, 25000
K1, B = 1.2, 0.75
manifest = json.loads((STORE / "manifest.json").read_text())
roots = checkout_roots()


def shown(key, name):
    return f"~/.claude/projects/{key}/memory/{name}"


def words(text):
    return re.findall(r"[a-z0-9]+", text.lower())


def loaded_index(key):
    path = STORE / "memory" / key / "MEMORY.md"
    if not path.is_file():
        return ""
    return "\n".join(path.read_text(errors="replace").split("\n")[:LINES])[:CHARS]


def topics(key, index):
    """The key's topic files as (name, what the agent sees of it), the ones MEMORY.md links first, then by name."""
    directory = STORE / "memory" / key
    files = {f.relative_to(directory).as_posix(): f for f in sorted(directory.rglob("*"))
             if f.is_file() and f != directory / "MEMORY.md"}
    linked = [n for n in re.findall(r"\]\(([^)\s]+)\)", index) if n in files]
    out = []
    for name in dict.fromkeys(linked + sorted(files)):
        entry = " ".join(line for line in index.splitlines() if f"]({name})" in line)
        out.append((key, name, files[name], f"{name} {entry}"))
    return out


def ranked(candidates, text):
    """Okapi BM25 of the text against each candidate's visible words; a stable sort keeps the order above for ties."""
    docs = [Counter(words(c[3])) for c in candidates]
    if not docs:
        return []
    avg = sum(sum(d.values()) for d in docs) / len(docs) or 1
    df = Counter(w for d in docs for w in d)
    n = len(docs)

    def score(d):
        size = sum(d.values())
        return sum(math.log(1 + (n - df[w] + 0.5) / (df[w] + 0.5)) * d[w] * (K1 + 1) /
                   (d[w] + K1 * (1 - B + B * size / avg)) for w in set(words(text)) if w in d)

    scores = [score(d) for d in docs]
    return [c for _, c in sorted(zip(scores, candidates), key=lambda p: -p[0])]


def keys_of(repo):
    if repo not in manifest["projects"]:
        sys.exit(f"search: the store has no project for {repo}")
    keys = sorted(manifest["projects"][repo])
    main = key_of(roots[repo], roots[repo]) if repo in roots else None
    if main in keys:
        keys.remove(main)
        keys.insert(0, main)
    return keys


def memory(repo, text):
    out, candidates = [], []
    for key in keys_of(repo):
        index = loaded_index(key)
        if index.strip():
            out.append({"text": f"{shown(key, 'MEMORY.md')}:\n{index}", "ref": f"{key}/MEMORY.md"})
        candidates += topics(key, index)
    for key, name, path, _ in ranked(candidates, text):
        out.append({"text": f"{shown(key, name)}:\n{path.read_text(errors='replace')}", "ref": f"{key}/{name}"})
    return out


rows = [json.loads(line) for line in Path("/work/questions.jsonl").read_text().split("\n") if line.strip()]
with open("/work/search.jsonl", "a") as out:
    for i, q in enumerate(rows, 1):
        for form in ("query", "prompt"):
            results = memory(q["repo"], q[form])
            out.write(json.dumps({"id": q["id"], "form": form, "results": results}, ensure_ascii=False) + "\n")
            out.flush()
        print(f"search: {i}/{len(rows)} {q['id']} ({len(results)} files)", file=sys.stderr)
