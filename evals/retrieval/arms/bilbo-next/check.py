"""Check the bilbo stores unpacked in /work/store: the manifest's sessions are the corpus's (as many as the limit says),
entries.jsonl has one row a session with distinct ids in a store, and every store answers `bilbo recall`: exit 0, its
stderr counts the entries the build's own recall counted (`indexed` in the manifest) and holds no other line than that,
`passed over` and `typos:`, every header is followed by its lines, every ref of the probe is a short one unless another entry of the store ends alike, and `bilbo read` takes the first; a query of stopwords only exits 1 with `bilbo: nothing matched`; and `recall --in <remembered>/<repo>` for the store's first
repository exits 0 with `bilbo: --in kept N entries`, N the repository's rows of entries.jsonl. build-store.sh --check runs it in the build
service. A store whose `noted` is over 0 also answers a probe with `--kind note` with the field `note` on every hit, prints
no note with BILBO_RECALL_NOTES_BUDGET=0, and its `--in kept` counts a repository's note entries too."""

import json
import re
import subprocess
import sys
import unicodedata
from datetime import datetime
from pathlib import Path

STORE = Path("/work/store")
CORPUS = Path("/corpus")
BILBO = "/usr/local/bin/bilbo"
HEADER = re.compile(r"^=== (([0-9A-HJKMNP-TV-Z]{10}|[0-9A-HJKMNP-TV-Z]{26})#r(\d+)(?:-(\d+))?)\t(\d+) lines?(?:\t(?!cut(?:\t|$))([a-z]+))?(?:\tpart \d+/\d+)?(?:\tcut)?$")
SEARCHED = re.compile(r"^bilbo: searched ([\d,]+) entr(?:y|ies)$")
PASSED_OVER = re.compile(r"^bilbo: passed over: .+$")
TYPOS = re.compile(r"^bilbo: typos: .+$")
IN_KEPT = re.compile(r"^bilbo: --in kept ([\d,]+) entr(?:y|ies)( and left out (?:none|[\d,]+) of the best [\d,]+(?:: .+)?)?$")


def has_notes(manifest, name):
    return manifest["stores"][name].get("noted", 0) > 0


def stamp(path):
    for line in path.open(encoding="utf-8"):
        entry = json.loads(line) if line.strip() else {}
        if entry.get("timestamp"):
            return datetime.fromisoformat(entry["timestamp"])
    return None


def chosen(manifest):
    """The sessions the build chose: the order of the bm25 arm, cut at the limit in the manifest's inputs."""
    found, other = [], []
    for path in sorted(CORPUS.glob("*/*.jsonl")):
        start = stamp(path)
        (found if start else other).append((start, path.parent.name, path.name))
    found.sort()
    limit = next(i.split()[1] for i in manifest["inputs"] if i.startswith("sessions "))
    return [(repo, name) for _, repo, name in found + other][: None if limit == "all" else int(limit)]


def environment(name, store=STORE):
    d = store / "stores" / name
    return {"HOME": f"{d}/home", "BILBO_HOME": f"{d}/root", "XDG_STATE_HOME": f"{d}/state",
            "XDG_CACHE_HOME": f"{d}/cache", "BILBO_CONFIG": f"{d}/config.toml", "PATH": "/usr/bin:/bin", "NO_COLOR": "1"}


def run(name, *args, binary=BILBO, store=STORE, env=None):
    done = subprocess.run([binary, *args], env=environment(name, store) | (env or {}), capture_output=True, check=False)
    try:
        out = done.stdout.decode()
    except UnicodeDecodeError:
        out = None
    return done.returncode, out, done.stderr.decode()


REORDERS = set("\u061c\u200e\u200f\u202a\u202b\u202c\u202d\u202e\u2066\u2067\u2068\u2069")


def word(text):
    if text and not text.startswith("=") and all(c.isascii() and (c.isalnum() or c in "/._-+:@%,=") for c in text):
        return text
    return "'" + text.replace("'", "'\\''") + "'"


def more_line(budget, words, scope=None, kind=None):
    query = " ".join(words)
    if any(unicodedata.category(c) == "Cc" or c in REORDERS for c in query + (scope or "")):
        return "more: the same command with --page 2"
    flags = (f" --in={word(scope)}" if scope is not None else "") + (f" --kind {word(kind)}" if kind else "")
    return f"more: bilbo recall --page 2{flags} --budget {word(str(budget))} -- {word(query)}"


def parsed(stdout, more=None):
    if stdout is None:
        return None
    lines = stdout.split("\n")
    if lines.pop() != "":
        return None
    found, i = [], 0
    while i < len(lines):
        if found and more is not None and i == len(lines) - 1 and lines[i] == more:
            break
        m = HEADER.match(lines[i])
        if not m or i + int(m.group(5)) >= len(lines):
            return None
        found.append((m.group(1), m.group(6)))
        i += 1 + int(m.group(5))
    return found


def headers(stdout, more=None):
    found = parsed(stdout, more)
    return None if found is None else [ref for ref, _ in found]


