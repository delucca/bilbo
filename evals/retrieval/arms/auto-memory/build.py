"""Build the auto-memory store: replay every session through Claude Code's own auto memory, headless, in one timeline
across all sessions. Before each point the memory the developer's Claude Code wrote in that stretch of the session is
applied to the project's memory directory; Claude Code 2.1.296 then resumes a private copy of the recorded transcript,
once at each session's end and once before each compaction, and writes memory itself: the main model, and the
background extraction when the account's feature flags allow it under `claude -p`. Each project's memory lives in one
directory under the isolated CLAUDE_CONFIG_DIR. Then save those directories with a manifest in /work/store.
After every point the memory directories are the checkpoint, and a run that finds /work/progress.json resumes there."""

import ast
import hashlib
import json
import os
import platform
import queue
import re
import shutil
import signal
import subprocess
import sys
import threading
import time
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path
from types import SimpleNamespace

CORPUS = Path("/corpus")
WORK = Path("/work")
HOME = Path("/tmp/home")
CONFIG = WORK / "claude"  # CLAUDE_CONFIG_DIR, on the volume: the memory directories are the checkpoint
SNAPSHOT = WORK / "snapshot"
REPLAY = WORK / "replay"  # the replay's own transcript entries: they stay here, out of the store
DEBUG = WORK / "debug"
PROGRESS = WORK / "progress.json"
POINTS = WORK / "points.jsonl"
BIN = "/usr/local/bin/claude"
VERSION = "2.1.296"
MODEL = os.environ["BILBO_EVAL_LLM_MODEL"]
AUTH = ("CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_API_KEY")

# The replay's one message at each point. The developer said nothing there: live, the extraction ran after the last turn
# with no new message, and at a session's end no more work happened. A pause tells the model to stop without announcing
# the session's end, which a bare "ok" would not (it reads as go on or yes at about 61 of the 188 points, and only 2 of the
# corpus' 851 typed prompts are "ok"). It has whitespace, so the recall selector runs at it when its flag is on.
PROMPT = "let's pause here for a bit"
# The three choices a probe on the account can flip, each used in one place below.
STREAM_INPUT = True  # one stream-json frame, stdin held open until the extraction reports; else a plain -p prompt
FORK = False  # resume in place on the private copy; else --fork-session and read the fork's file
EXTRACT_CAP = 1800  # seconds a point waits for the background extraction to report, not counting a wait on a limit
INACTIVE = 1800  # seconds of silence on stdout before claude is taken to be stuck
UNTYPED_LIMIT = 3 * 3600  # seconds an untyped 429 is waited on before the build stops with its text
# Claude Code cuts a headless extraction off 60 s after the result (drainPendingExtraction); holding stdin open avoids it.

# The feature flags the manifest records (GrowthBook values cached per account in CLAUDE_CONFIG_DIR/.claude.json).
FLAGS = ("tengu_passport_quail", "tengu_slate_thimble", "tengu_moth_copse", "tengu_onyx_plover", "tengu_bramble_lintel",
         "tengu_gleaming_fair", "tengu_gleaming_fair_reuse", "tengu_sunny_locket", "tengu_foamy_spring",
         "tengu_ochre_finch", "tengu_gorse_fathom", "tengu_salt_marsh", "tengu_mill_orange", "tengu_vellum_anchor",
         "tengu_lantern_wick_braid", "tengu_lantern_wick_braid_release", "tengu_lantern_wick_braid_text", "tengu_lantern_wick_mode",
         "tengu_lantern_wick_release", "tengu_lantern_wick_release_by_model", "tengu_lantern_wick_resume_by_model",
         "tengu_lantern_wick_text", "tengu_lantern_wick_text_by_model")
# Kinds of API error Claude Code reports in the result that no timer cures: the build stops.
FATAL = ("long_context_credits_required", "model_requires_usage_credits", "consent_unanswered", "no_allowed_fallback",
         "model_substitution_disabled", "claude_code_version_too_old", "api_key_auth_disabled", "org_disabled_credential",
         "field_not_granted", "tls_untrusted_ca", "invalid_credential_header", "effort_requires_thinking")
