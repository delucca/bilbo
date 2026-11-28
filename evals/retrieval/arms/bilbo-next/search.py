"""Search the bilbo stores unpacked in /work/store for the questions in /work/questions.jsonl: `bilbo recall` over the
question's repository's store (scope repo) or the one store (scope all), once for each form. A result is one hit of
recall's output, its text opening with the session's start time as the bm25 arm's `dated()` does. Appends one row per
question and form to /work/search.jsonl. build-store.sh --search runs it in the build service.

It reads no setting from the environment. `run(settings, store=, questions=, out=, binary=)` takes them: `budget`
(recall's --budget), `question_words` (None keeps the key's default; a list of words is written to a config file of its
own), `candidates` (None keeps the key's default; a number is passed as BILBO_RECALL_CANDIDATES), `typo_terms` (None keeps the key's
default; a number is passed as BILBO_RECALL_TYPO_TERMS), `pair_weight` (None keeps the key's default; a number, 0 included, is passed as
BILBO_RECALL_PAIR_WEIGHT), `notes_budget` (the same, as BILBO_RECALL_NOTES_BUDGET) and `in` (None, or "repo":
each recall is scoped with --in <remembered folder>/<the question's repository>, and its `--in kept` line must count the
entries of that repository). Every row also holds `passed`, `typos` (`[[term, [near, ...]], ...]` from recall's `typos:` line,
empty without one), `run` (the store's key, the driver's 12 hex and the settings
in force) and, scoped, `in` and `kept`. The four paths
default to the container's, so a native run is the same code with other paths. Run as a script it uses DEFAULTS; the
comparison's variants are drivers (scripts/variants/<name>/drive.py) that call `run` with their own. `search.py loop`
checks the question loop with no store and no binary; `search.py lines` checks the `typos:` line's parser."""

import hashlib
from decimal import Decimal
import json
import re
import subprocess
import sys
import time
import unicodedata
from pathlib import Path

STORE = Path("/work/store")
BILBO = "/usr/local/bin/bilbo"
VARIANT_CONFIG = Path("/tmp/bilbo-question-words.toml")
QUESTIONS = Path("/work/questions.jsonl")
OUT = Path("/work/search.jsonl")
HEADER = re.compile(r"^=== (([0-9A-HJKMNP-TV-Z]{10}|[0-9A-HJKMNP-TV-Z]{26})#r(\d+)(?:-(\d+))?)\t(\d+) lines?(?:\t(?!cut(?:\t|$))([a-z]+))?(?:\tpart (\d+)/(\d+))?(\tcut)?$")
SEARCHED = re.compile(r"^bilbo: searched [\d,]+ entr(?:y|ies)$")
PASSED_OVER = re.compile(r"^bilbo: passed over: .+$")
TYPOS = re.compile(r"^bilbo: typos: (.+)$")
TYPO_ITEM = re.compile(r"^([a-z]+) as ([a-z]+(?:, [a-z]+)*)$")
IN_KEPT = re.compile(r"^bilbo: --in kept ([\d,]+) entr(?:y|ies)( and left out (?:none|[\d,]+) of the best [\d,]+(?:: .+)?)?$")
SOURCE = r"[0-9A-HJKMNP-TV-Z]{26}#g(\d+):r(\d+)(?:-(\d+))?"
NOTE_LINE = re.compile(rf"^({SOURCE})(?: {SOURCE})*(?: \d{{4}}-\d{{2}}-\d{{2}})?$")
PASSED_ITEMS = (("copies", re.compile(r"^([\d,]+) cop(?:y|ies)$")),
                ("parts", re.compile(r"^([\d,]+) further parts? of a record$")),
                ("capped", re.compile(r"^([\d,]+) over the entry cap$")))
DEFAULTS = {"budget": 8000, "question_words": None, "candidates": None, "typo_terms": None, "pair_weight": None, "notes_budget": None, "in": None}


