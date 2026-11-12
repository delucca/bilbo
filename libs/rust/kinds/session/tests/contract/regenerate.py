"""The contract sample of the Claude dialect: what build.py's own render_session prints, kept beside the lines
it printed it from. build.py is the bm25 arm's script of the retrieval eval, which lives outside this tree:
every run names it.

    python3 -I regenerate.py --build <build.py>                write expected.jsonl from edges.jsonl and sample.jsonl
    python3 -I regenerate.py --build <build.py> --check        fail when expected.jsonl is not what build.py prints now
    python3 -I regenerate.py --build <build.py> --corpus <folder>   pick sample.jsonl again from the unpacked eval
                                                   corpus (<folder>/<repo>/*.jsonl), then write expected.jsonl
    python3 -I regenerate.py --build <build.py> --whole <folder> <out>   write what build.py prints for every
                                                   session of the corpus, for the ignored whole-corpus test
    --out <folder>                                 with --corpus: write sample.jsonl and expected.jsonl there,
                                                   to compare with the ones kept here
    python3 -I regenerate.py --prefaces --corpus <folder>   write prefaces.jsonl: for each session of sample.jsonl,
                                                   the first truthy timestamp of the session's file in the corpus and the
                                                   line the eval's search.py (started, dated) puts before its chunks;
                                                   needs no build.py; with --check, fail when the file is not what that gives
    ... --whole <folder> <out> also writes <out>'s name with .prefaces.jsonl for it: one row a session of the corpus,
                                                   with its file

edges.jsonl is written by hand: one line per branch of the render that the corpus does not reach. Every line
of both files must be one build.py renders; a line it stops on belongs in the reader's unit tests.
build.py is loaded as it is, its two search imports answered by empty modules."""

import importlib.util
import json
import pathlib
import sys
import tempfile
import types
from datetime import datetime, timezone

HERE = pathlib.Path(__file__).resolve().parent
LIMIT = 6000  # bytes: a sampled line is no longer than this,
LONGEST = 40000  # or than this when its branch has no shorter line: then the branch gets that one line
EACH = 2  # lines picked per branch