FATAL_PREFIX = ("gateway_",)
AUTH_FAILED = "Not logged in"
IDENTITY = {}  # the account's own identifiers, kept in memory only, to scan the store for them
DOCS = "https://code.claude.com/docs/en"
# What differs from a plain `claude -p`, each with where Claude Code's docs or shipped code ask for it.
CONFIG_ENTRIES = (
    ("CLAUDE_CONFIG_DIR", "/work/claude", f"{DOCS}/env-vars, CLAUDE_CONFIG_DIR (l.430 of the saved page): one config directory for the whole build, so one flag cache and the memory directories persist"),
    ("CLAUDE_CODE_PROJECT_DIR_NAME", "<project key>", f"{DOCS}/memory (l.514) and {DOCS}/env-vars (l.364), CLAUDE_CODE_PROJECT_DIR_NAME: the key the developer's machine derived, so the memory directory is the recorded one"),
    ("DISABLE_AUTOUPDATER", "1", f"{DOCS}/env-vars (l.444), DISABLE_AUTOUPDATER: the build runs the pinned version"),
    ("CLAUDE_CODE_RETRY_WATCHDOG", "1", f"{DOCS}/env-vars (l.375), CLAUDE_CODE_RETRY_WATCHDOG: for unattended sessions such as eval harnesses; a session that hits a usage limit waits out the remaining window"),
    ("HOME", "/tmp/home", "a writable home in the container; nothing in it is read, since the config directory is CLAUDE_CONFIG_DIR"),
    ("cwd", "an empty directory per session, under its recorded path", "the checkout is not part of the corpus; the extraction's prompt forbids looking at it"),
    ("--model", MODEL, "BILBO_EVAL_LLM_MODEL"),
    ("--permission-prompts", "none", "claude --help 2.1.296: nobody can answer a prompt in a container. The permission mode is the default; the recorded sessions ran bypassPermissions, auto and acceptEdits. Writes of memory files are allowed without a prompt, Bash appends to MEMORY.md are not"),
    ("--output-format", "stream-json", f"{DOCS}/headless, Stream responses: the result line carries usage and cost"),
    ("--verbose", "", "claude --help 2.1.296: stream-json output under -p needs it"),
    ("--input-format", "stream-json" if STREAM_INPUT else "text", "stdin stays open until the background extraction reports" if STREAM_INPUT else "a plain prompt: the extraction gets the 60 s Claude Code allows"),
    ("--resume", "a private copy of the recorded transcript" + (", forked" if FORK else ", in place"), "the history Claude Code would have had at that point"),
    ("--debug-file", "per attempt", "the [extractMemories] lines are the only record of the background extraction"),
)
EXTRACT = "[extractMemories] "
STARTING, FINISHED, SAVED = EXTRACT + "starting", EXTRACT + "finished", EXTRACT + "memories saved: "
NOTHING, SKIPPING, ERROR = EXTRACT + "no memories saved this run", EXTRACT + "skipping", EXTRACT + "error: "
DENIED, STRIP = "[autoMem] denied", "[thinking] server rejected a thinking block"
NOTES = {
    "approximation": "memory is written from the transcript alone: the checkout is empty, and the project's CLAUDE.md or AGENTS.md that the developer's Claude Code loaded is absent, so the model cannot look at files it would have read on the developer's machine",
    "closing_prompt": f"{PROMPT!r}, the replay's own: the developer said nothing at a session's end. At a point that ends mid-task or on a question the model is asked to pause there. English for every project, including kimi-cli, whose developer wrote mostly in Chinese: the memory's language may follow it",
    "recorded_writes": "before each point, the memory operations the developer's Claude Code recorded in that stretch of the session are applied to the project's memory directory (successful Write, Edit and MultiEdit on a memory file, and the Bash writes of three known shapes; a recorded command is never executed). Content is as the tool input had it: Claude Code's own frontmatter stamp is not applied. No Read seeds the start, so each project's memory starts empty although some developers' had indexes of 14 and 22 topics before the corpus. The one redacted path in the corpus (iptvnator) is taken as the MEMORY.md of the session's key. A command of a shape not known, or one whose quoting the parser does not follow, is a counted skip, never run",
    "direct_write_skip": "the background extraction skips a pass when the loaded history holds a Write or Edit under the current memory directory; recorded writes sit under the developer's own prefix, so they never trip it. A main-model write at the closing message does (closing_turn_preempted counts it)",
    "time": "memory files carry the build's clock; Claude Code stamps modified itself",
    "date": "the model is told the build's date, not the session's: a relative date in a session resolves against it, and there is no lever for the date",
    "thinking": "the recorded thinking blocks carry foreign signatures: the API rejects them and Claude Code strips every thinking block, for the main turn and the extraction",
    "review": "one background review per point, not one after every turn of the session; 11 session ends are an interrupt marker, which live was never reviewed",
    "llm": "calls are claude processes (points plus warm-ups). The main model's usage is the result line's; the background extraction runs after the result and its cache read, create and input tokens come from its debug line (llm.extraction), its output tokens are not reported locally. The calls of work an interrupt or a redo threw away are not counted",
    "unreplayed": "no auto dream: it never runs under -p (the sdk-cli entrypoint) and its gates are wall-clock (24 h and 5 sessions by file time); no command runs it by hand. The recall selector does run at each point's closing prompt when its flag is on, as on a laptop",
    "account": "with an OAuth login Claude Code adds the account's email address to every request's context; every memory directory and every point's record is scanned for the account's identifiers after each point, and the store again before it is written. The replay's own transcript entries stay in the work folder, out of the store",
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


def boundary(entry):
    return entry.get("type") == "system" and entry.get("subtype") == "compact_boundary"


def main_reply(entry):
    """An assistant entry of the main thread that a model wrote: a turn ended there."""
    return (entry.get("type") == "assistant" and not entry.get("isSidechain")
            and (entry.get("message") or {}).get("model") != "<synthetic>")


def prose(entry):
    """Whether a user entry has the three words of prose Claude Code's extraction looks for: a string or one text block.
    Command records and compact summaries count, tool results do not."""
    if entry.get("type") != "user" or entry.get("isMeta") or entry.get("isSidechain"):
        return False
    content = (entry.get("message") or {}).get("content")
    texts = [content] if isinstance(content, str) else [b.get("text", "") for b in content or [] if isinstance(b, dict) and b.get("type") == "text"]
    return any(len(t.split()) >= 3 for t in texts)


def context_at(entries, i):
    """The tokens the main model read at its last reply at or before entry i, as recorded."""
    for e in reversed(entries[: i + 1]):
        if main_reply(e):
            u = (e.get("message") or {}).get("usage") or {}
            return sum(u.get(k) or 0 for k in ("input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"))
    return 0


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


def recorded_ops(entries, session_key):
    """Every memory operation in a session, with the entry its result is in (where it took effect) and why it cannot
    be applied, if so."""
    uses, results, seen = {}, {}, set()
    for line, e in enumerate(entries):
        if e.get("uuid"):
            if e["uuid"] in seen:
                continue
            seen.add(e["uuid"])
        content = (e.get("message") or {}).get("content") if isinstance(e.get("message"), dict) else None
        for b in content if isinstance(content, list) else ():
            if isinstance(b, dict) and b.get("id") and b.get("type") == "tool_use":
                uses.setdefault(b["id"], (line, b))
            elif isinstance(b, dict) and b.get("tool_use_id") and b.get("type") == "tool_result":
                results.setdefault(b["tool_use_id"], (line, b))
    ops = []
    for tid, (use_line, b) in uses.items():
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
        op = {"tool": name, "key": key, "file": rest, "line": use_line, "skip": None, "failed": False}
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
        ops.append(op)
    return sorted(ops, key=lambda o: o["line"])


def apply_ops(ops):
    """Apply recorded memory operations in order to the memory directories, as Claude Code's tools would have: an Edit
    whose text is absent fails. Returns the counts applied, the skips by reason and the recorded failures."""
    applied, skipped, recorded_failed = Counter(), Counter(), 0
    for op in ops:
        if op["failed"]:
            recorded_failed += 1
            continue
        if op["skip"]:
            skipped[op["skip"]] += 1
            continue
        target = memory_dir(op["key"]) / op["file"]
        if ".." in Path(op["file"]).parts or not op["file"] or Path(op["file"]).is_absolute():
            skipped[f"{op['kind']}_bad_path"] += 1
            continue
        current = target.read_text() if target.is_file() else None
        kind, text = op["kind"], None
        if kind == "write":
            text = op["content"]
        elif kind == "edit":
            text = current
            for edit in op["edits"]:
                old, new = edit.get("old_string") or "", edit.get("new_string") or ""
                if text is None:
                    skipped["edit_file_unknown"] += 1
                    break
                n = text.count(old)
                if n == 0 or (n > 1 and not edit.get("replace_all")):
                    skipped["edit_target_missing" if n == 0 else "edit_ambiguous"] += 1
                    text = None
                    break
                text = text.replace(old, new) if edit.get("replace_all") else text.replace(old, new, 1)
            if text is None:
                continue
        elif op["shape"] == "write":
            text = op["payload"]
        elif current is None:
            skipped["bash_unknown_file"] += 1
            continue
        elif op["shape"] == "append":
            text = current + ("" if current.endswith("\n") or current == "" else "\n") + op["payload"]
        else:
            text = current
            for old, new in op["payload"]:
                if old not in text:
                    skipped["bash_replace_target_missing"] += 1
                    text = None
                    break
                text = text.replace(old, new)
            if text is None:
                continue
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text)
        applied[kind] += 1
    return {"write": applied["write"], "edit": applied["edit"], "bash": applied["bash"], "skipped": dict(skipped),
            "recorded_failed": recorded_failed}