def environment(name, settings, store=STORE):
    d = store / "stores" / name
    env = {"HOME": f"{d}/home", "BILBO_HOME": f"{d}/root", "XDG_STATE_HOME": f"{d}/state",
           "XDG_CACHE_HOME": f"{d}/cache", "BILBO_CONFIG": f"{d}/config.toml", "PATH": "/usr/bin:/bin", "NO_COLOR": "1"}
    if settings["question_words"] is not None:
        env["BILBO_CONFIG"] = str(VARIANT_CONFIG)
    if settings["candidates"] is not None:
        env["BILBO_RECALL_CANDIDATES"] = str(settings["candidates"])
    if settings["typo_terms"] is not None:
        env["BILBO_RECALL_TYPO_TERMS"] = str(settings["typo_terms"])
    if settings["pair_weight"] is not None:
        env["BILBO_RECALL_PAIR_WEIGHT"] = format(Decimal(str(settings["pair_weight"])), "f")
    if settings["notes_budget"] is not None:
        env["BILBO_RECALL_NOTES_BUDGET"] = str(settings["notes_budget"])
    return env


def source(line):
    m = NOTE_LINE.match(line)
    if not m:
        return None
    parent, generation = line[:26], int(m.group(2))
    first = int(m.group(3))
    return {"parent": parent, "generation": generation, "first": first, "last": int(m.group(4) or first)}


REORDERS = set("\u061c\u200e\u200f\u202a\u202b\u202c\u202d\u202e\u2066\u2067\u2068\u2069")


def word(text):
    if text and not text.startswith("=") and all(c.isascii() and (c.isalnum() or c in "/._-+:@%,=") for c in text):
        return text
    return "'" + text.replace("'", "'\\''") + "'"


def more_line(budget, text, scope=None):
    if any(unicodedata.category(c) == "Cc" or c in REORDERS for c in text + (scope or "")):
        return "more: the same command with --page 2"
    flags = f" --in={word(scope)}" if scope is not None else ""
    return f"more: bilbo recall --page 2{flags} --budget {word(str(budget))} -- {word(text)}"


def parse(question, form, stdout, more=None):
    lines = stdout.split("\n")
    if lines.pop() != "":
        sys.exit(f"search: {question}/{form}: stdout does not end with a line feed")
    hits, i = [], 0
    while i < len(lines):
        if hits and more is not None and i == len(lines) - 1 and lines[i] == more:
            break
        m = HEADER.match(lines[i])
        if not m:
            sys.exit(f"search: {question}/{form}: line {i + 1} is not a header: {lines[i][:200]!r}")
        ref, entry, a, b, n, kind, k, parts, cut = m.groups()
        n = int(n)
        body = lines[i + 1: i + 1 + n]
        if len(body) != n:
            sys.exit(f"search: {question}/{form}: the hit {ref} says {n} lines, {len(body)} are left")
        part = [int(k) - 1, int(parts)] if k else None
        hits.append({"ref": ref, "entry": entry, "lines": [int(a), int(b or a)], "part": part, "cut": cut is not None, "kind": kind, "text": "\n".join(body)})
        i += 1 + n
    return hits


def passed_over(question, form, line):
    counts = {key: 0 for key, _ in PASSED_ITEMS}
    for item in line.removeprefix("bilbo: passed over: ").split(", "):
        for key, pattern in PASSED_ITEMS:
            m = pattern.match(item)
            if m:
                counts[key] = int(m.group(1).replace(",", ""))
                break
        else:
            sys.exit(f"search: {question}/{form}: passed over: an item the arm does not know: {item[:200]!r}")
    return counts


def typos(question, form, line):
    found = []
    for item in TYPOS.match(line).group(1).split("; "):
        m = TYPO_ITEM.match(item)
        if not m:
            sys.exit(f"search: {question}/{form}: typos: an item the arm does not know: {item[:200]!r}")
        found.append([m.group(1), m.group(2).split(", ")])
    return found


