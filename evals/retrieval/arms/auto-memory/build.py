"""Build the auto-memory store: the memory files the developers' own Claude Code wrote during the recorded sessions,
extracted from the transcripts. The main model's successful Write, Edit and MultiEdit calls on a file under
`.claude/projects/<key>/memory/`, and the Bash writes of three known shapes, are applied in recorded order to that
project's memory directory, under the name the developer's machine used. Nothing runs Claude Code and nothing calls a
model; a recorded command is parsed, never run. Then save the directories with a manifest in /work/store."""

import ast
import hashlib
import json
import os
import platform
import re
import shutil
import sys
from collections import Counter
from datetime import datetime
from pathlib import Path

CORPUS = Path("/corpus")
WORK = Path("/work")
STORE = WORK / "store"
MEMORY = STORE / "memory"
NOTES = {
    "scope": "only what the developers' Claude Code wrote through the main model's tools during the recorded sessions: background memory extraction, auto dream and the recall selector are off on the accounts these were recorded on, and the corpus holds no trace of them",
    "empty_start": "each project's memory starts empty, as for every arm; an Edit on a file written before the recordings finds no text to change and is skipped, counted as edit_file_unknown",
    "bash": "Bash writes of three known shapes are applied; a recorded command is never run, and a write of another shape is skipped as bash_unparsed",
    "failed": "a tool call whose result is an error is not applied and is counted as recorded_failed",
    "order": "per project, by the time of the tool result, then the session's start and the entry's line",
    "cursor": "the two Cursor transcripts are left out: Claude Code never saw them",
    "run_step": "MEMORY.md is loaded up to its first 200 lines or 25,000 characters, and topic files are read on demand",
}


def log(msg):
    print(f"auto-memory: {msg}", file=sys.stderr, flush=True)


def stamp(entry):
    return datetime.fromisoformat(entry["timestamp"]) if entry.get("timestamp") else None


def lines_of(path):
    return [line for line in path.read_text().split("\n") if line.strip()]


def sessions():
    """Every Claude Code session as (start, repo, path), in the order they started across all repos. A transcript with
    no timestamp is not Claude Code's and is left out; Claude Code never saw it."""
    found, other = [], []
    for path in sorted(CORPUS.glob("*/*.jsonl")):
        start = next((t for t in map(stamp, map(json.loads, lines_of(path))) if t), None)
        (found if start else other).append((start, path.parent.name, path))
    found.sort()
    limit = os.environ.get("BILBO_EVAL_SESSIONS")
    return found[: int(limit) if limit else None], [f"{repo}/{path.name}" for _, repo, path in other]


def checkout_roots():
    """Each repository's main checkout: the directory most of its sessions start in."""
    starts = {}
    for path in sorted(CORPUS.glob("*/*.jsonl")):
        cwd = next((e["cwd"] for e in map(json.loads, lines_of(path)) if e.get("cwd")), None)
        if cwd:
            starts.setdefault(path.parent.name, Counter())[cwd] += 1
    return {repo: cwds.most_common(1)[0][0] for repo, cwds in starts.items()}


def key_of(root, cwd):
    """The project key Claude Code derives: the main checkout's path when the session started in it or under it
    (worktrees and subdirectories included), else the session's own first directory."""
    where = root if cwd == root or cwd.startswith(root.rstrip("/") + "/") else cwd
    return re.sub("[^a-zA-Z0-9]", "-", where)


def parse_lines(lines, keep_bad=True):
    """JSON objects from JSONL lines; a line that does not parse is {} (keep_bad) or left out."""
    out = []
    for line in lines:
        try:
            out.append(json.loads(line))
        except ValueError:
            if keep_bad:
                out.append({})
    return out


# The memory operations the developer's Claude Code recorded. A memory file is one under .claude/projects/<key>/memory
# of the developer's home; the corpus redacts one iptvnator path, which is the MEMORY.md of the session's key.
MEMORY_PATH = re.compile(r"(?P<root>[^\s\"'=<>]*?)/\.claude/projects/(?P<key>[^/\s\"']+)/memory(?P<rest>/[^\s\"'\;&|<>]*)?")
HOME_ROOT = re.compile(r"(?:/Users|/home)/[^/\s\"']+")
REDACTED = re.compile(r"^(/Users|/home)/[^/]+/\.REDACTED\.md$")


