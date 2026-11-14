"""Search the bilbo stores unpacked in /work/store for the questions in /work/questions.jsonl: `bilbo recall` over the
question's repository's store (scope repo) or the one store (scope all), once for each form. A result is one hit of
recall's output, its text opening with the session's start time as the bm25 arm's `dated()` does. Appends one row per
question and form to /work/search.jsonl. build-store.sh --search runs it in the build service.

It reads no setting from the environment. `run(settings, store=, questions=, out=, binary=)` takes them: `budget`
(recall's --budget), `question_words` (None keeps the key's default; a list of words is written to a config file of its
own), `candidates` (None keeps the key's default; a number is passed as BILBO_RECALL_CANDIDATES) and `in` (None, or "repo":
each recall is scoped with --in <remembered folder>/<the question's repository>, and its `--in kept` line must count the
entries of that repository). Every row also holds `passed`, `run` (the store's key, the driver's 12 hex and the settings
in force) and, scoped, `in` and `kept`. The four paths
default to the container's, so a native run is the same code with other paths. Run as a script it uses DEFAULTS; the
comparison's variants are drivers (scripts/variants/<name>/drive.py) that call `run` with their own. `search.py loop`
checks the question loop with no store and no binary."""

import hashlib
import json
import re
import subprocess
import sys
import time
from pathlib import Path

STORE = Path("/work/store")
BILBO = "/usr/local/bin/bilbo"
VARIANT_CONFIG = Path("/tmp/bilbo-question-words.toml")
QUESTIONS = Path("/work/questions.jsonl")
OUT = Path("/work/search.jsonl")
HEADER = re.compile(r"^=== (([0-9A-HJKMNP-TV-Z]{10}|[0-9A-HJKMNP-TV-Z]{26})#r(\d+)(?:-(\d+))?)\t(\d+) lines?((?:\t[^\t\n]+)*)$")
PART = re.compile(r"^part (\d+)/(\d+)$")
SEARCHED = re.compile(r"^bilbo: searched [\d,]+ entr(?:y|ies)$")
PASSED_OVER = re.compile(r"^bilbo: passed over: .+$")
IN_KEPT = re.compile(r"^bilbo: --in kept ([\d,]+) entr(?:y|ies)( and left out (?:none|[\d,]+) of the best [\d,]+(?:: .+)?)?$")
PASSED_ITEMS = (("copies", re.compile(r"^([\d,]+) cop(?:y|ies)$")),
                ("parts", re.compile(r"^([\d,]+) further parts? of a record$")),
                ("capped", re.compile(r"^([\d,]+) over the entry cap$")))
DEFAULTS = {"budget": 8000, "question_words": None, "candidates": None, "in": None}


def environment(name, settings, store=STORE):
    d = store / "stores" / name
    env = {"HOME": f"{d}/home", "BILBO_HOME": f"{d}/root", "XDG_STATE_HOME": f"{d}/state",
           "XDG_CACHE_HOME": f"{d}/cache", "BILBO_CONFIG": f"{d}/config.toml", "PATH": "/usr/bin:/bin", "NO_COLOR": "1"}
    if settings["question_words"] is not None:
        env["BILBO_CONFIG"] = str(VARIANT_CONFIG)
    if settings["candidates"] is not None:
        env["BILBO_RECALL_CANDIDATES"] = str(settings["candidates"])
    return env


def parse(question, form, stdout):
    """The hits of one recall output: a header line, then exactly as many lines as it says, to the end."""
    lines = stdout.split("\n")
    if lines.pop() != "":
        sys.exit(f"search: {question}/{form}: stdout does not end with a line feed")
    hits, i = [], 0
    while i < len(lines):
        m = HEADER.match(lines[i])
        if not m:
            sys.exit(f"search: {question}/{form}: line {i + 1} is not a header: {lines[i][:200]!r}")
        ref, entry, a, b, n, extra = m.groups()
        n = int(n)
        body = lines[i + 1: i + 1 + n]
        if len(body) != n:
            sys.exit(f"search: {question}/{form}: the hit {ref} says {n} lines, {len(body)} are left")
        part, cut = None, False
        for field in extra.split("\t")[1:]:
            p = PART.match(field)
            if p:
                part = [int(p.group(1)) - 1, int(p.group(2))]
            elif field == "cut":
                cut = True
            else:
                sys.exit(f"search: {question}/{form}: the header of {ref} has the field {field!r}")
        hits.append({"ref": ref, "entry": entry, "lines": [int(a), int(b or a)], "part": part, "cut": cut, "text": "\n".join(body)})
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