def scoped_probe(code, out, err, scope, want):
    kept = [m for m in map(IN_KEPT.match, err.splitlines()) if m]
    found = headers(out, more_line(4000, ["user", "assistant"], scope)) if code == 0 else None
    return code == 0 and len(kept) == 1 and int(kept[0].group(1).replace(",", "")) == want and bool(found)


def main(binary=BILBO, store=STORE):
    manifest = json.loads((store / "manifest.json").read_text())
    rows = [json.loads(line) for line in (store / "entries.jsonl").open(encoding="utf-8")]
    problems = []

    def bad(msg):
        problems.append(msg)
        print("check:", msg)

    want = chosen(manifest)
    got = [(r["repo"], r["session"]) for r in rows]
    print("sessions", manifest["sessions"], "of", len(want), "| entries", len(rows), "| entries as the manifest counts",
          manifest["entries"])
    if manifest["sessions"] != len(want) or manifest["entries"] != len(rows):
        bad("the manifest's sessions or entries differ from the corpus and the limit")
    if sorted(got) != sorted(want) or len(set(got)) != len(got):
        bad("entries.jsonl is not one row for each chosen session")
    if manifest["scope"] not in ("all", "repo"):
        bad(f"scope {manifest['scope']!r}")
    by_store = {}
    for r in rows:
        by_store.setdefault(r["store"], []).append(r)
    if sorted(by_store) != sorted(manifest["stores"]):
        bad("the stores of entries.jsonl are not the manifest's")
    if manifest["scope"] == "repo" and any({r["repo"] for r in rs} != {name} for name, rs in by_store.items()):
        bad("a store of scope repo holds another repository")
    for name, rs in by_store.items():
        ids = [r["id"] for r in rs]
        if len(set(ids)) != len(ids) or not all(re.fullmatch(r"[0-9A-Z]{26}", i) for i in ids):
            bad(f"{name}: entry ids are not 26 distinct characters")
        if manifest["stores"][name]["entries"] != len(rs):
            bad(f"{name}: the manifest counts {manifest['stores'][name]['entries']} entries, entries.jsonl {len(rs)}")
        if not manifest["stores"][name]["chunks"]:
            bad(f"{name}: no chunk")


    for name in sorted(manifest["stores"]):
        code, out, err = run(name, "recall", "--budget", "4000", "--", "user", "assistant", binary=binary, store=store)
        count = [m for m in map(SEARCHED.match, err.splitlines()) if m]
        found = headers(out, more_line(4000, ["user", "assistant"])) if code == 0 else None
        other = [line for line in err.splitlines() if not (SEARCHED.match(line) or PASSED_OVER.match(line) or TYPOS.match(line))]
        ok = (code == 0 and not other and len(count) == 1
              and int(count[0].group(1).replace(",", "")) == manifest["stores"][name]["indexed"] and bool(found))
        if not ok:
            bad(f"{name}: recall exited {code}; stderr {err.strip()[:300]!r}; hits {found and len(found)}")
            continue
        ids = [r["id"] for r in by_store[name]]
        for ref in found:
            whole = ref.split("#")[0]
            if len(whole) == 26 and not any(i != whole and i.endswith(whole[-10:]) for i in ids):
                bad(f"{name}: the ref {ref} prints a whole id no other entry's id ends like")
        read_code, _, read_err = run(name, "read", found[0], "--budget", "256", binary=binary, store=store)
        if read_code != 0:
            bad(f"{name}: read {found[0]} exited {read_code}: {read_err.strip()[:300]}")
        repo = by_store[name][0]["repo"]
        derived = manifest.get("derive", {}).get("repos", {})
        want = sum(r["repo"] == repo for r in by_store[name]) + derived.get(repo, {}).get("note_entries", 0)
        code, out, err = run(name, "recall", "--in", f"{manifest.get('remembered', '')}/{repo}", "--budget", "4000", "--", "user", "assistant",
                             binary=binary, store=store)
        scope = f"{manifest.get('remembered', '')}/{repo}"
        if "remembered" not in manifest or not scoped_probe(code, out, err, scope, want):
            bad(f"{name}: recall --in {repo} exited {code}; stderr {err.strip()[:300]!r}; the repository has {want} entries")
        code, out, err = run(name, "recall", "--", "the", "and", "of", "to", binary=binary, store=store)
        if code != 1 or out != "" or not err.splitlines() or err.splitlines()[-1] != "bilbo: nothing matched":
            bad(f"{name}: a query of stopwords exited {code}; stderr {err.strip()[:300]!r}")
        if has_notes(manifest, name):
            code, out, err = run(name, "recall", "--kind", "note", "--budget", "4000", "--", "user", "assistant", binary=binary, store=store)
            notes = parsed(out, more_line(4000, ["user", "assistant"], kind="note")) if code == 0 else None
            if code != 0 or not notes or any(kind != "note" for _, kind in notes):
                bad(f"{name}: recall --kind note exited {code}; stderr {err.strip()[:300]!r}; hits {notes}")
            code, out, err = run(name, "recall", "--budget", "4000", "--", "user", "assistant", binary=binary, store=store,
                                 env={"BILBO_RECALL_NOTES_BUDGET": "0"})
            shown = parsed(out, more_line(4000, ["user", "assistant"])) if code == 0 else None
            if code != 0 or not shown or any(kind == "note" for _, kind in shown):
                bad(f"{name}: recall with BILBO_RECALL_NOTES_BUDGET=0 exited {code} or printed a note: {shown}")
        kept = manifest["stores"][name]
        print(f"store {name}: {kept['entries']} entries, {kept['indexed']} searched, {kept['entries'] - kept['indexed']} "
              f"with no chunk, {len(found)} hits for the probe, first {found[0]}")

    print("FAILED" if problems else "OK")
    return 1 if problems else 0


