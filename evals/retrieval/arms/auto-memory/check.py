"""Check an auto-memory store unpacked in /work/store. As packed: the manifest opens as the other arms' do, every
project's memory directory holds exactly the files and sha256 the manifest lists, MEMORY.md's size and links and the
topic files' frontmatter agree with the manifest's own numbers, the per-operation provenance agrees with its counts,
and no file carries a credential. It needs no LLM and no service. build-store.sh --check runs it in the build service."""

import hashlib
import json
import re
import sys
from collections import Counter
from pathlib import Path

STORE = Path("/work/store")
manifest = json.loads((STORE / "manifest.json").read_text())
failed = []


def check(name, ok, detail=""):
    print(f"{name}: {'ok' if ok else 'FAILED'}{' ' + str(detail) if detail else ''}")
    if not ok:
        failed.append(name)


def digests(root):
    return {p.relative_to(root).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in sorted(root.rglob("*")) if p.is_file()} if root.is_dir() else {}


def stats(directory):
    index = directory / "MEMORY.md"
    text = index.read_text(errors="replace") if index.exists() else ""
    links = [m for m in re.findall(r"\]\(([^)\s]+\.md)\)", text) if "://" not in m]
    topics = [f for f in directory.rglob("*.md") if f.name != "MEMORY.md"] if directory.is_dir() else []
    framed = 0
    for f in topics:
        head = f.read_text(errors="replace")
        meta = head.split("---", 2)[1] if head.startswith("---") and head.count("---") >= 2 else ""
        framed += all(re.search(rf"^\s*{k}:", meta, re.M) for k in ("name", "description", "type"))
    files = [f for f in directory.rglob("*") if f.is_file()] if directory.is_dir() else []
    return {"memory_files": len(files), "bytes": sum(f.stat().st_size for f in files),
            "memory_md": {"lines": len(text.splitlines()), "chars": len(text), "over_limit": len(text.splitlines()) > 200 or len(text) > 25000,
                          "dangling_links": sum(not (directory / m).is_file() for m in links)},
            "topic_files": {"with_frontmatter": framed, "without": len(topics) - framed}}


head = list(manifest)[:5]
check("manifest opens with arm, key, inputs, device, versions", head == ["arm", "key", "inputs", "device", "versions"], head)
check("manifest names this arm", manifest["arm"] == "auto-memory")
check("inputs name the sessions and no LLM", any(i.startswith("sessions ") for i in manifest["inputs"])
      and not any(i.startswith("llm") for i in manifest["inputs"]))

keys = sorted(k for found in manifest["projects"].values() for k in found)
on_disk = sorted(p.name for p in (STORE / "memory").iterdir()) if (STORE / "memory").is_dir() else []
check("memory directories are the manifest's projects", on_disk == keys and sorted(manifest["memory_files"]) == keys, on_disk)
for repo, found in manifest["projects"].items():
    for key, listed in found.items():
        directory = STORE / "memory" / key
        seen = digests(directory)
        check(f"{repo} {key}: files and sha256 match the manifest", seen == manifest["memory_files"].get(key), f"{len(seen)} files")
        counted = stats(directory)
        check(f"{repo} {key}: MEMORY.md, links and frontmatter match the manifest",
              all(listed[k] == v for k, v in counted.items()), f"MEMORY.md {counted['memory_md']}, topics {counted['topic_files']}")

log = manifest["provenance"]
ops = manifest["operations"]
outcomes = Counter(o["outcome"] for o in log)
check("operations: provenance has one entry per recorded operation", len(log) == ops["recorded"], len(log))
check("operations: applied, failed and skipped match the provenance", (outcomes["applied"], outcomes["recorded_failed"]) == (ops["applied"], ops["recorded_failed"])
      and ops["skipped"] == {k: v for k, v in sorted(outcomes.items()) if k not in ("applied", "recorded_failed")})
check("projects: operations, applied, failed and skipped add up to the provenance's, per project", all(
    (p["operations"], sum(p["applied"].values()), p["recorded_failed"], p["skipped"])
    == (sum(o["project"] == key for o in log), sum(o["project"] == key and o["outcome"] == "applied" for o in log),
        sum(o["project"] == key and o["outcome"] == "recorded_failed" for o in log),
        dict(sorted(Counter(o["outcome"] for o in log if o["project"] == key and o["outcome"] not in ("applied", "recorded_failed")).items())))
    for found in manifest["projects"].values() for key, p in found.items()))
check("every applied operation's file is in the store", all(
    o["file"] in manifest["memory_files"].get(o["project"], {}) for o in log if o["outcome"] == "applied"))
applied = [o for o in log if o["outcome"] == "applied"]
check("writers: models and versions match the provenance", manifest["writers"] == {
    "models": dict(sorted(Counter(str(o["model"]) for o in applied).items())),
    "claude_code_versions": dict(sorted(Counter(str(o["version"]) for o in applied).items()))})
check("no file in the store carries a credential", not [f.name for f in STORE.rglob("*") if f.is_file()
                                                       and re.search(rb"sk-ant-[A-Za-z0-9_-]{20,}|oat01-[A-Za-z0-9_-]{20,}", f.read_bytes())])

print("FAILED" if failed else "OK")
sys.exit(1 if failed else 0)
