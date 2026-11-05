"""Build the auto-memory store: the memory files the developers' own Claude Code wrote during the recorded sessions,
extracted from the transcripts. The main model's successful Write, Edit and MultiEdit calls on a file under
`.claude/projects/<key>/memory/`, and its Bash commands that touch one, are applied in recorded order to that project's
memory directory, under the name the developer's machine used. Whenever the transcript shows a whole memory file (a
Read, an Edit's original file, a `cat`), the file is set to what it showed. Nothing runs Claude Code and nothing calls
a model; a recorded command is replayed by replay.py, never run. Then save the directories with a manifest in
/work/store."""

import hashlib
import json
import os
import platform
import posixpath
import re
import shutil
import sys
from collections import Counter
from datetime import datetime
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import replay  # noqa: E402

CORPUS = Path("/corpus")
WORK = Path("/work")
STORE = WORK / "store"
MEMORY = STORE / "memory"
NOTES = {
    "scope": "what the developers' Claude Code wrote through the main model's tools during the recorded sessions; Claude Code's own memory saves (memory_saved entries) name files but not their content, so they are counted in unrecorded_saves and reach the store only through a later snapshot",
    "unrecorded_saves": "Claude Code's own record that it wrote memory files outside the main model's tool calls; the transcript names the files but not what was written, so they are counted and not applied",
    "empty_start": "each project's memory starts empty, as for every arm; an Edit on a file the store does not hold, and whose original the transcript does not record, is skipped as edit_file_unknown",
    "snapshots": "whenever the transcript shows a whole memory file (a Read of all of it, an Edit's recorded original file, or a lone cat between echoed lines), the file is set to what it showed before later operations apply: applied (new or corrected) or unchanged; a Read of part of a file or a cat beside other output is counted under unparsed.reads",
    "bash": "a Bash command is replayed statement by statement, never run: cat, echo, printf and tee with redirections and heredocs, rm, mv, cp, sed -i with one s command, and Python scripts evaluated over strings and file reads and writes only; a write it cannot follow is skipped as bash_unparsed and listed under unparsed, and an append to a file the store does not hold as bash_file_unknown",
    "failed": "a tool call whose result is an error is not applied and is counted as recorded_failed; for Bash, only the last list of the command (the one whose exit status was recorded) is held back",
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


def session_id(path):
    """The session a corpus file records: Claude Code names its transcript after it, and the corpus keeps the name."""
    return path.name.split("--", 1)[1].removesuffix(".jsonl")


def own_start(entries, sid):
    """The index of the session's first own record. A cloud session continued in a terminal (`claude --teleport`)
    begins with the cloud session's history under that session's id, which ran on the other machine."""
    return next((i for i, e in enumerate(entries) if e.get("sessionId") == sid), 0)


def sessions():
    """Every Claude Code session as (start, repo, path), in the order they started across all repos, each at its first
    own record (own_start). A transcript with no timestamp is not Claude Code's and is left out; Claude Code never saw it."""
    found, other = [], []
    for path in sorted(CORPUS.glob("*/*.jsonl")):
        entries = parse_lines(lines_of(path))
        start = next((t for t in map(stamp, entries[own_start(entries, session_id(path)):]) if t), None)
        (found if start else other).append((start, path.parent.name, path))
    found.sort()
    limit = os.environ.get("BILBO_EVAL_SESSIONS")
    return found[: int(limit) if limit else None], [f"{repo}/{path.name}" for _, repo, path in other]


def checkout_roots():
    """Each repository's main checkout: the directory most of its sessions start in."""
    starts = {}
    for path in sorted(CORPUS.glob("*/*.jsonl")):
        entries = parse_lines(lines_of(path))
        cwd = next((e["cwd"] for e in entries[own_start(entries, session_id(path)):] if e.get("cwd")), None)
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
# of the developer's home, a Windows one too (C:\Users\<name>, or /c/Users/<name> from Git Bash); the corpus redacts one iptvnator path, which is the MEMORY.md of the session's key.
MEMORY_PATH = re.compile(r"(?P<root>[^\s\"'=<>]*?)/\.claude/projects/(?P<key>[^/\s\"']+)/memory(?P<rest>/[^\s\"'\;&|<>]*)?")
HOME_ROOT = re.compile(r"(?:/Users|/home|[A-Za-z]:/Users|/[a-z]/Users)/[^/\s\"']+")
REDACTED = re.compile(r"^(/Users|/home)/[^/]+/\.REDACTED\.md$")
CWD_RESET = re.compile(r"\n?Shell cwd was reset to [^\n]*\s*$")


def bash_output(result):
    """What a recorded Bash call printed, or None when Claude Code cut it or kept it elsewhere."""
    if isinstance(result, dict):
        if result.get("persistedOutputPath") or not isinstance(result.get("stdout"), str):
            return None
        out = result["stdout"]
    elif isinstance(result, str):
        out = result
    else:
        return None
    out = CWD_RESET.sub("", out)
    return None if re.search(r"\[\d+ (characters|lines) truncated\]", out) else out


def recorded_ops(entries, session_key, start, first=0):
    """Every memory operation in a session, with the entry its result is in (where it took effect, and its time), the
    Claude Code version and model recorded on the entry that made the call, and why it cannot be applied, if so. A Read
    of a whole memory file, and an Edit's recorded original file, are snapshots of what the file held. `entries` begin
    at the file's entry `first`, and "line" is the file's."""
    times, when = [], start
    for e in entries:
        when = stamp(e) or when
        times.append(when)
    uses, results, seen = {}, {}, set()
    for line, e in enumerate(entries, first):
        if e.get("uuid"):
            if e["uuid"] in seen:
                continue
            seen.add(e["uuid"])
        content = (e.get("message") or {}).get("content") if isinstance(e.get("message"), dict) else None
        for b in content if isinstance(content, list) else ():
            if isinstance(b, dict) and b.get("id") and b.get("type") == "tool_use":
                uses.setdefault(b["id"], (line, b, e))
            elif isinstance(b, dict) and b.get("tool_use_id") and b.get("type") == "tool_result":
                results.setdefault(b["tool_use_id"], (line, b, e.get("toolUseResult")))
    ops, unusable = [], Counter()
    for tid, (use_line, b, made) in uses.items():
        name, inp = b.get("name"), b.get("input") or {}
        path, done = inp.get("file_path"), results.get(tid)
        op = {"tool": name, "key": session_key, "line": use_line, "skip": None, "failed": False,
              "version": made.get("version"), "model": (made.get("message") or {}).get("model")}
        if name in ("Write", "Edit", "MultiEdit", "Read") and path:
            path = path.replace("\\", "/")
            if REDACTED.match(path):
                path = f"{replay.HOME}/.claude/projects/{session_key}/memory/MEMORY.md"
            match = MEMORY_PATH.search(path)
            if not match or not (match.group("rest") or "").strip("/"):
                continue
            op |= {"key": match.group("key"), "path": path}
            if match.group("root") and not HOME_ROOT.fullmatch(posixpath.normpath(match.group("root"))):
                op["skip"] = "wrong_root"
        elif name == "Bash" and MEMORY_PATH.search((inp.get("command") or "").replace("\\", "/")):
            op |= {"key": MEMORY_PATH.search(inp["command"].replace("\\", "/")).group("key"), "path": ""}
        else:
            continue
        if done:
            op["line"], op["failed"] = done[0], bool(done[1].get("is_error"))
        elif not op["skip"]:
            op["skip"] = "no_result"
        tur = done[2] if done else None
        if name == "Read":
            f = tur.get("file") if isinstance(tur, dict) else None
            if op["skip"] or op["failed"] or not isinstance(f, dict) or tur.get("type") == "file_unchanged":
                continue
            if not isinstance(f.get("content"), str) or f.get("startLine") != 1 or f.get("numLines") != f.get("totalLines"):
                unusable["Read of part of a file"] += 1
                continue
            op |= {"kind": "snapshot", "content": f["content"]}
        elif name == "Write":
            op |= {"kind": "write", "content": inp.get("content") or ""}
        elif name in ("Edit", "MultiEdit"):
            base = tur.get("originalFile") if isinstance(tur, dict) else None
            op |= {"kind": "edit", "edits": [inp] if name == "Edit" else list(inp.get("edits") or []),
                   "base": base if isinstance(base, str) else None}
        else:
            op |= {"kind": "bash", "command": inp.get("command") or "", "stdout": bash_output(tur if done else None)}
        ops.append(op | {"time": times[op["line"] - first]})
    return sorted(ops, key=lambda o: o["line"]), unusable


def memory_dir(key):
    return MEMORY / key


class Store:
    """The memory directories as one replay sees them, by absolute path, recording what each step did."""

    def __init__(self, key):
        self.key, self.effects, self.unusable, self.deleted = key, [], Counter(), set()

    def locate(self, path):
        m = MEMORY_PATH.search(path or "")
        if not m or (m.group("root") and not HOME_ROOT.fullmatch(posixpath.normpath(m.group("root")))):
            return None
        return m.group("key"), (m.group("rest") or "").strip("/")

    def is_memory(self, path):
        return self.locate(path) is not None

    def file(self, path):
        key, rest = self.locate(path)
        if not rest or ".." in Path(rest).parts or key in ("", ".", ".."):
            raise replay.Unparsed("a path that is not a memory file")
        return key, rest, memory_dir(key) / rest

    def exists(self, path):
        return self.locate(path) is not None and self.file(path)[2].is_file()

    def is_dir(self, path):
        return self.locate(path) is not None and (memory_dir(self.locate(path)[0]) / self.locate(path)[1]).is_dir()

    def unseen(self, path):
        return not self.exists(path) and self.locate(path)[:2] not in self.deleted

    def read(self, path):
        return self.file(path)[2].read_text()

    def effect(self, path, outcome, kind, detail=None):
        key, rest = self.locate(path) if self.locate(path) else (self.key, "")
        self.effects.append({"key": key, "file": rest, "outcome": outcome, "kind": kind} | ({"detail": detail} if detail else {}))

    def put(self, path, text, append, kind):
        key, rest, target = self.file(path)
        if append and target.is_file():
            text = target.read_text() + text
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text)
        self.deleted.discard((key, rest))
        self.effect(path, "applied", "append" if append else "write" if kind == "bash" else kind)

    def delete(self, path):
        key, rest, target = self.file(path)
        if target.is_file():
            target.unlink()
            self.deleted.add((key, rest))
            self.effect(path, "applied", "delete")
        else:
            self.effect(path, "delete_file_unknown", "delete")

    def snapshot(self, path, text):
        """Set the file to what the transcript shows it held; the line endings of an output are its own."""
        if not text:
            return
        text = text if text.endswith("\n") else text + "\n"
        key, rest, target = self.file(path)
        if target.is_file() and target.read_text().rstrip("\n") == text.rstrip("\n"):
            self.effect(path, "unchanged", "snapshot")
            return
        detail = "corrected" if target.is_file() else "new"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text)
        self.deleted.discard((key, rest))
        self.effect(path, "applied", "snapshot", detail)

    def unparsed(self, path, reason):
        self.effect(path, "bash_unparsed", "bash", reason)

    def read_unusable(self, reason, n):
        if n:
            self.unusable[reason] += n