def recall(question, form, name, text, settings, binary=BILBO, store=STORE, scope=None):
    """(hits, seconds, passed, kept, typos): `scope` is the --in value, None for no scope; `kept` is the count its line printed."""
    began = time.monotonic()
    flags = ["--in", scope] if scope is not None else []
    done = subprocess.run([binary, "recall", *flags, "--budget", str(settings["budget"]), "--", text],
                          env=environment(name, settings, store), capture_output=True, check=False)
    seconds = time.monotonic() - began
    stderr = done.stderr.decode()
    err = stderr.split("\n")[:-1] if stderr.endswith("\n") else stderr.split("\n")
    if done.returncode == 1 and err and err[-1] == "bilbo: nothing matched":
        if done.stdout:
            try:
                shown = done.stdout.decode()[:200]
            except UnicodeDecodeError:
                shown = done.stdout[:200]
            sys.exit(f"search: {question}/{form}: recall said nothing matched and printed {shown!r}")
        err = err[:-1]
        hits = []
    elif done.returncode == 0:
        try:
            stdout = done.stdout.decode()
        except UnicodeDecodeError:
            sys.exit(f"search: {question}/{form}: standard output is no UTF-8: {done.stdout[:200]!r}")
        hits = parse(question, form, stdout, more_line(settings["budget"], text, scope))
    else:
        sys.exit(f"search: {question}/{form}: recall exited {done.returncode}: {stderr.strip()[:500]}")
    kept, passed = None, {key: 0 for key, _ in PASSED_ITEMS}
    stray, misspelt = [], []
    for line in err:
        m = IN_KEPT.match(line) if scope is not None else None
        if m:
            kept = int(m.group(1).replace(",", ""))
        elif PASSED_OVER.match(line):
            passed = passed_over(question, form, line)
        elif TYPOS.match(line):
            misspelt = typos(question, form, line)
        elif not SEARCHED.match(line):
            stray.append(line)
    if stray:
        sys.exit(f"search: {question}/{form}: stderr: {stray[0][:300]}")
    if scope is not None and kept is None:
        sys.exit(f"search: {question}/{form}: --in {scope} printed no `--in kept` line")
    return hits, seconds, passed, kept, misspelt


def result(entries, question, form, name, rank, hit):
    """The row of one hit; a ref of 10 characters names the one entry of the store whose id ends with them."""
    ref = hit["entry"]
    if len(ref) == 26:
        found = [entries[name, ref]] if (name, ref) in entries else []
    else:
        found = [e for (store, i), e in entries.items() if store == name and i.endswith(ref)]
    if len(found) > 1:
        sys.exit(f"search: {question}/{form}: the ref {hit['ref']} ends {len(found)} entries of the store {name}; bilbo must print the whole id")
    if not found:
        sys.exit(f"search: {question}/{form}: {hit['ref']} is not an entry of the store {name}")
    entry = found[0]
    return {"text": hit["text"], "ref": hit["ref"], "entry": entry["id"], "repo": entry["repo"],
            "session": entry["session"], "lines": hit["lines"], "part": hit["part"], "rank": rank, "cut": hit["cut"]}


def note(entries, question, form, name, rank, hit):
    found = source(hit["text"].split("\n", 1)[0])
    if found is None:
        sys.exit(f"search: {question}/{form}: the first line of the note {hit['ref']} is no source line: {hit['text'][:200]!r}")
    entry = entries.get((name, found["parent"]))
    if entry is None:
        sys.exit(f"search: {question}/{form}: the note {hit['ref']} was written from {found['parent']}, which is no entry of the store {name}")
    return {"text": hit["text"], "ref": hit["ref"], "lines": hit["lines"], "cut": hit["cut"], "rank": rank, "entry": found["parent"],
            "repo": entry["repo"], "session": entry["session"], "generation": found["generation"],
            "records": [found["first"], found["last"]]}