def recall(question, form, name, text, settings, binary=BILBO, store=STORE, scope=None):
    """(hits, seconds, passed, kept): `scope` is the --in value, None for no scope; `kept` is the count its line printed."""
    began = time.monotonic()
    flags = ["--in", scope] if scope is not None else []
    done = subprocess.run([binary, "recall", *flags, "--budget", str(settings["budget"]), "--", text],
                          env=environment(name, settings, store), capture_output=True, check=False)
    seconds = time.monotonic() - began
    stdout, stderr = done.stdout.decode(), done.stderr.decode()
    err = stderr.split("\n")[:-1] if stderr.endswith("\n") else stderr.split("\n")
    if done.returncode == 1 and err and err[-1] == "bilbo: nothing matched":
        err = err[:-1]
        hits = []
    elif done.returncode == 0:
        hits = parse(question, form, stdout)
    else:
        sys.exit(f"search: {question}/{form}: recall exited {done.returncode}: {stderr.strip()[:500]}")
    kept, passed = None, {key: 0 for key, _ in PASSED_ITEMS}
    stray = []
    for line in err:
        m = IN_KEPT.match(line) if scope is not None else None
        if m:
            kept = int(m.group(1).replace(",", ""))
        elif PASSED_OVER.match(line):
            passed = passed_over(question, form, line)
        elif not SEARCHED.match(line):
            stray.append(line)
    if stray:
        sys.exit(f"search: {question}/{form}: stderr: {stray[0][:300]}")
    if scope is not None and kept is None:
        sys.exit(f"search: {question}/{form}: --in {scope} printed no `--in kept` line")
    return hits, seconds, passed, kept


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


def run(settings, store=STORE, questions=QUESTIONS, out=OUT, binary=BILBO):
    unknown = sorted(set(settings) - set(DEFAULTS))
    if unknown:
        sys.exit(f"search: unknown setting {unknown[0]!r}; settings: {', '.join(DEFAULTS)}")
    settings = DEFAULTS | settings
    if settings["in"] not in (None, "repo"):
        sys.exit(f"search: in must be None or \"repo\", not {settings['in']!r}")
    words = settings["question_words"]
    if words is not None and not (isinstance(words, list) and all(isinstance(w, str) for w in words)):
        sys.exit(f"search: question_words must be None or a list of strings, not {words!r}")
    if words is not None:
        quoted = ", ".join(json.dumps(w, ensure_ascii=False) for w in words)
        VARIANT_CONFIG.write_text(f"follow = []\n\n[recall]\nquestion_words = [{quoted}]\n", encoding="utf-8")
    manifest = json.loads((store / "manifest.json").read_text())
    store_scope = manifest["scope"]
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
                hits, seconds, passed, kept = recall(q["id"], form, name, q[form], settings, binary=binary, store=store, scope=scope)
                found = [result(entries, q["id"], form, name, rank, h) for rank, h in enumerate(hits, 1)]
                if scope is not None:
                    if kept != per_repo.get((name, q["repo"]), 0):
                        sys.exit(f"search: {q['id']}/{form}: --in kept {kept} entries, entries.jsonl has {per_repo.get((name, q['repo']), 0)} of {q['repo']}")
                    other = [r for r in found if r["repo"] != q["repo"]]
                    if other:
                        sys.exit(f"search: {q['id']}/{form}: a result of {other[0]['repo']}, the question's repo is {q['repo']}")
                row = {"id": q["id"], "form": form, "seconds": round(seconds, 3), "results": found, "passed": passed,
                       "run": run_id}
                if scope is not None:
                    row |= {"in": scope, "kept": kept}
                sink.write(json.dumps(row, ensure_ascii=False) + "\n")
                sink.flush()
            print(f"search: {i}/{len(rows)} {q['id']}", file=sys.stderr)


def loop():
    import tempfile

    def stand_in(question, form, name, text, settings, binary=BILBO, store=STORE, scope=None):
        seen.append((question, form, name, text, scope))
        return [], 0.0, {key: 0 for key, _ in PASSED_ITEMS}, (1 if scope is not None else None)

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


if __name__ == "__main__":
    if sys.argv[1:] == ["loop"]:
        sys.exit(loop())
    run(DEFAULTS)