def points(plan, roots):
    """Every replay point as a dict, all sessions in the order they happened, how many were skipped by reason, each
    repository's project keys and the memory operations the corpus records by key. A point is a session's last entry
    and the last entry before each compaction (a resume after one sees only the summary). Claude Code's own rules skip a
    point: no turn ended since the previous point (its stop hooks never ran), or its loaded history, from the last
    compaction, holds no three words of user prose. A point carries the recorded memory operations of its stretch of the
    session, the one since the previous point replayed."""
    out, skipped, keys, recorded = [], Counter(), {}, {}
    for start, repo, path in plan:
        entries = parse_lines(lines_of(path))
        sid = next(e["sessionId"] for e in entries if e.get("sessionId"))
        cwd = next((e["cwd"] for e in entries if e.get("cwd")), "/")
        key = key_of(roots[repo], cwd)
        keys.setdefault(repo, set()).add(key)
        ops = recorded_ops(entries, key)
        for op in ops:
            if not op["skip"] and not op["failed"]:
                found = recorded.setdefault(op["key"], {"writes": 0, "edits": 0, "bash": 0, "files": set()})
                found[{"write": "writes", "edit": "edits", "bash": "bash"}[op["kind"]]] += 1
                found["files"].add(op["file"])
        times, when = [], start
        for e in entries:
            when = stamp(e) or when
            times.append(when)
        last = len(entries) - 1
        marks = sorted({i for i in range(last) if boundary(entries[i + 1])} | {last})
        previous = replayed = -1
        last_point = None
        for n, i in enumerate(marks):
            since = max([b for b in range(i + 1) if boundary(entries[b])], default=0)
            reason = ("no_turn" if not any(main_reply(e) for e in entries[previous + 1: i + 1])
                      else "no_prose" if not any(prose(e) for e in entries[since: i + 1]) else None)
            previous = i
            if reason:
                skipped[reason] += 1
                continue
            mine = [o for o in ops if replayed < o["line"] <= i]
            replayed = i
            last_point = {"id": f"{repo}/{sid}/{n}", "repo": repo, "sid": sid, "n": n, "kind": "end" if i == last else "compact",
                        "line": i, "time": times[i], "start": start, "cwd": cwd, "key": key, "path": path, "context": context_at(entries, i),
                        "ops": mine, "touch": sorted({key} | {o["key"] for o in mine})}
            out.append(last_point)
        if last_point:  # operations after the session's last replayed point: counted, never applied
            last_point["ops"] += [o | {"skip": "after_last_point"} for o in ops if o["line"] > replayed and not o["skip"]]
            last_point["touch"] = sorted(set(last_point["touch"]) | {o["key"] for o in last_point["ops"]})
    out.sort(key=lambda p: (p["time"], p["start"], p["n"]))
    for index, p in enumerate(out):
        p["index"] = index
    return out, dict(skipped), keys, {k: v | {"files": len(v["files"])} for k, v in sorted(recorded.items())}


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


def claude_env(key):
    """Claude Code's environment, built from scratch: nothing of the caller's reaches it but the one credential."""
    env = {"HOME": str(HOME), "PATH": "/usr/local/bin:/usr/bin:/bin", "LANG": "C.UTF-8", "TMPDIR": "/tmp",
           "CLAUDE_CONFIG_DIR": str(CONFIG), "CLAUDE_CODE_PROJECT_DIR_NAME": key, "DISABLE_AUTOUPDATER": "1",
           "CLAUDE_CODE_RETRY_WATCHDOG": "1"}
    credential = next((v for v in AUTH if os.environ.get(v)), None)
    return env | ({credential: os.environ[credential]} if credential else {})


def on(value):
    """A gate that reads `enabled` or `available` of an object flag."""
    return value is True or (isinstance(value, dict) and (value.get("enabled") is True or value.get("available") is True))


def truthy(value):
    """JavaScript truthiness of a flag value, which is how Claude Code gates most of them."""
    return not (value is None or value is False or value == 0 or value == "")


def read_cache():
    try:
        return json.loads((CONFIG / ".claude.json").read_text())
    except (OSError, ValueError):
        return {}


def read_flags():
    """The memory flags Claude Code cached for this account, by name: values only, no account identifier."""
    cache = read_cache()
    values = {name: (cache.get("cachedGrowthBookFeatures") or {}).get(name) for name in FLAGS}
    return {"cached_at": bool(cache.get("cachedGrowthBookFeaturesAt")), "values": values,
            "extraction_under_p": truthy(values["tengu_passport_quail"]) and truthy(values["tengu_slate_thimble"]),
            "recall": truthy(values["tengu_moth_copse"]), "dream": on(values["tengu_onyx_plover"]),
            "prompt_variant": "no_index" if truthy(values["tengu_moth_copse"]) else "index"}


def flags_problem(flags):
    """Why this account cannot run the arm, if it cannot: the background extraction must run under -p on every turn."""
    values = flags["values"]
    if not flags["extraction_under_p"]:
        return "the background extraction does not run under -p on this account (tengu_passport_quail and tengu_slate_thimble must both be on)"
    cadence = values["tengu_bramble_lintel"]
    if cadence is not None and not isinstance(cadence, bool):
        try:
            every = float(cadence)  # JavaScript compares a numeric string as a number
        except (TypeError, ValueError):
            every = None
        if every is None or every > 1:
            return f"the extraction runs only every {cadence!r} turns (tengu_bramble_lintel), and a point is one turn"
    return None


def load_identity():
    """The account's identifiers from Claude Code's own cache, kept in memory only and added to as the cache changes:
    the store is scanned for them."""
    account = read_cache().get("oauthAccount") or {}
    for field in ("emailAddress", "accountUuid", "organizationUuid"):
        if isinstance(account.get(field), str) and len(account[field]) >= 4:
            IDENTITY.setdefault(field, set()).add(account[field])
    for field in ("fullName", "displayName"):
        if isinstance(account.get(field), str) and len(account[field].split()) >= 2:
            IDENTITY.setdefault(field, set()).add(account[field])


def identity_hits(name, text):
    """The account identifiers a file or text holds, as (name, field): never the value."""
    low = text.lower()
    return [(name, field) for field, values in IDENTITY.items() if any(v.lower() in low for v in values)]


def scan_dir(root, label=None):
    return [h for f in sorted(root.rglob("*")) if f.is_file()
            for h in identity_hits(f"{label or root.name}/{f.relative_to(root).as_posix()}", f.read_bytes().decode("utf-8", "replace"))]


def atomic(path, text):
    tmp = path.with_name(path.name + ".tmp")
    with tmp.open("w") as f:
        f.write(text)
        f.flush()
        os.fsync(f.fileno())
    tmp.rename(path)


def save(progress):
    atomic(PROGRESS, json.dumps(progress, indent=2) + "\n")


def memory_dir(key):
    return CONFIG / "projects" / key / "memory"


def digests(root):
    return {p.relative_to(root).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in sorted(root.rglob("*")) if p.is_file()} if root.is_dir() else {}


def clear_project(key):
    """Everything in the project directory but its memory: the point's transcript copy, its subdirectory, a fork."""
    for p in (CONFIG / "projects" / key).glob("*") if (CONFIG / "projects" / key).is_dir() else ():
        if p.name != "memory":
            shutil.rmtree(p) if p.is_dir() else p.unlink()


def snapshot(key):
    """Copy the memory directory aside, under a temporary name first: a stop in the middle leaves the last snapshot whole."""
    SNAPSHOT.mkdir(exist_ok=True)
    tmp = SNAPSHOT / f"{key}.new"
    shutil.rmtree(tmp, ignore_errors=True)
    if memory_dir(key).is_dir():
        shutil.copytree(memory_dir(key), tmp)
    drop_snapshot(key)
    if memory_dir(key).is_dir():
        tmp.rename(SNAPSHOT / key)
    else:
        (SNAPSHOT / f"{key}.absent").touch()


def restore(key, tag=""):
    """Put the memory directory back as it was before the point, whatever the point wrote before it was cut off. A
    snapshot cut between its drop and its rename is whole under `.new`."""
    shutil.rmtree(memory_dir(key), ignore_errors=True)
    for name in (f"{tag}{key}", f"{tag}{key}.new"):
        if (SNAPSHOT / name).is_dir():
            shutil.copytree(SNAPSHOT / name, memory_dir(key))
            break
    clear_project(key)


def restore_all(keys):
    for k in keys:
        restore(k)


def drop_snapshot(key):
    shutil.rmtree(SNAPSHOT / key, ignore_errors=True)
    (SNAPSHOT / f"{key}.absent").unlink(missing_ok=True)