def run(settings, store=STORE, questions=QUESTIONS, out=OUT, binary=BILBO):
    unknown = sorted(set(settings) - set(DEFAULTS))
    if unknown:
        sys.exit(f"search: unknown setting {unknown[0]!r}; settings: {', '.join(DEFAULTS)}")
    settings = DEFAULTS | settings
    if settings["in"] not in (None, "repo"):
        sys.exit(f"search: in must be None or \"repo\", not {settings['in']!r}")
    weight = settings["pair_weight"]
    if weight is not None and (isinstance(weight, bool) or not isinstance(weight, (int, float))):
        sys.exit(f"search: pair_weight must be None or a number, not {weight!r}")
    words = settings["question_words"]
    if words is not None and not (isinstance(words, list) and all(isinstance(w, str) for w in words)):
        sys.exit(f"search: question_words must be None or a list of strings, not {words!r}")
    if words is not None:
        quoted = ", ".join(json.dumps(w, ensure_ascii=False) for w in words)
        VARIANT_CONFIG.write_text(f"follow = []\n\n[recall]\nquestion_words = [{quoted}]\n", encoding="utf-8")
    manifest = json.loads((store / "manifest.json").read_text())
    store_scope = manifest["scope"]
    note_entries = {repo: info["note_entries"] for repo, info in manifest.get("derive", {}).get("repos", {}).items()}
    entries, per_repo = {}, {}
    for line in (store / "entries.jsonl").open(encoding="utf-8"):
        row = json.loads(line)
        entries[row["store"], row["id"]] = row
        per_repo[row["store"], row["repo"]] = per_repo.get((row["store"], row["repo"]), 0) + 1
    if settings["in"] and "remembered" not in manifest:
        sys.exit("search: the manifest has no `remembered`: the store was built before the scoped arm")
    script = Path(sys.argv[0])
    driver = hashlib.sha256(script.read_bytes()).hexdigest()[:12] if script.name == "drive.py" else None
    run_id = {"store": manifest["key"], "driver": driver, "settings": settings}
    rows = [json.loads(line) for line in Path(questions).read_text().split("\n") if line.strip()]
    with open(out, "a") as sink:
        for i, q in enumerate(rows, 1):
            name = q["repo"] if store_scope == "repo" else "all"
            if name not in manifest["stores"]:
                sys.exit(f"search: {q['id']}: the manifest has no store {name}")
            for form in ("query", "prompt"):
                scope = f"{manifest['remembered']}/{q['repo']}" if settings["in"] else None
                hits, seconds, passed, kept, misspelt = recall(q["id"], form, name, q[form], settings, binary=binary, store=store, scope=scope)
                chunks = [h for h in hits if h["kind"] != "note"]
                found = [result(entries, q["id"], form, name, rank, h) for rank, h in enumerate(chunks, 1)]
                notes = [note(entries, q["id"], form, name, rank, h) for rank, h in enumerate((h for h in hits if h["kind"] == "note"), 1)]
                if scope is not None:
                    want = per_repo.get((name, q["repo"]), 0) + note_entries.get(q["repo"], 0)
                    if kept != want:
                        sys.exit(f"search: {q['id']}/{form}: --in kept {kept} entries, entries.jsonl has {per_repo.get((name, q['repo']), 0)} sessions of {q['repo']} and the manifest {note_entries.get(q['repo'], 0)} note entries")
                    other = [r for r in found + notes if r["repo"] != q["repo"]]
                    if other:
                        sys.exit(f"search: {q['id']}/{form}: a result of {other[0]['repo']}, the question's repo is {q['repo']}")
                row = {"id": q["id"], "form": form, "seconds": round(seconds, 3), "results": found, "notes": notes, "passed": passed,
                       "typos": misspelt, "run": run_id}
                if scope is not None:
                    row |= {"in": scope, "kept": kept}
                sink.write(json.dumps(row, ensure_ascii=False) + "\n")
                sink.flush()
            print(f"search: {i}/{len(rows)} {q['id']}", file=sys.stderr)


def loop():
    import tempfile

    def stand_in(question, form, name, text, settings, binary=BILBO, store=STORE, scope=None):
        seen.append((question, form, name, text, scope))
        return [], 0.0, {key: 0 for key, _ in PASSED_ITEMS}, (1 if scope is not None else None), []

    seen = []
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        stores = {"alpha": {"entries": 1}, "beta": {"entries": 1}}
        (root / "manifest.json").write_text(json.dumps({"scope": "repo", "stores": stores, "key": "loop",
                                                        "remembered": "/r"}))
        (root / "entries.jsonl").write_text("".join(
            json.dumps({"store": n, "id": n.upper().ljust(26, "0"), "repo": n, "session": "s"}) + "\n" for n in stores))
        questions = root / "questions.jsonl"
        questions.write_text("".join(json.dumps({"id": f"q-{n}", "repo": n, "query": f"q {n}", "prompt": f"p {n}"}) + "\n" for n in stores))
        real = globals()["recall"]
        globals()["recall"] = stand_in
        try:
            for settings in ({}, {"in": "repo"}):
                seen.clear()
                run(settings, store=root, questions=questions, out=root / "out.jsonl")
                expected = [(f"q-{n}", form, n, f"{form[0]} {n}", f"/r/{n}" if settings else None)
                            for n in stores for form in ("query", "prompt")]
                if seen != expected:
                    print(f"loop: {settings}: searched {seen}, expected {expected}")
                    return 1
        except SystemExit as stop:
            print(f"loop: {stop}")
            return 1
        finally:
            globals()["recall"] = real
    print("loop: OK")
    return 0