def load(path):
    sys.dont_write_bytecode = True  # no __pycache__ beside build.py
    for name in ("bm25s", "Stemmer"):
        sys.modules.setdefault(name, types.ModuleType(name))
    spec = importlib.util.spec_from_file_location("bm25_build", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def rendered(build, name, text):
    """build.py's messages for one session's lines, each as an expected.jsonl row."""
    with tempfile.TemporaryDirectory() as folder:
        path = pathlib.Path(folder) / "session.jsonl"
        path.write_bytes(text)
        messages, _ = build.render_session(path)
    return [{"file": name, "line": m["line"], "text": m["text"], "heads": [list(h) for h in m["heads"]]} for m in messages]


def branches(entry):
    """The branches of render_session and render_entry one entry takes."""
    kind = entry.get("type")
    role = entry.get("role") if kind is None else kind
    if kind is None:
        yield "cursor entry"
    if role not in ("user", "assistant"):
        yield f"other entry {kind}/{entry.get('subtype')}"
        return
    if entry.get("isSidechain"):
        yield "sidechain"
        return
    if entry.get("isMeta"):
        yield "meta"
        return
    if entry.get("isCompactSummary"):
        yield "compact summary"
    content = (entry.get("message") or {}).get("content")
    if isinstance(content, str):
        yield f"{role} string content"
        if content != content.strip():
            yield "text to strip"
        if not content.strip():
            yield "empty after strip"
        return
    if not isinstance(content, list):
        yield "content of another type"
        return
    kinds = [b.get("type") if isinstance(b, dict) else None for b in content]
    if len([k for k in kinds if k in ("text", "tool_use", "tool_result", "image", "document")]) > 1:
        yield "several pieces"
    for block, kind in zip(content, kinds):
        if kind == "text":
            yield f"{role} text block"
            if block.get("text", "") != block.get("text", "").strip():
                yield "text to strip"
            if not block.get("text", "").strip():
                yield "empty after strip"
            if not block.get("text", "").isascii():
                yield "text that is not ASCII"
        elif kind == "tool_use":
            given = block.get("input")
            if not given:
                yield "tool_use with no input"
            elif not isinstance(given, dict):
                yield "tool_use input that is no object"
            else:
                for value in given.values():
                    yield f"input value {type(value).__name__}"
                    if isinstance(value, (dict, list)) and json.dumps(value) != json.dumps(value, ensure_ascii=False):
                        yield "nested input that is not ASCII"
                    if isinstance(value, (dict, list)) and "\\" in json.dumps(value, ensure_ascii=False):
                        yield "nested input with an escape"
        elif kind == "tool_result":
            yield "tool_error" if block.get("is_error") else "tool_result"
            inner = block.get("content")
            if isinstance(inner, str):
                yield "result string"
                if not inner.strip():
                    yield "result empty after strip"
            elif isinstance(inner, list):
                for item in inner:
                    yield f"result item {item.get('type') if isinstance(item, dict) else None}"
            else:
                yield f"result content {type(inner).__name__}"
        elif kind in ("image", "document"):
            yield f"{kind} block"
        else:
            yield f"skipped block {kind}"


def pick(corpus):
    """For each branch its EACH shortest corpus lines, and with a result the call that names its tool. A session's
    picked lines stay in their order, so a result still follows its call. A branch whose every line is over
    LONGEST (an image) is left to edges.jsonl."""
    best = {}
    sessions = sorted(corpus.glob("*/*.jsonl"))
    for path in sessions:
        name = f"{path.parent.name}/{path.name}"
        for number, raw in enumerate(path.open("rb"), 1):
            if len(raw) > LONGEST:
                continue
            if not raw.strip():
                found = ["blank line"]
            else:
                found = set(branches(json.loads(raw)))
            for branch in found:
                rows = best.setdefault(branch, [])
                rows.append((len(raw), name, number))
                rows.sort()
                del rows[EACH:]
    chosen = {}
    for rows in best.values():
        short = [row for row in rows if row[0] <= LIMIT] or rows[:1]
        for _, name, number in short:
            chosen.setdefault(name, set()).add(number)
    sample = []
    for name in sorted(chosen):
        wanted, calls, lines = chosen[name], {}, []
        for number, raw in enumerate((corpus / name).open("rb"), 1):
            entry = json.loads(raw) if raw.strip() else {}
            content = (entry.get("message") or {}).get("content") if isinstance(entry, dict) else None
            blocks = [b for b in content if isinstance(b, dict)] if isinstance(content, list) else []
            for b in blocks:
                if b.get("type") == "tool_use" and len(raw) <= LIMIT:
                    calls[b.get("id")] = number
            if number in wanted:
                for b in blocks:
                    if b.get("type") == "tool_result" and b.get("tool_use_id") in calls:
                        wanted.add(calls[b["tool_use_id"]])
        for number, raw in enumerate((corpus / name).open("rb"), 1):
            if number in wanted:
                lines.append(raw.decode())
        sample.append({"file": name, "lines": lines})
    return sample, sorted(best)


def preface(path):
    """search.py's started(): the first truthy timestamp of the session's file and the line it makes."""
    for line in path.open(encoding="utf-8"):
        stamp = json.loads(line).get("timestamp") if line.strip() else None
        if stamp:
            when = datetime.fromisoformat(stamp).astimezone(timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
            return {"timestamp": stamp, "preface": f"Session started {when}:"}
    return {"timestamp": None, "preface": "Session date unknown:"}


def prefaces(corpus, check):
    out = HERE / "prefaces.jsonl"
    rows = [preface(corpus / json.loads(line)["file"]) for line in (HERE / "sample.jsonl").open(encoding="utf-8")]
    text = "".join(json.dumps(row, ensure_ascii=False) + "\n" for row in rows)
    if check:
        if out.read_text(encoding="utf-8") != text:
            sys.exit("prefaces.jsonl is not what the eval's search.py gives for sample.jsonl")
        print(f"prefaces.jsonl: {len(rows)} sessions, as the eval gives them")
    else:
        out.write_text(text, encoding="utf-8")
        print(f"prefaces.jsonl: {len(rows)} sessions written")


def main():
    args = sys.argv[1:]
    check = "--check" in args
    if "--prefaces" in args:
        if "--corpus" not in args:
            sys.exit("regenerate.py --prefaces needs --corpus <the unpacked eval corpus>")
        return prefaces(pathlib.Path(args[args.index("--corpus") + 1]), check)
    if "--build" not in args:
        sys.exit("regenerate.py needs --build <path to the eval's build.py>")
    build = load(args[args.index("--build") + 1])
    if "--whole" in args:
        corpus, out = pathlib.Path(args[args.index("--whole") + 1]), args[args.index("--whole") + 2]
        sessions = sorted(corpus.glob("*/*.jsonl"))
        with open(out, "w", encoding="utf-8") as f:
            count = 0
            for path in sessions:
                for row in rendered(build, f"{path.parent.name}/{path.name}", path.read_bytes()):
                    f.write(json.dumps(row, ensure_ascii=False) + "\n")
                    count += 1
        print(f"{out}: {len(sessions)} sessions, {count} messages")
        whole = pathlib.Path(out).with_suffix(".prefaces.jsonl")
        with open(whole, "w", encoding="utf-8") as f:
            for path in sessions:
                row = {"file": f"{path.parent.name}/{path.name}", **preface(path)}
                f.write(json.dumps(row, ensure_ascii=False) + "\n")
        print(f"{whole}: {len(sessions)} sessions")
        return
    out = pathlib.Path(args[args.index("--out") + 1]) if "--out" in args else HERE
    if "--corpus" in args:
        sample, found = pick(pathlib.Path(args[args.index("--corpus") + 1]))
        text = "".join(json.dumps(row, ensure_ascii=False) + "\n" for row in sample)
        (out / "sample.jsonl").write_text(text, encoding="utf-8")
        print(f"sample.jsonl: {len(sample)} sessions, {sum(len(r['lines']) for r in sample)} lines, {len(found)} branches")
        for branch in found:
            print(f"  {branch}")
    rows = rendered(build, "edges", (HERE / "edges.jsonl").read_bytes())
    for line in (out / "sample.jsonl").open(encoding="utf-8"):
        session = json.loads(line)
        rows += rendered(build, session["file"], "".join(session["lines"]).encode())
    text = "".join(json.dumps(row, ensure_ascii=False) + "\n" for row in rows)
    path = out / "expected.jsonl"
    if check:
        if path.read_text(encoding="utf-8") != text:
            sys.exit("expected.jsonl is not what build.py prints for edges.jsonl and sample.jsonl")
        print(f"expected.jsonl: {len(rows)} messages, as build.py prints them")
    else:
        path.write_text(text, encoding="utf-8")
        print(f"expected.jsonl: {len(rows)} messages written")


main()