def keep_snapshot(key, tag):
    """Copy a failed point's snapshot under a tag, so three failures in a row can be undone. The original stays until
    progress is saved."""
    for suffix in ("", ".absent"):
        src, dst = SNAPSHOT / f"{key}{suffix}", SNAPSHOT / f"{tag}{key}{suffix}"
        shutil.rmtree(dst, ignore_errors=True)  # a copy a stop left behind
        dst.unlink(missing_ok=True)
        if src.is_dir():
            shutil.copytree(src, dst)
        elif src.exists():
            shutil.copy(src, dst)


def drop_streak_snapshots():
    for old in SNAPSHOT.glob("streak-*"):
        shutil.rmtree(old) if old.is_dir() else old.unlink()


class Child:
    """The claude process of the moment, killed with its helpers if the build stops."""

    process = None

    @classmethod
    def kill(cls):
        if cls.process and cls.process.poll() is None:
            try:
                os.killpg(cls.process.pid, signal.SIGKILL)
            except OSError:
                pass
            cls.process.wait()


class Run:
    """What one claude process said: its result line, its system lines, the rate-limit lines, its stderr tail and the
    file its whole stream went to."""

    def __init__(self, stream):
        self.result, self.system, self.events, self.stderr, self.waited, self.stream = None, [], [], "", False, stream

    @property
    def init(self):
        return next((s for s in self.system if s.get("subtype") == "init"), None)


def waiting_out(text):
    """Whether the extraction's fork is backing off: it has started, and the newest API-error or extraction line since
    is a 429 or 529. The main turn's retries come before the extraction starts and do not count, nor does any other line."""
    lines = text.splitlines()
    started = [i for i, x in enumerate(lines) if STARTING in x]
    if not started:
        return False
    relevant = [x for x in lines[started[-1] + 1:] if EXTRACT in x or "API error (attempt" in x]
    return bool(relevant) and bool(re.search(r"API error \(attempt \d+/\d+\): (429|529)\b", relevant[-1]))


def run_claude(args, key, cwd, debug, stdin_frame=None, hold=None):
    """One claude process: its stdout lines until the result, then, while `hold` (called with the debug text and the
    seconds waited, not counting a wait on a limit) says so and the result is not an error, its stdin stays open."""
    debug.unlink(missing_ok=True)
    stderr = debug.with_suffix(".stderr")
    stream = debug.with_suffix(".stream.jsonl")
    cmd = [BIN, "-p", "--model", MODEL, "--verbose", "--output-format", "stream-json", "--permission-prompts", "none",
           "--debug-file", str(debug), *args]
    Child.process = proc = subprocess.Popen(cmd, cwd=cwd, env=claude_env(key), stdin=subprocess.PIPE if stdin_frame else subprocess.DEVNULL,
                                            stdout=subprocess.PIPE, stderr=stderr.open("w"), text=True, start_new_session=True)
    lines = queue.Queue()

    def read():
        with stream.open("w") as out:
            for line in proc.stdout:
                out.write(line)
                out.flush()
                lines.put(line)
        lines.put(None)

    reader = threading.Thread(target=read, daemon=True)
    reader.start()
    run, deadline = Run(stream), time.monotonic() + INACTIVE
    try:
        if stdin_frame:
            try:
                proc.stdin.write(stdin_frame + "\n")
                proc.stdin.flush()
            except OSError:  # claude is already gone: no result, so the point is retried
                pass
        while run.result is None and time.monotonic() < deadline:
            try:
                line = lines.get(timeout=5)
            except queue.Empty:
                continue
            if line is None:
                break
            deadline = time.monotonic() + INACTIVE
            try:
                entry = json.loads(line)
            except ValueError:
                continue
            if entry.get("type") == "system":
                run.system.append(entry)
                if entry.get("subtype") == "api_retry":  # Claude Code is backing off: give it the delay and ten minutes more
                    deadline = max(deadline, time.monotonic() + (entry.get("retry_delay_ms") or 0) / 1000 + 600)
            elif entry.get("type") == "result":
                run.result = entry
            elif entry.get("type") == "rate_limit_event" or entry.get("api_error_params"):
                run.events.append(entry)
        if run.result is not None and hold and not run.result.get("is_error"):
            active, tick, begun = 0.0, time.monotonic(), time.monotonic()
            while proc.poll() is None:
                text = debug.read_text(errors="replace") if debug.exists() else ""
                now = time.monotonic()
                if waiting_out(text):
                    if not run.waited:
                        log("claude is waiting out a limit in the background extraction")
                    run.waited = True
                else:
                    active += now - tick
                tick = now
                if not hold(text, active) or now - begun > EXTRACT_CAP + 6 * 3600:
                    break
                time.sleep(2)
        if proc.stdin and not proc.stdin.closed:
            try:
                proc.stdin.close()
            except OSError:
                pass
        try:
            proc.wait(120)
        except subprocess.TimeoutExpired:
            pass
    finally:
        Child.kill()
        Child.process = None
    reader.join(10)  # the stream file is whole before anything counts from it
    run.stderr = stderr.read_text(errors="replace")[-2000:]
    return run


def api_error_kind(result):
    """The kind of API error that ended the turn, as the result reports it."""
    error = (result or {}).get("api_error")
    if isinstance(error, dict):
        error = error.get("kind") or error.get("type")
    return error if isinstance(error, str) else None


def classify(result):
    """ok, limit (a usage limit: wait for its reset), fatal (no timer cures it: stop), auth (stop), transient (retry)
    or other (the point failed)."""
    if result is None:
        return "transient"
    if not result.get("is_error"):
        return "ok"
    kind, status = api_error_kind(result), result.get("api_error_status")
    text = str(result.get("result") or "")
    if kind in FATAL or (kind or "").startswith(FATAL_PREFIX):
        return "fatal"
    if kind == "usage_limit_reached":
        return "limit"
    if status in (401, 403) or AUTH_FAILED in text or (kind or "").startswith("authentication"):
        return "auth"
    if str(result.get("subtype", "")).startswith("error_max"):
        return "other"
    if kind == "no_response" or (isinstance(status, int) and status >= 500) or re.search(r"overloaded|connection|timed? ?out|ECONN", text, re.I):
        return "transient"
    if status == 429 or re.search(r"usage limit|rate limit|limit resets|hit your limit", text, re.I):
        return "limit"
    if kind:
        return "other"
    return "transient" if status is None else "other"


def limit_wait(run):
    """Seconds to wait for a usage limit: until the reset time of the limit that refused (the error's own record, else
    a rejected rate-limit event) plus two minutes, else ten minutes."""
    resets = []
    for e in run.events:
        own = e.get("rate_limit_info") or {}
        info = (e.get("api_error_params") or {}).get("rate_limit_info") or (own if own.get("status") == "rejected" else {})
        at = info.get("resetsAt")
        if isinstance(at, (int, float)):
            resets.append(at / 1000 if at > 1e12 else at)
    return max(0, max(resets) - time.time()) + 120 if resets else 600


