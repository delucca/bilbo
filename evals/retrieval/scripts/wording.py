"""Compare each arm's run on the original questions with its run on the reworded ones. Usage:
wording.py <reworded.jsonl> <classes.json> <out.json> <arm>=<original-run>,<reworded-run> ...
The first arm is the one tested: its drop is set against every other arm's. A drop is the original headline minus the
reworded one, per form, with classes weighted as score.py weights them; the 95% intervals come from 10,000 paired
resamples of the questions within each class. Overlap is the share of the original's terms, tokenized as the bm25 arm
tokenizes, that the reworded text still holds."""

import json
import random
import sys
from pathlib import Path

import bm25s
import Stemmer

FORMS = ("query", "prompt")
RESAMPLES = 10_000
SEED = 20261105

reworded_file, classes_file, out = sys.argv[1:4]
weights = {c["name"]: c["weight"] for c in json.loads(Path(classes_file).read_text())["classes"]}
rows = [json.loads(line) for line in Path(reworded_file).read_text().split("\n") if line.strip()]
stemmer = Stemmer.Stemmer("english")


def terms(text):
    return set(bm25s.tokenize([text], lower=True, token_pattern=r"(?u)\b\w\w+\b", stopwords="en", stemmer=stemmer,
                              return_ids=False, show_progress=False)[0])


def overlap(form):
    shares = []
    for r in rows:
        old = terms(r["reword"][form])
        shares.append(len(old & terms(r[form])) / len(old) if old else 1.0)
    return sum(shares) / len(shares)


def scores(run):
    return {(r["id"], r["form"]): r["score"] for r in
            (json.loads(line) for line in (Path(run) / "scores.jsonl").read_text().split("\n") if line.strip())}


arms = {}
for spec in sys.argv[4:]:
    arm, runs = spec.split("=", 1)
    original, reworded = (scores(p) for p in runs.split(","))
    arms[arm] = (original, reworded)

ids = [r["id"] for r in rows]
cls = {r["id"]: r["class"] for r in rows}
by_class = {c: [i for i in ids if cls[i] == c] for c in weights if any(cls[i] == c for i in ids)}
for arm, (o, w) in arms.items():
    missing = [i for i in ids for f in FORMS if (i, f) not in o or (i, f) not in w]
    if missing:
        sys.exit(f"wording: {arm} has no score for {', '.join(f'{i}/{f}' for i, f in missing[:5])}")


def headline(s, form, sample):
    total = sum(weights[c] for c in sample)
    return sum(weights[c] * sum(s[(i, form)] for i in qs) / len(qs) for c, qs in sample.items()) / total


def drop(arm, form, sample):
    o, w = arms[arm]
    return headline(o, form, sample) - headline(w, form, sample)


rng = random.Random(SEED)
samples = [{c: [rng.choice(qs) for _ in qs] for c, qs in by_class.items()} for _ in range(RESAMPLES)]


def interval(fn):
    xs = sorted(fn(s) for s in samples)
    return [xs[int(0.025 * RESAMPLES)], xs[int(0.975 * RESAMPLES) - 1]]


tested = next(iter(arms))
report = {"questions": len(ids), "resamples": RESAMPLES, "seed": SEED, "tested": tested,
          "overlap": {f: overlap(f) for f in FORMS}, "arms": {}, "versus": {}}
for arm in arms:
    o, w = arms[arm]
    report["arms"][arm] = {f: {"original": headline(o, f, by_class), "reworded": headline(w, f, by_class),
                               "drop": drop(arm, f, by_class), "drop_95": interval(lambda s: drop(arm, f, s))}
                           for f in FORMS}
for arm in list(arms)[1:]:
    report["versus"][arm] = {f: {"extra_drop": drop(tested, f, by_class) - drop(arm, f, by_class),
                                 "extra_drop_95": interval(lambda s: drop(tested, f, s) - drop(arm, f, s))}
                             for f in FORMS}
Path(out).write_text(json.dumps(report, indent=2) + "\n")


print(f"term overlap with the original: query {100 * report['overlap']['query']:.0f}%, "
      f"prompt {100 * report['overlap']['prompt']:.0f}%\n")
print("| Arm | query: original | reworded | drop (95%) | prompt: original | reworded | drop (95%) |")
print("|---|---:|---:|---:|---:|---:|---:|")
for arm, r in report["arms"].items():
    cells = [f"{100 * r[f]['original']:.0f}% | {100 * r[f]['reworded']:.0f}% | {100 * r[f]['drop']:.1f} "
             f"({100 * r[f]['drop_95'][0]:.1f} to {100 * r[f]['drop_95'][1]:.1f})" for f in FORMS]
    print(f"| {arm} | {' | '.join(cells)} |")
print(f"\n| {tested} drops more than | query (95%) | prompt (95%) |\n|---|---:|---:|")
for arm, r in report["versus"].items():
    print(f"| {arm} | " + " | ".join(f"{100 * r[f]['extra_drop']:.1f} ({100 * r[f]['extra_drop_95'][0]:.1f} to "
                                     f"{100 * r[f]['extra_drop_95'][1]:.1f})" for f in FORMS) + " |")