def bash_op(command):
    """The write a recorded Bash command made, when it is one of the three shapes known, else None for a read or
    ("unknown",) for a write of another shape. The command is parsed, never run."""
    m = re.match(r"\s*echo\s+'((?:[^']|'\\'')*)'\s*>>\s*(\S+)\s*$", command)
    if m:
        return ("unknown",) if "'\\''" in m.group(1) else ("append", m.group(2).split("/memory/")[-1], m.group(1) + "\n")
    m = re.match(r"\s*cd \S+/memory && echo \"(.*)\" >> (\S+) && tail", command, re.S)
    if m:
        return ("unknown",) if re.search(r"[$`\\]", m.group(1)) else ("append", m.group(2), m.group(1) + "\n")
    m = re.match(r"\s*cd \S+/memory && python3 - <<'EOF'\n(.*)\nEOF\n", command, re.S)
    if m:
        name = re.search(r"p='([^']+)'", m.group(1))
        try:
            pairs = []
            for call in ast.walk(ast.parse(m.group(1))):
                if isinstance(call, ast.Call) and getattr(call.func, "attr", "") == "replace":
                    pairs.append(tuple(ast.literal_eval(a) for a in call.args[:2]))
        except (SyntaxError, ValueError):
            return ("unknown",)
        return ("replace", name.group(1), pairs) if name and pairs else ("unknown",)
    m = re.match(r"\s*mkdir -p (\S+) && cat > (\S+) << 'EOF'\n(.*)\nEOF\s*$", command, re.S)
    if m:
        return ("write", m.group(2).split("/memory/")[-1], m.group(3) + "\n")
    if re.match(r"\s*(ls|cat)\b", command) or re.match(r"\s*(cd \S+/memory && cat|mkdir -p \S+ && ls) ", command):
        return None
    return ("unknown",) if re.search(r">>|\bcat >|open\(|\btee\b|\bsed -i|\bmv |\brm |\bcp ", command) else None


def recorded_ops(entries, session_key, start):
    """Every memory operation in a session, with the entry its result is in (where it took effect, and its time), the
    Claude Code version and model recorded on the entry that made the call, and why it cannot be applied, if so."""
    times, when = [], start
    for e in entries:
        when = stamp(e) or when
        times.append(when)
    uses, results, seen = {}, {}, set()
    for line, e in enumerate(entries):
        if e.get("uuid"):
            if e["uuid"] in seen:
                continue
            seen.add(e["uuid"])
        content = (e.get("message") or {}).get("content") if isinstance(e.get("message"), dict) else None
        for b in content if isinstance(content, list) else ():
            if isinstance(b, dict) and b.get("id") and b.get("type") == "tool_use":
                uses.setdefault(b["id"], (line, b, e))
            elif isinstance(b, dict) and b.get("tool_use_id") and b.get("type") == "tool_result":
                results.setdefault(b["tool_use_id"], (line, b))
    ops = []
    for tid, (use_line, b, made) in uses.items():
        name, inp = b.get("name"), b.get("input") or {}
        path, match, key, rest = inp.get("file_path"), None, None, "MEMORY.md"
        if name in ("Write", "Edit", "MultiEdit") and path:
            match = MEMORY_PATH.search(path)
            if not match and REDACTED.match(path):
                key = session_key
        elif name == "Bash":
            match = MEMORY_PATH.search(inp.get("command") or "")
        if not match and not key:
            continue
        if match:
            key, rest = match.group("key"), (match.group("rest") or "/").lstrip("/")
        op = {"tool": name, "key": key, "file": rest, "line": use_line, "skip": None, "failed": False,
              "version": made.get("version"), "model": (made.get("message") or {}).get("model")}
        if match and match.group("root") and not HOME_ROOT.fullmatch(match.group("root")):
            op["skip"] = "wrong_root"
        done = results.get(tid)
        if done:
            op["line"], op["failed"] = done[0], bool(done[1].get("is_error"))
        elif not op["skip"]:
            op["skip"] = "no_result"
        if name == "Write":
            op |= {"kind": "write", "content": inp.get("content") or ""}
        elif name in ("Edit", "MultiEdit"):
            op |= {"kind": "edit", "edits": [inp] if name == "Edit" else list(inp.get("edits") or [])}
        else:
            shape = bash_op(inp.get("command") or "")
            if shape is None:
                continue
            if shape[0] == "unknown":
                op |= {"kind": "bash", "skip": op["skip"] or "bash_unparsed"}
            else:
                op |= {"kind": "bash", "shape": shape[0], "file": shape[1], "payload": shape[2]}
        ops.append(op | {"time": times[op["line"]]})
    return sorted(ops, key=lambda o: o["line"])