def init_problem(init, key):
    """What is wrong with the system/init line, if anything: auto memory must write where the build reads, with the
    model and version asked for, with analytics on (the flags die with telemetry off) and no team memory store."""
    if not init:
        return "claude printed no system/init line"
    paths = init.get("memory_paths") or {}
    auto = (paths.get("auto") or "").rstrip("/")
    want = {"memory_paths.auto": (auto, str(memory_dir(key))), "model": (init.get("model"), MODEL),
            "claude_code_version": (init.get("claude_code_version"), VERSION)}
    wrong = [f"{k} is {got!r}, expected {exp!r}" for k, (got, exp) in want.items() if got != exp]
    if init.get("analytics_disabled"):
        wrong.append("analytics_disabled is true")
    if paths.get("team"):
        wrong.append("a team memory store is set (memory_paths.team)")
    return "; ".join(wrong) or None


def init_facts(init):
    init = init or {}
    paths = init.get("memory_paths") or {}
    return {"memory_paths": {"auto": paths.get("auto"), "team": paths.get("team")}, "model": init.get("model"),
            "claude_code_version": init.get("claude_code_version"), "analytics_disabled": init.get("analytics_disabled"),
            "effort": init.get("effort"), "permissionMode": init.get("permissionMode"), "apiKeySource": init.get("apiKeySource")}


def warmup(progress):
    """One throwaway call so Claude Code fetches the account's feature flags into CLAUDE_CONFIG_DIR, and the flags
    recorded. Flags that land after the first exit take a second call. The build stops here unless the account runs the
    background extraction under -p on every turn."""
    tries = 0
    while not progress["flags"].get("cached_at") and progress["warmups"] < 2:
        fetched_before = read_cache().get("cachedGrowthBookFeaturesAt")
        run = run_claude(["Reply with the single word ready."], "warmup", HOME, DEBUG / "warmup.log")
        shutil.rmtree(CONFIG / "projects" / "warmup", ignore_errors=True)
        outcome = classify(run.result)
        what = (run.result or {}).get("result") or run.stderr[-300:]
        if outcome == "auth":
            sys.exit(f"auto-memory: claude is not logged in with the credential given ({str(what)[:200]})")
        if outcome == "fatal":
            sys.exit(f"auto-memory: the warm-up call was refused ({api_error_kind(run.result)}); run again once that is settled")
        if outcome in ("limit", "transient"):
            tries += outcome == "transient"
            if tries >= 10:
                sys.exit(f"auto-memory: the warm-up call failed 10 times ({what}); run again")
            progress["retries"]["limit_pauses" if outcome == "limit" else "transient"] += 1
            wait = limit_wait(run) if outcome == "limit" else 60 * tries
            log(f"warm-up: {outcome} ({str(what)[:100]!r}); waiting {round(wait)} s")
            time.sleep(wait)
            continue
        if outcome != "ok":
            sys.exit(f"auto-memory: the warm-up call failed ({what})")
        problem = init_problem(run.init, "warmup")
        if problem:
            sys.exit(f"auto-memory: the warm-up's system/init is wrong: {problem}")
        progress["warmups"] += 1
        progress["auth"] = {"credential": run.init.get("apiKeySource"), "variable": next(v for v in AUTH if os.environ.get(v))}
        add_usage(progress["warmup"], run.result)
        progress["flags"] = read_flags()
        if fetched_before is not None and read_cache().get("cachedGrowthBookFeaturesAt") == fetched_before:
            progress["flags"]["cached_at"] = False  # the cache is the first run's: not a fresh fetch, so warm up again
        save(progress)
    flags = progress["flags"]
    problem = flags_problem(flags)
    if problem:
        progress["flags"], progress["warmups"] = {}, 0  # a rerun fetches the flags again
        save(progress)
        sys.exit(f"auto-memory: {problem}; nothing is built. Run again after the account or its flags change (the flags are fetched again), or with --rebuild")
    log(f"flags: extraction under -p {flags['extraction_under_p']}, prompt variant {flags['prompt_variant']}")


MODEL_FIELDS = ("inputTokens", "outputTokens", "cacheReadInputTokens", "cacheCreationInputTokens", "webSearchRequests", "costUSD")


def add_usage(total, result):
    total["calls"] = total.get("calls", 0) + 1
    for k in ("input_tokens", "output_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"):
        total[k] = total.get(k, 0) + ((result.get("usage") or {}).get(k) or 0)
    total["total_cost_usd"] = round(total.get("total_cost_usd", 0) + (result.get("total_cost_usd") or 0), 6)
    by_model = total.setdefault("by_model", {})
    for name, usage in (result.get("modelUsage") or {}).items():
        for k in MODEL_FIELDS:
            if isinstance(usage.get(k), (int, float)) and not isinstance(usage.get(k), bool):
                by_model.setdefault(name, {})[k] = round(by_model.get(name, {}).get(k, 0) + usage[k], 6)


def debug_facts(text):
    """What the debug file says of the background extraction, the thinking retry and the last API error, and the
    status each extraction error carries (its own, else the API error line before it)."""
    found, saved, failures, api_status = Counter(), [], [], None
    for line in text.splitlines():
        error = re.search(r"API error \(attempt \d+/\d+\): (\d{3})", line)
        if error:
            api_status = found["last_api_error_status"] = int(error[1])
        if STARTING in line:
            found["started"] += 1
            api_status = None  # the main turn's status is not the fork's
        elif FINISHED in line:
            found["finished"] += 1
            found["files_written"] += int((re.search(r"(\d+) files? written", line) or [0, 0])[1])
            usage = re.search(r"cache: read=(\d+) create=(\d+) input=(\d+)", line)
            if usage:
                found["extraction_cache_read"] += int(usage[1])
                found["extraction_cache_create"] += int(usage[2])
                found["extraction_input"] += int(usage[3])
        elif SAVED in line:
            saved += [p.strip() for p in line.split(SAVED, 1)[1].split(", ")]
        elif NOTHING in line:
            found["nothing"] += 1
        elif SKIPPING in line:
            found["skipped_" + ("direct_write" if "already wrote to memory" in line else "no_prose" if "no user prose" in line else "rule")] += 1
        elif ERROR in line:
            found["errors"] += 1
            own = re.search(r"error: (?:\w*Error: )?(\d{3})\b", line)
            failures.append(int(own[1]) if own else api_status)
        elif DENIED in line:
            found["denied"] += 1
        elif STRIP in line:
            found["strips"] += 1
    found["cut_off"] = int(found["started"] > found["finished"] + found["errors"])
    found["transient_extraction_errors"] = sum(s in (429, 529) or (s is not None and s >= 500) for s in failures)
    return dict(found), saved


def reported(text, seconds):
    """Whether the extraction is still to report: it has not started within 20 s of the result, or it has started and
    not finished, within the cap."""
    started = STARTING in text
    done = any(m in text for m in (FINISHED, SKIPPING, ERROR, NOTHING, SAVED))
    return seconds < EXTRACT_CAP and not done and (started or seconds < 20)


def memory_writes(lines, directory):
    """The main model's Write and Edit calls on the memory directory, from the point's new transcript entries."""
    found = Counter()
    for entry in parse_lines(lines, keep_bad=False):
        found["turns"] += entry.get("type") == "assistant"
        found["memory_saved_transcript"] += entry.get("type") == "system" and entry.get("subtype") == "memory_saved"
        blocks = (entry.get("message") or {}).get("content")
        for b in blocks if entry.get("type") == "assistant" and isinstance(blocks, list) else ():
            path = (b.get("input") or {}).get("file_path") or ""
            if b.get("type") == "tool_use" and b.get("name") in ("Write", "Edit") and path.startswith(f"{directory}/"):
                found["main_" + b["name"].lower() + "s"] += 1
    return dict(found)


