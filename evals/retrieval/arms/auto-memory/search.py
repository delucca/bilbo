"""Give each question in /work/questions.jsonl the auto memory of its repository, from the store unpacked in
/work/store: for every project key of the repository, the main checkout's first, MEMORY.md as Claude Code loads it,
then every topic file, those MEMORY.md links first. Auto memory takes no query, so both forms get the same rows.
Appends one row per question and form to /work/search.jsonl. build-store.sh --search runs it in the build service."""

import json
import re
import sys
from pathlib import Path

sys.path.insert(0, "/app")
from build import checkout_roots, key_of  # noqa: E402

STORE = Path("/work/store")
LINES, CHARS = 200, 25000
manifest = json.loads((STORE / "manifest.json").read_text())
roots = checkout_roots()


def loaded_index(directory):
    path = directory / "MEMORY.md"
    if not path.is_file():
        return []
    text = "\n".join(path.read_text(errors="replace").split("\n")[:LINES])[:CHARS]
    return [{"text": f"{path.name}:\n{text}", "ref": f"{directory.name}/{path.name}"}] if text.strip() else []


def topics(directory):
    index = directory / "MEMORY.md"
    linked = re.findall(r"\]\(([^)\s]+)\)", index.read_text(errors="replace")) if index.is_file() else []
    files = {f.relative_to(directory).as_posix(): f for f in sorted(directory.rglob("*")) if f.is_file() and f != index}
    order = list(dict.fromkeys([n for n in linked if n in files] + sorted(files)))
    return [{"text": f"{name}:\n{files[name].read_text(errors='replace')}", "ref": f"{directory.name}/{name}"} for name in order]


def memory(repo):
    if repo not in manifest["projects"]:
        sys.exit(f"search: the store has no project for {repo}")
    keys = sorted(manifest["projects"][repo])
    main = key_of(roots[repo], roots[repo]) if repo in roots else None
    if main in keys:
        keys.remove(main)
        keys.insert(0, main)
    out = []
    for key in keys:
        directory = STORE / "memory" / key
        out += loaded_index(directory) + topics(directory)
    return out


rows = [json.loads(line) for line in Path("/work/questions.jsonl").read_text().splitlines() if line.strip()]
with open("/work/search.jsonl", "a") as out:
    for i, q in enumerate(rows, 1):
        results = memory(q["repo"])
        for form in ("query", "prompt"):
            out.write(json.dumps({"id": q["id"], "form": form, "results": results}, ensure_ascii=False) + "\n")
            out.flush()
        print(f"search: {i}/{len(rows)} {q['id']} ({len(results)} files)", file=sys.stderr)