def memory_dir(key):
    return MEMORY / key


def apply_op(op):
    """Apply one recorded memory operation to its project's memory directory, as Claude Code's tools would have: an Edit
    whose text is absent fails. Returns "applied", "recorded_failed" or the reason it was skipped."""
    if op["failed"]:
        return "recorded_failed"
    if op["skip"]:
        return op["skip"]
    path = Path(op["file"])
    if ".." in path.parts or not op["file"] or path.is_absolute():
        return f"{op['kind']}_bad_path"
    target = memory_dir(op["key"]) / op["file"]
    current = target.read_text() if target.is_file() else None
    kind, text = op["kind"], None
    if kind == "write":
        text = op["content"]
    elif kind == "edit":
        text = current
        for edit in op["edits"]:
            old, new = edit.get("old_string") or "", edit.get("new_string") or ""
            if text is None:
                return "edit_file_unknown"
            n = text.count(old)
            if n == 0 or (n > 1 and not edit.get("replace_all")):
                return "edit_target_missing" if n == 0 else "edit_ambiguous"
            text = text.replace(old, new) if edit.get("replace_all") else text.replace(old, new, 1)
    elif op["shape"] == "write":
        text = op["payload"]
    elif current is None:
        return "bash_unknown_file"
    elif op["shape"] == "append":
        text = current + ("" if current.endswith("\n") or current == "" else "\n") + op["payload"]
    else:
        text = current
        for old, new in op["payload"]:
            if old not in text:
                return "bash_replace_target_missing"
            text = text.replace(old, new)
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(text)
    return "applied"


def digests(root):
    return {p.relative_to(root).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in sorted(root.rglob("*")) if p.is_file()} if root.is_dir() else {}


def stats(directory):
    """MEMORY.md against its load limits, its links, and the topic files' frontmatter."""
    index = directory / "MEMORY.md"
    text = index.read_text(errors="replace") if index.exists() else ""
    links = [m for m in re.findall(r"\]\(([^)\s]+\.md)\)", text) if "://" not in m]
    topics = [f for f in directory.rglob("*.md") if f.name != "MEMORY.md"] if directory.is_dir() else []
    framed = 0
    for f in topics:
        head = f.read_text(errors="replace")
        meta = head.split("---", 2)[1] if head.startswith("---") and head.count("---") >= 2 else ""
        framed += all(re.search(rf"^\s*{k}:", meta, re.M) for k in ("name", "description", "type"))
    return {"memory_files": sum(1 for f in directory.rglob("*") if f.is_file()) if directory.is_dir() else 0,
            "bytes": sum(f.stat().st_size for f in directory.rglob("*") if f.is_file()) if directory.is_dir() else 0,
            "memory_md": {"lines": len(text.splitlines()), "chars": len(text), "over_limit": len(text.splitlines()) > 200 or len(text) > 25000,
                          "dangling_links": sum(not (directory / m).is_file() for m in links)},
            "topic_files": {"with_frontmatter": framed, "without": len(topics) - framed}}