def entry_texts(entry):
    content = (entry.get("message") or {}).get("content") if isinstance(entry.get("message"), dict) else None
    return [content] if isinstance(content, str) else [b.get("text", "") for b in content or [] if isinstance(b, dict) and b.get("type") == "text"]


def prompt_first(entries):
    """The closing prompt is a user entry that comes before the first new assistant entry."""
    for e in entries:
        if e.get("type") == "assistant":
            return False
        if e.get("type") == "user" and not e.get("isMeta") and PROMPT in [t.strip() for t in entry_texts(e)]:
            return True
    return False


def stream_counts(path):
    """What the whole stream says, the lines after the result included (the extraction's memory_saved comes after it)."""
    found = Counter()
    for line in path.read_text(errors="replace").splitlines() if path.exists() else ():
        try:
            e = json.loads(line)
        except ValueError:
            continue
        if e.get("type") == "system":
            found[e.get("subtype")] += 1
    return found


def attempt_loop(p, progress, lines, copy):
    """Run claude on the point until it settles: apply the point's recorded writes, wait out a limit, retry a transient
    failure, redo a cut-off extraction once. Stops the build on a fault no retry cures."""
    key, touched, cwd = p["key"], p["touch"], Path(p["cwd"])
    tries = redone = attempt = untyped = 0
    cut_off = False
    while True:
        attempt += 1
        clear_project(key)
        copy.parent.mkdir(parents=True, exist_ok=True)
        copy.write_text("\n".join(lines) + "\n")
        cwd.mkdir(parents=True, exist_ok=True)
        pre = digests(memory_dir(key))
        recorded = apply_ops(p["ops"])
        before = digests(memory_dir(key))
        started = time.monotonic()
        debug = DEBUG / f"{p['index']}.{attempt}.log"
        frame = json.dumps({"type": "user", "message": {"role": "user", "content": PROMPT}}) if STREAM_INPUT else None
        args = (["--input-format", "stream-json"] if STREAM_INPUT else []) + (["--fork-session"] if FORK else []) + ["--resume", str(copy)]
        run = run_claude(args + ([] if STREAM_INPUT else [PROMPT]), key, cwd, debug, frame, reported if STREAM_INPUT else None)
        result = run.result
        outcome = classify(result)
        facts, saved = debug_facts(debug.read_text(errors="replace") if debug.exists() else "")
        if outcome == "ok" and facts["transient_extraction_errors"]:
            outcome = "transient"
        what = (result or {}).get("result") or run.stderr[-200:]
        if outcome == "auth":
            restore_all(touched)
            sys.exit(f"auto-memory: claude lost its login at {p['id']} ({str(what)[:200]})")
        if outcome == "fatal":
            restore_all(touched)
            sys.exit(f"auto-memory: claude refused {p['id']} with {api_error_kind(result)}; run again once that is settled")
        if outcome in ("limit", "transient"):
            restore_all(touched)
            redone += 1
            progress["retries"]["limit_pauses" if outcome == "limit" else "transient"] += 1
            tries += outcome == "transient"
            save(progress)
            if tries >= 10:
                sys.exit(f"auto-memory: claude failed 10 times at {p['id']}; run again to resume from the last checkpoint")
            wait = limit_wait(run) if outcome == "limit" else min(60 * tries, 600)
            if outcome == "limit" and api_error_kind(result) != "usage_limit_reached":
                untyped += wait  # a 429 that names no usage limit may be a policy no reset clears
                if untyped > UNTYPED_LIMIT:
                    sys.exit(f"auto-memory: claude answered {p['id']} with a 429 that names no usage limit for {round(untyped / 60)} minutes: {str(what)[:300]}")
            log(f"{outcome} at {p['id']} ({str(what)[:100]!r}); waiting {round(wait)} s")
            time.sleep(wait)
            continue
        problem = init_problem(run.init, key) if run.init or outcome == "ok" else None
        if problem:
            restore_all(touched)
            sys.exit(f"auto-memory: the system/init of {p['id']} is wrong: {problem}")
        if outcome == "ok" and facts["cut_off"] and not cut_off:  # the extraction started and never reported: once more
            cut_off = True
            redone += 1
            restore_all(touched)
            log(f"the extraction at {p['id']} started and never reported; redoing the point once")
            continue
        return SimpleNamespace(run=run, result=result or {}, outcome=outcome, facts=facts, saved=saved, recorded=recorded,
                               pre=pre, before=before, started=started, redone=redone)


def build_record(p, progress, lines, copy, a):
    """The point's record from what the attempt left: the new transcript entries (kept in the work folder), the memory
    changes, the usage and every fact the store's check reads."""
    key, directory, result, facts = p["key"], memory_dir(p["key"]), a.result, a.facts
    forks = [f for f in copy.parent.glob("*.jsonl") if f != copy]
    after = lines_of(max(forks, key=lambda f: f.stat().st_mtime) if FORK and forks else copy)
    new = after[len(lines):] if after[: len(lines)] == lines else after
    out = REPLAY / key / p["sid"] / f"{p['n']}.jsonl"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text("".join(line + "\n" for line in new))
    now = digests(directory)
    status = "failed" if a.outcome != "ok" else "cut_off" if facts["cut_off"] else "ok"
    entries = parse_lines(new)
    flags = read_flags()
    streamed = stream_counts(a.run.stream)
    windows = [u.get("contextWindow") for u in (result.get("modelUsage") or {}).values() if isinstance(u.get("contextWindow"), int)]
    wrote = memory_writes(new, directory)
    return {
        "id": p["id"], "session": f"{p['repo']}/{p['sid']}", "key": key, "kind": p["kind"], "line": p["line"],
        "time": p["time"].isoformat(), "status": status, "failed_with": result.get("result") if status == "failed" else None,
        "api_error": api_error_kind(result), "api_error_status": result.get("api_error_status"), "result_subtype": result.get("subtype"),
        "redone": a.redone, "recorded_context": p["context"],
        "recorded": a.recorded, "recorded_files": sorted(k for k, v in a.before.items() if a.pre.get(k) != v),
        "files": sorted(k for k, v in now.items() if a.before.get(k) != v), "removed": sorted(set(a.before) - set(now)),
        "saved_paths": a.saved, "memory_saved_stream": streamed["memory_saved"], "api_retries": streamed["api_retry"],
        "copy_rewritten": after[: len(lines)] != lines, "prompt_first": prompt_first(entries), "prompt_variant": flags["prompt_variant"],
        "seconds": round(time.monotonic() - a.started), "num_turns": result.get("num_turns"),
        "permission_denials": len(result.get("permission_denials") or []), "usage": result.get("usage"), "modelUsage": result.get("modelUsage"),
        "total_cost_usd": result.get("total_cost_usd"), "models": sorted(result.get("modelUsage") or {}),
        "context_window": max(windows, default=None), "init": init_facts(a.run.init), "waited_on_limit": a.run.waited,
        "compacted_on_resume": sum(boundary(e) for e in entries),
        "continue_injected": sum(bool(e.get("isMeta")) and "Continue from where you left off" in json.dumps(e.get("message")) for e in entries),
        "usage_limit_notes": sum("usageLimitNote" in json.dumps(e) for e in entries),
        "new_kinds": dict(Counter(f"{e.get('type')}/{e['subtype']}" if e.get("subtype") else str(e.get("type")) for e in entries)),
        "last_reply": str(result.get("result") or "")[:1000],
        "flag_changes": {k: v for k, v in flags["values"].items() if v != progress["flags"]["values"].get(k)},
        "closing_turn_preempted": bool(wrote.get("main_writes", 0) + wrote.get("main_edits", 0) and facts.get("skipped_direct_write")),
        "extraction_usage": {"cache_read": facts.get("extraction_cache_read", 0), "cache_create": facts.get("extraction_cache_create", 0),
                             "input": facts.get("extraction_input", 0)},
    } | wrote | facts