def apply_op(op):
    """Apply one recorded memory operation to the memory directories, as Claude Code's tools would have: an Edit whose
    text is absent fails. Returns what it did, one effect per file it touched, and the reads it could not use."""
    store = Store(op["key"])
    if op["failed"] and op["kind"] != "bash":
        store.effect(op["path"], "recorded_failed", op["kind"])
    elif op["skip"] and (op["kind"] != "bash" or re.search(r">|\brm\b|python|\bsed\b|\btee\b|\bmv\b|\bcp\b", op["command"])):
        store.effect(op["path"], op["skip"], op["kind"])
    elif op["skip"]:
        pass
    elif op["kind"] == "snapshot":
        store.snapshot(op["path"], op["content"])
    elif op["kind"] == "write":
        store.put(op["path"], op["content"], False, "write")
    elif op["kind"] == "edit":
        if op["base"] is not None:
            store.snapshot(op["path"], op["base"])
        text = store.read(op["path"]) if store.exists(op["path"]) else None
        for edit in op["edits"]:
            old, new = edit.get("old_string") or "", edit.get("new_string") or ""
            if text is None and old == "":
                text = new
                continue
            if text is None:
                store.effect(op["path"], "edit_file_unknown", "edit")
                return store
            n = text.count(old)
            if n == 0 or (n > 1 and not edit.get("replace_all")):
                store.effect(op["path"], "edit_target_missing" if n == 0 else "edit_ambiguous", "edit")
                return store
            text = text.replace(old, new) if edit.get("replace_all") else text.replace(old, new, 1)
        store.put(op["path"], text, False, "edit")
    else:
        try:
            replay.Shell(store, "/unknown-cwd").run(op["command"], op["failed"], op["stdout"])
        except replay.Unparsed as e:
            if re.search(r">|\brm\b|python|\bsed\b|\btee\b|\bmv\b|\bcp\b", op["command"]):
                store.unparsed("", str(e))
    return store


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