HEADER_TAKEN = ["", "note", "part 1/2", "cut", "note\tpart 1/2", "note\tcut", "part 1/2\tcut", "note\tpart 1/2\tcut"]
HEADER_REFUSED = ["note\tcut\tpart 1/2\tcut", "part 1/2\tpart 2/2", "cut\tcut", "note\tnote", "cut\tpart 1/2",
                  "part 1/2\tnote", "cut\tnote", "Note", "note\t", "note\tcut\tpart 1/2"]


def trailer_lines():
    ref = "2T6W9YB0DE#r1"
    hit = f"=== {ref}\t1 line\nbody\n"
    plain = more_line(4000, ["user", "assistant"], "/r/a", "note")
    escaped = more_line(4000, ["carrot\nrows"])
    if plain != "more: bilbo recall --page 2 --in=/r/a --kind note --budget 4000 -- 'user assistant'" or escaped != "more: the same command with --page 2":
        print(f"lines: the trailers read {plain!r} and {escaped!r}")
        return 1
    if more_line(4000, ["=ls"]) != "more: bilbo recall --page 2 --budget 4000 -- '=ls'":
        print(f"lines: a query that starts with = reads {more_line(4000, ['=ls'])!r}")
        return 1
    if headers(f"{hit}{plain}\n", plain) != [ref] or headers(f"{hit}{escaped}\n", escaped) != [ref]:
        print("lines: a trailer for the arguments was refused")
        return 1
    if headers(f"=== {ref}\t2 lines\nbody\n{plain}\n", plain) != [ref]:
        print("lines: a hit whose last counted line is a trailer's text was refused")
        return 1
    for why, out in (("another query", f"{hit}{more_line(4000, ['other'], '/r/a', 'note')}\n"),
                     ("another page", f"{hit}{plain.replace('--page 2', '--page 3')}\n"),
                     ("one byte more", f"{hit}{plain} \n"), ("no hit before it", f"{plain}\n"),
                     ("two trailers", f"{hit}{plain}\n{plain}\n"), ("a line after it", f"{hit}{plain}\nmore\n")):
        if headers(out, plain) is not None:
            print(f"lines: a trailer was accepted with {why}")
            return 1
    if headers(f"{hit}{plain}\n") is not None:
        print("lines: a trailer was accepted where none is expected")
        return 1
    import os
    import tempfile

    with tempfile.TemporaryDirectory() as tmp:
        program = Path(tmp) / "recall.sh"
        program.write_text("#!/bin/sh\nprintf '\\377'\necho 'bilbo: --in kept 3 entries' >&2\nexit 0\n")
        os.chmod(program, 0o755)
        for printed in ("\\377", "=== 2T6W9YB0DE#r1\\t1 line\\n\\377\\n"):
            program.write_text(f"#!/bin/sh\nprintf '{printed}'\necho 'bilbo: --in kept 3 entries' >&2\nexit 0\n")
            code, out, err = run("n", "recall", binary=str(program), store=Path(tmp))
            if scoped_probe(code, out, err, "/r/a", 3):
                print(f"lines: the scoped probe took standard output with a byte that is no UTF-8: {printed}")
                return 1
    own = more_line(4000, ["user", "assistant"], "/r/a")
    kept = "bilbo: --in kept 3 entries\n"
    for why, out, taken in (("a hit and its own trailer", f"{hit}{own}\n", True),
                            ("a trailer for another query", f"{hit}{more_line(4000, ['other'], '/r/a')}\n", False),
                            ("the trailer without its --in", f"{hit}{more_line(4000, ['user', 'assistant'])}\n", False),
                            ("no hit", "", False)):
        if scoped_probe(0, out, kept, "/r/a", 3) != taken:
            print(f"lines: the scoped probe's judgement of {why} is not {taken}")
            return 1
    return 0


def lines():
    if trailer_lines():
        return 1
    ref = "2T6W9YB0DE#r1"
    for fields in HEADER_TAKEN:
        if headers(f"=== {ref}\t1 line{chr(9) + fields if fields else ''}\nbody\n") != [ref]:
            print(f"lines: the header fields {fields!r} were refused")
            return 1
    for fields in HEADER_REFUSED:
        if headers(f"=== {ref}\t1 line\t{fields}\nbody\n") is not None:
            print(f"lines: the header fields {fields!r} were accepted")
            return 1
    print("lines: OK")
    return 0


if __name__ == "__main__":
    if sys.argv[1:] == ["lines"]:
        sys.exit(lines())
    sys.exit(main())
