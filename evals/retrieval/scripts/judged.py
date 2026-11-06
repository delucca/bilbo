"""The judge's verdicts as scoring reads them. A context's verdict is the last judged row for the hash of the full judge
prompt that judge.py builds from the current question and that context, so a verdict given for other facts is never
used. judge.py is a script: its prompt builder is read from its source, up to the argument parser."""

import hashlib
import json
from collections import defaultdict
from pathlib import Path

BUDGET = 4000


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