def memory_saves(entries, session_key):
    """Claude Code's own memory saves in a session, as (project key, verb, file names): the project is the one the
    written paths name, else the session's. The transcript holds the names and not the content."""
    saves = []
    for e in entries:
        if e.get("type") == "system" and e.get("subtype") == "memory_saved":
            hits = [MEMORY_PATH.search(str(w).replace("\\", "/")) for w in e.get("writtenPaths") or ()]
            hits = [m for m in hits if m]
            saves.append((hits[0].group("key") if hits else session_key, e.get("verb"),
                          [(m.group("rest") or "/").lstrip("/") for m in hits]))
    return saves


def recorded(plan, roots):
    """Every memory operation of every session, the project keys by repository and the sessions that made any, in
    the order they took effect: per project by the result's time, then the session's start and the entry's line.
    Also Claude Code's own memory saves, per project."""
    ops, keys, made, saves, unusable = [], {}, set(), {}, Counter()
    for start, repo, path in plan:
        entries = parse_lines(lines_of(path))
        sid = session_id(path)
        first = own_start(entries, sid)
        entries = entries[first:]
        cwd = next((e["cwd"] for e in entries if e.get("cwd")), "/")
        key = key_of(roots[repo], cwd)
        keys.setdefault(repo, set()).add(key)
        for save_key, verb, names in memory_saves(entries, key):
            mine = saves.setdefault(save_key, {"saves": 0, "verbs": Counter(), "files": Counter()})
            mine["saves"] += 1
            mine["verbs"][str(verb)] += 1
            mine["files"].update(names)
        found, missed = recorded_ops(entries, key, start, first)
        unusable += missed
        for op in found:
            ops.append(op | {"repo": repo, "session": sid, "start": start})
            made.add(sid)
    ops.sort(key=lambda o: (o["key"], o["time"], o["start"], o["line"]))
    return ops, keys, len(made), unusable, {k: {"saves": v["saves"], "verbs": dict(sorted(v["verbs"].items())), "files": dict(sorted(v["files"].items()))}
                                  for k, v in sorted(saves.items())}