def finish(p, progress, record):
    """Record the point: stop on a fault that would fail every point or on the account's identity, then append the
    record, save progress, and only then drop or rename the point's snapshots (a stop in between must find them)."""
    key, touched = p["key"], p["touch"]
    wrong = []
    if record["status"] != "failed":
        if not record["prompt_first"]:
            wrong.append("the closing prompt is not the first new user entry")
        if record["continue_injected"]:
            wrong.append("Claude Code added a meta continue message")
        if record["copy_rewritten"]:
            wrong.append("Claude Code rewrote the transcript copy it resumed")
        if record["prompt_variant"] != progress["flags"]["prompt_variant"]:
            wrong.append(f"the prompt variant changed from {progress['flags']['prompt_variant']} to {record['prompt_variant']} (tengu_moth_copse); "
                         "the store would mix two prompts: run again with --rebuild")
    if wrong:
        restore_all(touched)
        sys.exit(f"auto-memory: {p['id']}: {'; '.join(wrong)}; nothing was recorded for it")
    load_identity()
    hits = [h for k in touched for h in scan_dir(memory_dir(k), k)] + identity_hits("the point's record", record_text(record))
    if hits:  # restore nothing: the work folder stays for the user
        sys.exit(f"auto-memory: the account's identity is in {p['id']}: " + ", ".join(f"{field} in {name}" for name, field in hits[:10]) + "; the work folder is kept")
    clear_project(key)
    with POINTS.open("a") as f:
        f.write(json.dumps(record) + "\n")
        f.flush()
        os.fsync(f.fileno())
    progress["done"].append(p["id"])
    progress["in_flight"] = None
    add_usage(progress["usage"], record)
    # Three points in a row failing the same way are a systematic fault, not bad luck: undo them and stop.
    signature = str(record["api_error"] or record["api_error_status"] or record["result_subtype"])
    streak = progress["streak"]
    failed = record["status"] == "failed"
    if failed:
        same = streak["signature"] == signature
        progress["streak"] = {"signature": signature, "n": streak["n"] + 1 if same else 1,
                              "points": (streak["points"] if same else []) + [{"id": p["id"], "index": p["index"], "keys": touched}]}
        for k in touched:
            keep_snapshot(k, f"streak-{p['index']}-")
    else:
        progress["streak"] = {"signature": None, "n": 0, "points": []}
    save(progress)
    for k in touched:
        drop_snapshot(k)
    if not failed:
        drop_streak_snapshots()
    if progress["streak"]["n"] >= 3:
        undo_streak(progress)
        sys.exit(f"auto-memory: three points in a row failed with {signature}; they were undone, so a rerun redoes them")
    return record


def record_text(record):
    """The record as text, escaped and not: an identifier with accents must be found either way."""
    return json.dumps(record) + json.dumps(record, ensure_ascii=False)


def replay(p, progress):
    lines = lines_of(p["path"])[: p["line"] + 1]
    copy = CONFIG / "projects" / p["key"] / f"{p['sid']}.jsonl"
    for k in p["touch"]:
        snapshot(k)
    progress["in_flight"] = p["id"]
    save(progress)
    return finish(p, progress, build_record(p, progress, lines, copy, attempt_loop(p, progress, lines, copy)))


def undo_streak(progress):
    """Put the memory back as it was before the streak's first point and take the points off the done list and the
    records, with their usage."""
    for sp in reversed(progress["streak"]["points"]):
        for k in sp["keys"]:
            restore(k, f"streak-{sp['index']}-")
    gone = {sp["id"] for sp in progress["streak"]["points"]}
    progress["done"] = [i for i in progress["done"] if i not in gone]
    kept = [r for r in records() if r["id"] not in gone]
    atomic(POINTS, "".join(json.dumps(r) + "\n" for r in kept))
    progress["usage"] = {}
    for r in kept:
        add_usage(progress["usage"], r)
    progress["streak"] = {"signature": None, "n": 0, "points": []}
    progress["in_flight"] = None
    save(progress)
    drop_streak_snapshots()


def records():
    """The points' records; a line a hard kill cut short is left out, since its point is not done."""
    return parse_lines(POINTS.read_text().splitlines() if POINTS.exists() else (), keep_bad=False)


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


def packed_keys(keys):
    """Every project key the store holds by repository: the sessions' own, and under "(other)" any a recorded write named."""
    extra = {d.parent.name for d in (CONFIG / "projects").glob("*/memory") if d.parent.name != "warmup"} - {k for ks in keys.values() for k in ks}
    return keys | ({"(other)": extra} if extra else {})