def recorded(plan, roots):
    """Every memory operation of every session, the project keys by repository and the sessions that made any, in
    the order they took effect: per project by the result's time, then the session's start and the entry's line."""
    ops, keys, made = [], {}, set()
    for start, repo, path in plan:
        entries = parse_lines(lines_of(path))
        sid = next(e["sessionId"] for e in entries if e.get("sessionId"))
        cwd = next((e["cwd"] for e in entries if e.get("cwd")), "/")
        key = key_of(roots[repo], cwd)
        keys.setdefault(repo, set()).add(key)
        for op in recorded_ops(entries, key, start):
            ops.append(op | {"repo": repo, "session": sid, "start": start})
            made.add(sid)
    ops.sort(key=lambda o: (o["key"], o["time"], o["start"], o["line"]))
    return ops, keys, len(made)


def packed_keys(keys):
    """Every project key the store holds by repository: the sessions' own, and under "(other)" any a recorded write named."""
    extra = {d.name for d in MEMORY.iterdir()} - {k for ks in keys.values() for k in ks} if MEMORY.is_dir() else set()
    return keys | ({"(other)": extra} if extra else {})


def manifest(skipped_files, ops, outcomes, keys, sessions_made):
    inputs = (WORK / "key.txt").read_text().splitlines()
    keys = packed_keys(keys)
    applied = [o for o, r in zip(ops, outcomes) if r == "applied"]
    projects = {}
    for repo, ks in sorted(keys.items()):
        projects[repo] = {}
        for key in sorted(ks):
            mine = [r for o, r in zip(ops, outcomes) if o["key"] == key]
            count = Counter(mine)
            projects[repo][key] = {
                "operations": len(mine), "applied": {k: sum(1 for o, r in zip(ops, outcomes) if o["key"] == key and r == "applied" and o["kind"] == k)
                                                     for k in ("write", "edit", "bash")},
                "recorded_failed": count.pop("recorded_failed", 0), "skipped": dict(sorted((k, v) for k, v in count.items() if k != "applied"))
            } | stats(memory_dir(key))
    return {
        "arm": "auto-memory",
        "key": inputs[0],
        "inputs": inputs[1:],
        "device": os.environ["BILBO_EVAL_DEVICE"],
        "versions": {"python": platform.python_version(), "arch": platform.machine()},
        "notes": NOTES,
        "projects": projects,
        "sessions": {"with_memory_operations": sessions_made, "skipped": skipped_files},
        "operations": {"recorded": len(ops), "applied": len(applied), "recorded_failed": outcomes.count("recorded_failed"),
                       "skipped": dict(sorted(Counter(r for r in outcomes if r not in ("applied", "recorded_failed")).items()))},
        "writers": {"models": dict(sorted(Counter(str(o["model"]) for o in applied).items())),
                    "claude_code_versions": dict(sorted(Counter(str(o["version"]) for o in applied).items()))},
        "provenance": [{"project": o["key"], "session": o["session"], "time": o["time"].isoformat(), "tool": o["tool"], "file": o["file"],
                        "outcome": r, "version": o["version"], "model": o["model"]} for o, r in zip(ops, outcomes)],
        "memory_files": {key: digests(memory_dir(key)) for ks in keys.values() for key in sorted(ks)},
    }


def main():
    plan, skipped_files = sessions()
    ops, keys, sessions_made = recorded(plan, checkout_roots())
    shutil.rmtree(STORE, ignore_errors=True)
    if bad := sorted({o["key"] for o in ops if o["key"] in ("", ".", "..")}):
        sys.exit(f"auto-memory: a recorded write names the project key {bad}, which is not a directory name")
    for key in sorted({k for ks in keys.values() for k in ks} | {o["key"] for o in ops}):
        memory_dir(key).mkdir(parents=True, exist_ok=True)
    outcomes = [apply_op(op) for op in ops]
    (STORE / "manifest.json").write_text(json.dumps(manifest(skipped_files, ops, outcomes, keys, sessions_made), indent=2) + "\n")
    log(f"done: {STORE}, {outcomes.count('applied')} of {len(ops)} operations applied")


if __name__ == "__main__":
    main()
