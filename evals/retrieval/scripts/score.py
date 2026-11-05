"""Score the judged contexts. Usage: score.py <selected.jsonl> <classes.json> <run-dir> <gold-dir> <random-dir>.
A question's score is the share of its facts present; misled is a stale item present while the fact that replaced it is
absent. The headline weights the class means by classes.json."""

import json
import sys
from collections import defaultdict
from pathlib import Path

BUDGET = 4000
TOKENIZER = "tiktoken o200k_base"
JUDGE = "claude-opus-5-5"
FORMS = ("query", "prompt")

selected, classes_file, run, gold, rand = sys.argv[1:6]
run, gold, rand = Path(run), Path(gold), Path(rand)
questions = [json.loads(line) for line in Path(selected).read_text().splitlines() if line.strip()]
weights = {c["name"]: c["weight"] for c in json.loads(Path(classes_file).read_text())["classes"]}


def rows(path):
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()] if path.exists() else []


def verdicts(d):
    """(id, form) -> last judged row whose ctx is the current context."""
    judged = defaultdict(list)
    for r in rows(d / "judged.jsonl"):
        judged[(r["id"], r["form"], r["ctx"])].append(r)
    return {(c["id"], c["form"]): judged[(c["id"], c["form"], c["sha"])][-1]
            for c in rows(d / f"contexts-{BUDGET}.jsonl") if judged[(c["id"], c["form"], c["sha"])]}


def grade(q, v):
    present = [f["id"] for f in q["facts"] if v["facts"][f["id"]]]
    stale = q.get("stale", [])
    misled = any(v["stale"][s["id"]] and not v["facts"][s["replaces"]] for s in stale) if stale else None
    return {"score": len(present) / len(q["facts"]), "present": present, "facts": len(q["facts"]), "misled": misled}


arm, controls = verdicts(run), {"gold": verdicts(gold), "random": verdicts(rand)}
missing = [f"{q['id']}/{f}" for q in questions for f in FORMS if (q["id"], f) not in arm] + \
          [f"{n}:{q['id']}" for n, t in controls.items() for q in questions if (q["id"], "both") not in t]
if missing:
    sys.exit(f"score: no verdict for {', '.join(missing)}")

graded = [{"id": q["id"], "class": q["class"], "repo": q["repo"], "form": f, **grade(q, arm[(q["id"], f)])}
          for q in questions for f in FORMS]
ctl = {n: {q["id"]: grade(q, t[(q["id"], "both")]) for q in questions} for n, t in controls.items()}


def mean(xs):
    return sum(xs) / len(xs) if xs else None


by_class = {}
for name in weights:
    qs = [q for q in questions if q["class"] == name]
    if not qs:
        continue
    cell = {"n": len(qs), "weight": weights[name]}
    for f in FORMS:
        g = [r for r in graded if r["class"] == name and r["form"] == f]
        flags = [r["misled"] for r in g if r["misled"] is not None]
        cell[f] = mean([r["score"] for r in g])
        cell[f"misled_{f}"] = mean(flags)
        cell[f"stale_n_{f}"] = len(flags)
    for n in ctl:
        cell[n] = mean([ctl[n][q["id"]]["score"] for q in qs])
    by_class[name] = cell


def headline(key):
    return sum(c["weight"] * c[key] for c in by_class.values()) / sum(c["weight"] for c in by_class.values())


head = {k: headline(k) for k in (*FORMS, *ctl)}
misled = {}
for f in FORMS:
    flags = [r["misled"] for r in graded if r["form"] == f and r["misled"] is not None]
    misled[f] = {"misled": sum(flags), "of": len(flags)}

arm_name, store = run.name.rsplit("-", 1)
(run / "scores.jsonl").write_text("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in graded))
(run / "score.json").write_text(json.dumps({
    "arm": arm_name, "store": store, "budget": BUDGET, "tokenizer": TOKENIZER, "judge": JUDGE, "questions": len(questions),
    "partial": len(by_class) < len(weights), "headline": head, "classes": by_class, "gold": ctl["gold"], "random": ctl["random"],
    "misled": misled}, indent=2) + "\n")


def pct(x):
    return "-" if x is None else f"{100 * x:.0f}%"


lines = []
if len(by_class) < len(weights):
    lines.append(f"partial: {len(by_class)} of {len(weights)} classes, weights renormalised")
lines += ["", "| Class | n | weight | query | prompt | misled q | misled p | gold | random |", "|---|---:|---:|---:|---:|---:|---:|---:|---:|"]
for name, c in by_class.items():
    lines.append(f"| {name} | {c['n']} | {c['weight']} | {pct(c['query'])} | {pct(c['prompt'])} | "
                 f"{pct(c['misled_query'])} ({c['stale_n_query']}) | {pct(c['misled_prompt'])} ({c['stale_n_prompt']}) | "
                 f"{pct(c['gold'])} | {pct(c['random'])} |")
mq, mp = misled["query"], misled["prompt"]
lines.append(f"| **headline** | {len(questions)} | {sum(c['weight'] for c in by_class.values())} | **{pct(head['query'])}** | "
             f"{pct(head['prompt'])} | {mq['misled']}/{mq['of']} | {mp['misled']}/{mp['of']} | {pct(head['gold'])} | {pct(head['random'])} |")
text = "\n".join(lines) + "\n"
(run / "score.md").write_text(text)
print(text, end="")