HEADER_TAKEN = [("", (None, None, False)), ("note", ("note", None, False)), ("part 1/2", (None, [0, 2], False)),
                ("cut", (None, None, True)), ("note\tpart 1/2", ("note", [0, 2], False)), ("note\tcut", ("note", None, True)),
                ("part 1/2\tcut", (None, [0, 2], True)), ("note\tpart 1/2\tcut", ("note", [0, 2], True))]
HEADER_REFUSED = ["note\tcut\tpart 1/2\tcut", "part 1/2\tpart 2/2", "cut\tcut", "note\tnote", "cut\tpart 1/2",
                  "part 1/2\tnote", "cut\tnote", "Note", "note\t", "note\tcut\tpart 1/2"]


def header_lines():
    ref = "2T6W9YB0DE#r1"
    for fields, want in HEADER_TAKEN:
        out = f"=== {ref}\t1 line{chr(9) + fields if fields else ''}\nbody\n"
        got = parse("q", "query", out)
        found = (got[0]["kind"], got[0]["part"], got[0]["cut"]) if len(got) == 1 else None
        if found != want:
            print(f"lines: the header fields {fields!r} gave {found}, expected {want}")
            return 1
    for fields in HEADER_REFUSED:
        try:
            parse("q", "query", f"=== {ref}\t1 line\t{fields}\nbody\n")
        except SystemExit:
            continue
        print(f"lines: the header fields {fields!r} were accepted")
        return 1
    return 0


def trailer_lines():
    ref = "2T6W9YB0DE#r1"
    hit = f"=== {ref}\t1 line\nbody\n"
    plain, escaped = more_line(8000, "carrot rows", "/r/a"), more_line(8000, "carrot\nrows")
    if plain != "more: bilbo recall --page 2 --in=/r/a --budget 8000 -- 'carrot rows'" or escaped != "more: the same command with --page 2":
        print(f"lines: the trailers read {plain!r} and {escaped!r}")
        return 1
    if more_line(8000, "=ls") != "more: bilbo recall --page 2 --budget 8000 -- '=ls'":
        print(f"lines: a query that starts with = reads {more_line(8000, '=ls')!r}")
        return 1
    for query, scope, line in (("carrot rows", "/r/a", plain), ("carrot\nrows", None, escaped), ("it's", None, more_line(8000, "it's"))):
        got = parse("q", "query", f"{hit}{line}\n", more_line(8000, query, scope))
        if len(got) != 1 or got[0]["text"] != "body":
            print(f"lines: the trailer {line!r} was not taken for the arguments")
            return 1
    inside = parse("q", "query", f"=== {ref}\t2 lines\nbody\n{plain}\n", plain)
    if len(inside) != 1 or inside[0]["text"] != f"body\n{plain}":
        print("lines: a hit whose last counted line is a trailer's text was not read as the hit's")
        return 1
    refused = [("another query", f"{hit}{more_line(8000, 'other', '/r/a')}\n"),
               ("another page", f"{hit}{plain.replace('--page 2', '--page 3')}\n"),
               ("one byte more", f"{hit}{plain} \n"),
               ("no hit before it", f"{plain}\n"),
               ("two trailers", f"{hit}{plain}\n{plain}\n"),
               ("a line after it", f"{hit}{plain}\nmore\n")]
    for why, out in refused:
        try:
            parse("q", "query", out, plain)
        except SystemExit:
            continue
        print(f"lines: a trailer was accepted with {why}")
        return 1
    try:
        parse("q", "query", f"{hit}{plain}\n")
    except SystemExit:
        return 0
    print("lines: a trailer was accepted where none is expected")
    return 1


