"""The judge's verdicts as scoring reads them. A context's verdict is the last judged row for the hash of the full judge
prompt that judge.py builds from the current question and that context, so a verdict given for other facts is never
used. judge.py is a script: its prompt builder is read from its source, up to the argument parser."""

import hashlib
import json
import re
import sys
from collections import defaultdict
from pathlib import Path

BUDGET = 4000
PARTS = ("arm", "store", "search", "driver", "reworded", "variation", "rerun")
RUN = re.compile(r"(?P<arm>.+)-(?P<store>[0-9a-f]{12})-(?P<search>[0-9a-f]{12})(?:-driver-(?P<driver>[0-9a-f]{12}))?"
                 r"(?:-reworded-(?P<reworded>[0-9a-f]{12}))?(?:-variation-(?P<variation>[0-9a-f]{12}))?"
                 r"(?:-rerun-(?P<rerun>[a-z0-9]+))?")


def run_parts(name):
    m = RUN.fullmatch(name)
    if not m:
        sys.exit(f"judged: {name!r} is not <arm>-<store>-<search>[-driver-<d>][-reworded-<r>][-variation-<v>][-rerun-<label>]")
    return tuple(m[p] for p in PARTS)


def rows(path):
    return [json.loads(line) for line in path.read_text().split("\n") if line.strip()] if path.exists() else []


def prompt_builder():
    source = (Path(__file__).parent / "judge.py").read_text()
    scope = {"__name__": "judge_prompt"}
    exec(source[:source.index("ap = argparse.ArgumentParser()")], scope)
    return scope["build_prompt"]


def verdicts(d, questions):
    """(id, form) -> the verdict in d/judged.jsonl for the current prompt of each of d's contexts, `questions` by id."""
    build, judged = prompt_builder(), defaultdict(list)
    for r in rows(d / "judged.jsonl"):
        judged[(r["id"], r["form"], r["ctx"], r["prompt"])].append(r)
    out = {}
    for c in rows(d / f"contexts-{BUDGET}.jsonl"):
        if c["id"] in questions:
            sha = hashlib.sha256(build(questions[c["id"]], c["text"]).encode()).hexdigest()
            if judged[(c["id"], c["form"], c["sha"], sha)]:
                out[(c["id"], c["form"])] = judged[(c["id"], c["form"], c["sha"], sha)][-1]
    return out


def grade(q, v):
    present = [f["id"] for f in q["facts"] if v["facts"][f["id"]]]
    stale = q.get("stale", [])
    misled = any(v["stale"][s["id"]] and not v["facts"][s["replaces"]] for s in stale) if stale else None
    return {"score": len(present) / len(q["facts"]), "present": present, "facts": len(q["facts"]), "misled": misled}


def names():
    a, b, c, d = "0123456789ab", "ba9876543210", "abcdef012345", "fedcba543210"
    table = [
        (f"bm25-{a}-{b}", ("bm25", a, b, None, None, None, None)),
        (f"bilbo-next-{a}-{b}", ("bilbo-next", a, b, None, None, None, None)),
        (f"bilbo-next-{a}-{b}-driver-{c}", ("bilbo-next", a, b, c, None, None, None)),
        (f"bilbo-next-{a}-{b}-variation-{c}", ("bilbo-next", a, b, None, None, c, None)),
        (f"bilbo-next-{a}-{b}-rerun-two", ("bilbo-next", a, b, None, None, None, "two")),
        (f"bilbo-next-{a}-{b}-driver-{c}-reworded-{d}-variation-{a}-rerun-x1",
         ("bilbo-next", a, b, c, d, a, "x1")),
        (f"a-b-c-{a}-{b}-driver-{c}", ("a-b-c", a, b, c, None, None, None)),
        (f"bilbo-next-{a}", None),
        (f"bilbo-next-{a}-{b}-driver-{c}x", None),
        (f"bilbo-next-{a}-{b}-driver-abc", None),
        (f"bilbo-next-{a}-{b}-rerun-Two", None),
        (f"bilbo-next-{a}-{b}-variation-{c}-driver-{d}", None),
    ]
    for name, want in table:
        try:
            got = run_parts(name)
        except SystemExit:
            got = None
        if got != want:
            print(f"names: {name}: got {got}, wanted {want}")
            return 1
    print("names: OK")
    return 0


if __name__ == "__main__":
    if sys.argv[1:] != ["names"]:
        sys.exit("usage: judged.py names")
    sys.exit(names())