def manifest(progress, skipped_files, skipped, pts, keys, expected):
    inputs = (WORK / "key.txt").read_text().splitlines()
    recs = records()

    def total(name):
        return sum(r.get(name) or 0 for r in recs)

    keys = packed_keys(keys)
    memory_files = {key: digests(memory_dir(key)) for ks in keys.values() for key in ks}
    deleted = sorted(f"{r['key']}/{f}" for r in recs for f in r["files"] + r["removed"] if f not in memory_files.get(r["key"], {}))
    usage = progress["usage"]
    kinds = Counter(r["kind"] for r in recs)
    changes = [{"point": r["id"], "flags": r["flag_changes"]} for r in recs if r["flag_changes"]]
    applied = {k: total_recorded(recs, k) for k in ("write", "edit", "bash", "recorded_failed")}
    skips = Counter()
    for r in recs:
        skips.update(r["recorded"]["skipped"])
    variant = progress["flags"]["prompt_variant"]
    return {
        "arm": "auto-memory",
        "key": inputs[0],
        "inputs": inputs[1:],
        "device": os.environ["BILBO_EVAL_DEVICE"],
        "versions": {"claude-code": subprocess.run([BIN, "--version"], env=claude_env("version"), capture_output=True, text=True).stdout.split()[0],
                     "binary_sha256": hashlib.sha256(Path(BIN).read_bytes()).hexdigest(), "arch": platform.machine(), "python": platform.python_version()},
        "config": [{"key": k, "value": v, "cite": cite} for k, v, cite in CONFIG_ENTRIES],
        "prompt": PROMPT,
        "auth": {"credential": (progress.get("auth") or {}).get("credential"), "credentials_seen": sorted({str(r["init"].get("apiKeySource")) for r in recs}),
                 "user_email_in_context": "emailAddress" in IDENTITY},
        "flags": progress["flags"] | {"warmups": progress["warmups"], "changes": changes},
        "notes": NOTES,
        "projects": {repo: {key: {"sessions": len({r["session"] for r in recs if r["key"] == key}), "points": sum(r["key"] == key for r in recs)}
                            | stats(memory_dir(key)) for key in sorted(ks)} for repo, ks in sorted(keys.items())},
        "sessions": {"replayed": len({r["session"] for r in recs}), "skipped": skipped_files},
        "points": {"planned": len(pts) + sum(skipped.values()), "replayed": len(recs), "skipped": skipped,
                   "failed": sum(r["status"] == "failed" for r in recs), "cut_off": sum(r["status"] == "cut_off" for r in recs),
                   "redone": total("redone"), "at_session_end": kinds["end"], "before_compaction": kinds["compact"], "retries": progress["retries"]},
        "recorded": {"in_plan": expected, "applied": applied, "skipped": dict(skips)},
        "context": {"max_recorded": max((r["recorded_context"] for r in recs), default=0), "over_200k": sum(r["recorded_context"] > 200_000 for r in recs),
                    "context_windows": sorted({r["context_window"] for r in recs if r["context_window"]}), "compacted_on_resume": total("compacted_on_resume")},
        "extraction": {"started": total("started"), "finished": total("finished"), "files_written": total("files_written"),
                       "memories_saved_debug": sum(len(r["saved_paths"]) for r in recs), "nothing_to_save": total("nothing"),
                       "skipped": {k: total("skipped_" + k) for k in ("direct_write", "no_prose", "rule")}, "errors": total("errors"),
                       "cut_off": total("cut_off"), "denied_tool_calls": total("denied"),
                       "memory_saved_stream": total("memory_saved_stream"), "memory_saved_transcript": total("memory_saved_transcript")},
        "main_model": {"memory_writes": total("main_writes"), "memory_edits": total("main_edits"), "turns": total("turns"), "num_turns": total("num_turns"),
                       "permission_denials": total("permission_denials"), "thinking_strips": total("strips"), "api_retries": total("api_retries"),
                       "closing_turn_preempted": sum(bool(r["closing_turn_preempted"]) for r in recs), "usage_limit_notes": total("usage_limit_notes"),
                       "permission_modes": dict(Counter(str(r["init"].get("permissionMode")) for r in recs))},
        "run_step": {"memory_dir": "memory/<key>/", "prompt_variant": variant, "topic_recall_bytes": 4096,
                     "loaded_at_start": "MEMORY.md, its first 200 lines or 25,000 characters" if variant == "index" else "MEMORY.md is still loaded when present (the applied recorded writes make some); the prompt no longer asks the model to keep it, and topic files come through the recall selector",
                     "recall_selector_flag": "tengu_moth_copse", "recall_selector_on_this_account": progress["flags"]["recall"],
                     "dates": "files Claude Code 2.1.296 writes in the build carry the build's date, not the session's, so a question about when something happened is biased against this arm; applied recorded files keep the dates their authors wrote (7 of the 17 hold one)"},
        "memory_files": memory_files,
        "deleted_later": deleted,
        "llm": {"model": MODEL, "calls": usage.get("calls", 0) + progress["warmup"].get("calls", 0)}
        | {k: usage.get(k, 0) + progress["warmup"].get(k, 0) for k in ("input_tokens", "output_tokens", "cache_read_input_tokens", "cache_creation_input_tokens")}
        | {"total_cost_usd": round(usage.get("total_cost_usd", 0) + progress["warmup"].get("total_cost_usd", 0), 6), "by_model": usage.get("by_model", {}),
           "extraction": {"cache_read_input_tokens": sum(r["extraction_usage"]["cache_read"] for r in recs),
                          "cache_creation_input_tokens": sum(r["extraction_usage"]["cache_create"] for r in recs),
                          "input_tokens": sum(r["extraction_usage"]["input"] for r in recs)},
           "warmup": progress["warmup"]},
        "replay_seconds": round(progress["seconds"]),
        "finished": datetime.now(timezone.utc).isoformat(timespec="seconds"),
    }


def total_recorded(recs, name):
    return sum(r["recorded"].get(name) or 0 for r in recs)


def write_store(progress, skipped_files, skipped, pts, keys, expected):
    out = WORK / "store"
    shutil.rmtree(out, ignore_errors=True)
    for key in sorted(k for ks in packed_keys(keys).values() for k in ks):
        (out / "memory" / key).mkdir(parents=True)
        if memory_dir(key).is_dir():
            shutil.copytree(memory_dir(key), out / "memory" / key, dirs_exist_ok=True)
    shutil.copy(POINTS, out / "points.jsonl")
    load_identity()
    text = json.dumps(manifest(progress, skipped_files, skipped, pts, keys, expected), indent=2) + "\n"
    hits = scan_dir(out, "") + identity_hits("manifest.json", text + json.dumps(json.loads(text), indent=2, ensure_ascii=False))
    if hits:
        shutil.rmtree(out)  # nothing is left to pack
        sys.exit("auto-memory: the account's identity is in the store, which was removed: " + ", ".join(f"{field} in {name.lstrip('/')}" for name, field in hits[:10]))
    (out / "manifest.json").write_text(text)
    failed = [r["id"] for r in records() if r["status"] == "failed"]
    log(f"done: {out}" + (f" ({len(failed)} points failed: {', '.join(failed)})" if failed else ""))


def interrupted(signum, frame):
    sys.exit("auto-memory: stopped; run again to resume from the last checkpoint")


def main():
    signal.signal(signal.SIGINT, interrupted)
    signal.signal(signal.SIGTERM, interrupted)
    given = [v for v in AUTH if os.environ.get(v)]
    if not given:
        sys.exit("auto-memory: set CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY; the build runs claude -p with it")
    if len(given) > 1:
        sys.exit(f"auto-memory: {' and '.join(given)} are both set; unset one: the build uses exactly one credential")
    HOME.mkdir(parents=True, exist_ok=True)
    version = subprocess.run([BIN, "--version"], env=claude_env("version"), capture_output=True, text=True).stdout.split()
    if version[:1] != [VERSION]:
        sys.exit(f"auto-memory: expected Claude Code {VERSION}, found {version}")
    for d in (CONFIG, DEBUG, REPLAY):
        d.mkdir(parents=True, exist_ok=True)

    plan, skipped_files = sessions()
    pts, skipped, keys, expected = points(plan, checkout_roots())
    if PROGRESS.exists():
        progress = json.loads(PROGRESS.read_text())
        if (progress.get("auth") or {}).get("variable") not in (None, given[0]):
            sys.exit(f"auto-memory: this build started with {progress['auth']['variable']} and is resumed with {given[0]}; use the same credential")
        if progress["in_flight"]:
            restore_all(next(p["touch"] for p in pts if p["id"] == progress["in_flight"]))
            progress["in_flight"] = None
            save(progress)  # before any snapshot is retaken, so a stop cannot find a snapshot-less point in flight
        if progress["streak"]["n"] >= 3:  # an undo a stop cut short
            undo_streak(progress)
        kept = [r for r in records() if r["id"] in set(progress["done"])]  # a record whose point never finished is dropped
        atomic(POINTS, "".join(json.dumps(r) + "\n" for r in kept))
        log(f"resuming after {len(progress['done'])} points")
    else:
        progress = {"done": [], "in_flight": None, "flags": {}, "warmups": 0, "warmup": {}, "usage": {}, "seconds": 0,
                    "retries": {"limit_pauses": 0, "transient": 0}, "streak": {"signature": None, "n": 0, "points": []}}
        POINTS.write_text("")
    started, base = time.monotonic(), progress["seconds"]
    warmup(progress)
    load_identity()
    try:
        done = set(progress["done"])
        for p in pts:
            if p["id"] in done:
                continue
            r = replay(p, progress)
            progress["seconds"] = base + time.monotonic() - started
            save(progress)
            log(f"{len(progress['done'])}/{len(pts)} {p['id']} ({p['kind']}): {r['status']}, {r['seconds']} s, "
                f"{len(r['files'])} memory files changed, extraction started {r.get('started', 0)}")
    finally:
        Child.kill()

    write_store(progress, skipped_files, skipped, pts, keys, expected)


if __name__ == "__main__":
    main()