def packed_keys(keys):
    """Every project key the store holds by repository: the sessions' own, and under "(other)" any a recorded write named."""
    extra = {d.name for d in MEMORY.iterdir()} - {k for ks in keys.values() for k in ks} if MEMORY.is_dir() else set()
    return keys | ({"(other)": extra} if extra else {})


KEPT = ("applied", "unchanged", "recorded_failed")


def manifest(skipped_files, done, keys, sessions_made, saves, unusable):
    """`done` holds (operation, effect) pairs in the order they were applied."""
    inputs = (WORK / "key.txt").read_text().splitlines()
    keys = packed_keys(keys)
    applied = [o for o, e in done if e["outcome"] == "applied"]
    writes = [o for o, e in done if e["outcome"] == "applied" and e["kind"] != "snapshot"]
    outcomes = [e["outcome"] for _, e in done]
    projects = {}
    for repo, ks in sorted(keys.items()):
        projects[repo] = {}
        for key in sorted(ks):
            mine = [e for _, e in done if e["key"] == key]
            count = Counter(e["outcome"] for e in mine)
            projects[repo][key] = {
                "operations": len(mine), "applied": dict(sorted(Counter(e["kind"] for e in mine if e["outcome"] == "applied").items())),
                "unchanged": count.pop("unchanged", 0), "recorded_failed": count.pop("recorded_failed", 0),
                "skipped": dict(sorted((k, v) for k, v in count.items() if k != "applied"))
            } | stats(memory_dir(key))
    unparsed = [(o, e) for o, e in done if e["outcome"] == "bash_unparsed"]
    return {
        "arm": "auto-memory",
        "key": inputs[0],
        "inputs": inputs[1:],
        "device": os.environ["BILBO_EVAL_DEVICE"],
        "versions": {"python": platform.python_version(), "arch": platform.machine()},
        "notes": NOTES,
        "projects": projects,
        "sessions": {"with_memory_operations": sessions_made, "skipped": skipped_files},
        "operations": {"recorded": len(done), "applied": len(applied), "unchanged": outcomes.count("unchanged"),
                       "recorded_failed": outcomes.count("recorded_failed"),
                       "skipped": dict(sorted(Counter(r for r in outcomes if r not in KEPT).items()))},
        "snapshots": dict(sorted(Counter(e.get("detail") if e["outcome"] == "applied" else e["outcome"]
                                         for _, e in done if e["kind"] == "snapshot").items())),
        "unparsed": {"writes": len(unparsed), "reads": sum(unusable.values()),
                     "write_reasons": dict(sorted(Counter(e["detail"] for _, e in unparsed).items())),
                     "read_reasons": dict(sorted(unusable.items())),
                     "commands": [{"project": e["key"], "session": o["session"], "line": o["line"], "reason": e["detail"],
                                   "command": o["command"][:200]} for o, e in unparsed]},
        "unrecorded_saves": saves,
        "writers": {"models": dict(sorted(Counter(str(o["model"]) for o in writes).items())),
                    "claude_code_versions": dict(sorted(Counter(str(o["version"]) for o in writes).items()))},
        "provenance": [{"project": e["key"], "session": o["session"], "time": o["time"].isoformat(), "tool": o["tool"], "kind": e["kind"],
                        "file": e["file"], "outcome": e["outcome"], "version": o["version"], "model": o["model"]} for o, e in done],
        "memory_files": {key: digests(memory_dir(key)) for ks in keys.values() for key in sorted(ks)},
    }


def main():
    plan, skipped_files = sessions()
    ops, keys, sessions_made, unusable, saves = recorded(plan, checkout_roots())
    shutil.rmtree(STORE, ignore_errors=True)
    if bad := sorted({o["key"] for o in ops if o["key"] in ("", ".", "..")}):
        sys.exit(f"auto-memory: a recorded write names the project key {bad}, which is not a directory name")
    for key in sorted({k for ks in keys.values() for k in ks} | {o["key"] for o in ops}):
        memory_dir(key).mkdir(parents=True, exist_ok=True)
    done = []
    for op in ops:
        store = apply_op(op)
        unusable += store.unusable
        done += [(op, e) for e in store.effects]
    (STORE / "manifest.json").write_text(json.dumps(manifest(skipped_files, done, keys, sessions_made, saves, unusable), indent=2) + "\n")
    log(f"done: {STORE}, {sum(e['outcome'] == 'applied' for _, e in done)} of {len(done)} operations applied")


if __name__ == "__main__":
    main()
