"""Search the bilbo stores unpacked in /work/store for the questions in /work/questions.jsonl: `bilbo recall` over the
question's repository's store (scope repo) or the one store (scope all), once for each form. A result is one hit of
recall's output, its text opening with the session's start time as the bm25 arm's `dated()` does. Appends one row per
question and form to /work/search.jsonl. build-store.sh --search runs it in the build service.

It reads no setting from the environment. `run(settings)` takes them: `budget` (recall's --budget), `question_words`
(None keeps the key's default; a list of words is written to a config file of its own) and `candidates` (None keeps the
key's default; a number is passed as BILBO_RECALL_CANDIDATES). Run as a script it uses DEFAULTS; the comparison's
variants are drivers (scripts/variants/<name>/drive.py) that call `run` with their own."""

import json
import re
import subprocess
import sys
import time
from pathlib import Path

STORE = Path("/work/store")
BILBO = "/usr/local/bin/bilbo"
VARIANT_CONFIG = Path("/tmp/bilbo-question-words.toml")
HEADER = re.compile(r"^=== (bilbo:([0-9A-Z]{26})#r(\d+)(?:-(\d+))?)\t([a-z]+)\t(\d{4}-\d{2}-\d{2})\t(\d+) lines?((?:\t[^\t\n]+)*)$")
PART = re.compile(r"^part (\d+)/(\d+)$")
SEARCHED = re.compile(r"^bilbo: searched [\d,]+ entr(?:y|ies)$")
DEFAULTS = {"budget": 4000, "question_words": None, "candidates": None}


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
        ref, entry, a, b, kind, date, n, extra = m.groups()
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
        hits.append({"ref": ref, "entry": entry, "lines": [int(a), int(b or a)], "part": part, "kind": kind, "date": date,
                     "cut": cut, "text": "\n".join(body)})
        i += 1 + n
    return hits


def recall(question, form, name, text, settings, binary=BILBO, store=STORE):
    began = time.monotonic()
    done = subprocess.run([binary, "recall", "--budget", str(settings["budget"]), "--", text],
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
    stray = [line for line in err if not SEARCHED.match(line)]
    if stray:
        sys.exit(f"search: {question}/{form}: stderr: {stray[0][:300]}")
    return hits, seconds


def result(entries, question, form, name, rank, hit):
    entry = entries.get((name, hit["entry"]))
    if not entry:
        sys.exit(f"search: {question}/{form}: {hit['ref']} is not an entry of the store {name}")
    return {"text": hit["text"], "ref": hit["ref"], "entry": hit["entry"], "repo": entry["repo"],
            "session": entry["session"], "lines": hit["lines"], "part": hit["part"], "kind": hit["kind"],
            "date": hit["date"], "rank": rank, "cut": hit["cut"]}


def run(settings):
    unknown = sorted(set(settings) - set(DEFAULTS))
    if unknown:
        sys.exit(f"search: unknown setting {unknown[0]!r}; settings: {', '.join(DEFAULTS)}")
    settings = DEFAULTS | settings
    words = settings["question_words"]
    if words is not None and not (isinstance(words, list) and all(isinstance(w, str) for w in words)):
        sys.exit(f"search: question_words must be None or a list of strings, not {words!r}")
    if words is not None:
        quoted = ", ".join(json.dumps(w, ensure_ascii=False) for w in words)
        VARIANT_CONFIG.write_text(f"follow = []\n\n[recall]\nquestion_words = [{quoted}]\n", encoding="utf-8")
    manifest = json.loads((STORE / "manifest.json").read_text())
    scope = manifest["scope"]
    entries = {}
    for line in (STORE / "entries.jsonl").open(encoding="utf-8"):
        row = json.loads(line)
        entries[row["store"], row["id"]] = row
    questions = [json.loads(line) for line in Path("/work/questions.jsonl").read_text().split("\n") if line.strip()]
    with open("/work/search.jsonl", "a") as out:
        for i, q in enumerate(questions, 1):
            name = q["repo"] if scope == "repo" else "all"
            if name not in manifest["stores"]:
                sys.exit(f"search: {q['id']}: the manifest has no store {name}")
            for form in ("query", "prompt"):
                hits, seconds = recall(q["id"], form, name, q[form], settings)
                rows = [result(entries, q["id"], form, name, rank, h) for rank, h in enumerate(hits, 1)]
                out.write(json.dumps({"id": q["id"], "form": form, "seconds": round(seconds, 3), "results": rows},
                                     ensure_ascii=False) + "\n")
                out.flush()
            print(f"search: {i}/{len(questions)} {q['id']}", file=sys.stderr)


if __name__ == "__main__":
    run(DEFAULTS)