def no_match_lines():
    import os
    import tempfile

    rows = (
        ("", 1, "bilbo: nothing matched", None),
        ("x", 1, "bilbo: nothing matched", "'x'"),
        ("more: bilbo recall --page 2 -- q\\n", 1, "bilbo: nothing matched", "'more: bilbo recall --page 2 -- q\\n'"),
        ("\\377", 1, "bilbo: nothing matched", "b'\\xff'"),
        ("=== 2T6W9YB0DE#r1\\t1 line\\n\\377\\n", 0, "bilbo: searched 1 entry", "b'=== 2T6W9YB0DE#r1\\t1 line\\n\\xff\\n'"),
    )
    with tempfile.TemporaryDirectory() as tmp:
        for printed, code, said, shown in rows:
            program = Path(tmp) / "recall.sh"
            program.write_text(f"#!/bin/sh\nprintf '%b' '{printed}'\necho '{said}' >&2\nexit {code}\n")
            os.chmod(program, 0o755)
            try:
                got = recall("q", "query", "n", "text", DEFAULTS, binary=str(program), store=Path(tmp))
                message = None
            except SystemExit as stop:
                got, message = None, str(stop)
            if shown is None:
                if got is None or got[0] != []:
                    print(f"lines: a no-match run that printed nothing gave {got}, {message}")
                    return 1
            elif got is not None or not message.startswith("search: q/query: ") or shown not in message or (shown[0] != "b" and "b'" in message):
                print(f"lines: a run that printed {printed!r} (exit {code}) was judged {got}, {message!r}")
                return 1
    return 0


def lines():
    if header_lines() or trailer_lines() or no_match_lines():
        return 1
    table = [("bilbo: typos: carot as carrot", [["carot", ["carrot"]]]),
             ("bilbo: typos: carot as carrot; setin as set, setton", [["carot", ["carrot"]], ["setin", ["set", "setton"]]])]
    for line, expected in table:
        if typos("q", "query", line) != expected:
            print(f"lines: {line!r} gave {typos('q', 'query', line)}, expected {expected}")
            return 1
    for line in ("bilbo: typos: carot2 as carrot", "bilbo: typos: carot as carrot2", "bilbo: typos: carot as carrot; x1 as y"):
        try:
            typos("q", "query", line)
        except SystemExit:
            continue
        print(f"lines: {line!r} was accepted")
        return 1
    for key, variable, cases in (("typo_terms", "BILBO_RECALL_TYPO_TERMS", ((0, "0"), (3, "3"), (None, None))),
                                 ("pair_weight", "BILBO_RECALL_PAIR_WEIGHT", ((0, "0"), (0.25, "0.25"), (0.00005, "0.00005"), (10, "10"), (None, None)))):
        for value, want in cases:
            got = environment("n", DEFAULTS | {key: value}).get(variable)
            if got != want:
                print(f"lines: {key} {value!r} gave the variable {got!r}, expected {want!r}")
                return 1
    for value, want in ((0, "0"), (2500, "2500"), (None, None)):
        got = environment("n", DEFAULTS | {"notes_budget": value}).get("BILBO_RECALL_NOTES_BUDGET")
        if got != want:
            print(f"lines: notes_budget {value!r} gave the variable {got!r}, expected {want!r}")
            return 1
    for value in (True, False):
        try:
            run({"pair_weight": value}, store=Path("/nonexistent"))
        except SystemExit as stop:
            if str(stop) != f"search: pair_weight must be None or a number, not {value!r}":
                print(f"lines: pair_weight {value!r} stopped with {stop}")
                return 1
            continue
        print(f"lines: pair_weight {value!r} was accepted")
        return 1
    print("lines: OK")
    return 0


if __name__ == "__main__":
    if sys.argv[1:] == ["loop"]:
        sys.exit(loop())
    if sys.argv[1:] == ["lines"]:
        sys.exit(lines())
    run(DEFAULTS)
