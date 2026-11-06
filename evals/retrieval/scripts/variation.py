"""Compare each arm's run on the original wording with its run on the query variation. Usage:
variation.py <questions.jsonl> <classes.json> <out.json> <arm>=<original-run>,<variation-run> ...
Before anything it checks that each pair belongs together and to these questions: both run dirs are named for the arm,
the same store and the same search, the first with no variation and the second with the variation key of
questions.jsonl now; both score.json name the same; both runs' selected.jsonl are the answerable questions as they are
now, in the original wording; both runs' scores were judged against that selection (selected.sha256, which score.py
writes; for a run without one, the scores recomputed from its verdicts for the current judge prompts must equal
its scores.jsonl); and the variation run's searched.jsonl holds, for every question, the variation's text as
it is now. The report records both run dirs and the SHA-256 of what the variation run searched.
The first arm is the one tested: its drop is set against every other arm's. A drop is the original headline minus the
variation's, per form, with classes weighted as score.py weights them; the 95% intervals come from 10,000 paired
resamples of the questions within each class. A relative drop is the drop over the original headline. Each figure is
given for all the answerable questions and for three strata: the framing-only questions, whose every topic word is on
the keep-list, the rest, and the under-reworded ones, whose variation changed nothing but function words. The rule set before the runs: on `query`, the tested arm's drop is at least RULE_POINTS
larger than each other arm's, with the interval of the difference above zero; `prompt` is reported the same way, as
the secondary form, without the arms of NO_PROMPT_RULE. Overlap is the share of the original's terms, tokenized as the
bm25 arm tokenizes, that the variation still holds."""

import hashlib
import json
import random
import re
import sys
from pathlib import Path

import bm25s
import Stemmer

import judged
import questions

FORMS = ("query", "prompt")
RESAMPLES = 10_000
SEED = 20261105
RULE_POINTS = 0.05
NO_PROMPT_RULE = {"claude-mem": "its prompt form is the SessionStart injection, which ignores the question's text"}
RUN = re.compile(r"(?P<arm>.+)-(?P<store>[0-9a-f]{12})-(?P<search>[0-9a-f]{12})"
                 r"(?:-variation-(?P<variation>[0-9a-f]{12}))?(?:-rerun-(?P<rerun>[a-z0-9]+))?")

questions_file, classes_file, out = sys.argv[1:4]
weights = {c["name"]: c["weight"] for c in json.loads(Path(classes_file).read_text())["classes"]}
rows = questions.answerable(questions_file)
missing = [r["id"] for r in rows if "variation" not in r["wording"]]
if missing:
    sys.exit(f"variation: no variation for {', '.join(missing[:5])}; run vary.sh")
variation_key = questions.key(rows)
selected_now = "".join(questions.line(r, "original") for r in rows)
stemmer = Stemmer.Stemmer("english")


def terms(text):
    return set(bm25s.tokenize([text], lower=True, token_pattern=r"(?u)\b\w\w+\b", stopwords="en", stemmer=stemmer,
                              return_ids=False, show_progress=False)[0])


def overlap(form):
    shares = []
    for r in rows:
        old = terms(r["wording"]["original"][form])
        shares.append(len(old & terms(r["wording"]["variation"][form])) / len(old) if old else 1.0)
    return sum(shares) / len(shares)


def lines(path):
    return [json.loads(line) for line in Path(path).read_text().split("\n") if line.strip()]


def stale_scores(d):
    """Why d's scores may not be those of the questions as they are now, or nothing. Without a selected.sha256 the
    scores are recomputed from the verdicts for the current judge prompts and must equal scores.jsonl."""
    selected = d / "selected.jsonl"
    recorded = d / "selected.sha256"
    if recorded.exists():
        if recorded.read_text().strip() != hashlib.sha256(selected.read_bytes()).hexdigest():
            return f"{d.name} was scored against another selected.jsonl; run eval.sh for it again"
        return None
    qs = judged.rows(selected)
    got = judged.verdicts(d, {q["id"]: q for q in qs})
    expected = [{"id": q["id"], "class": q["class"], "repo": q["repo"], "form": f, **judged.grade(q, got[(q["id"], f)])}
                for q in qs for f in FORMS if (q["id"], f) in got]
    if len(expected) != len(qs) * len(FORMS) or expected != judged.rows(d / "scores.jsonl"):
        return f"{d.name} has no selected.sha256 and its scores.jsonl is not what its verdicts give now; run eval.sh for it again"
    return None


def pair(arm, original, variation):
    """Both runs' scores, after the checks the docstring lists; any mismatch stops the comparison."""
    o, v = Path(original), Path(variation)
    mo, mv = RUN.fullmatch(o.name), RUN.fullmatch(v.name)
    problems = []
    if not mo or not mv:
        sys.exit(f"variation: {arm}: run dirs must be named <arm>-<store>-<search>[-variation-<key>][-rerun-<label>]")
    if mo["arm"] != arm or mv["arm"] != arm:
        problems.append("a run dir is not this arm's")
    if (mo["store"], mo["search"]) != (mv["store"], mv["search"]):
        problems.append("the two runs differ in store or search")
    if mo["variation"]:
        problems.append("the original run is a variation run")
    if mv["variation"] != variation_key:
        problems.append(f"the variation run is keyed {mv['variation']}, the questions' variation is {variation_key}")
    for d, m in ((o, mo), (v, mv)):
        s = json.loads((d / "score.json").read_text())
        if (s["arm"], s["store"], s["search"], s.get("variation")) != (arm, m["store"], m["search"], m["variation"]):
            problems.append(f"{d.name}/score.json names another run")
        if (d / "selected.jsonl").read_text() != selected_now:
            problems.append(f"{d.name}/selected.jsonl is not the answerable questions as they are now")
        elif stale := stale_scores(d):
            problems.append(stale)
    searched = v / "searched.jsonl"
    if not searched.exists():
        problems.append(f"{v.name} has no searched.jsonl")
    else:
        last = {r["id"]: (r["prompt"], r["query"]) for r in lines(searched)}
        stale = [r["id"] for r in rows
                 if last.get(r["id"]) != (r["wording"]["variation"]["prompt"], r["wording"]["variation"]["query"])]
        if stale:
            problems.append(f"{v.name} searched other text for {', '.join(stale[:5])}")
    if problems:
        sys.exit(f"variation: {arm}: " + "; ".join(problems))
    record = {"original_run": str(o), "variation_run": str(v), "store": mv["store"], "search": mv["search"],
              "variation": variation_key, "searched_sha256": hashlib.sha256(searched.read_bytes()).hexdigest()}
    return record, tuple({(r["id"], r["form"]): r["score"] for r in lines(d / "scores.jsonl")} for d in (o, v))


arms, runs = {}, {}
for spec in sys.argv[4:]:
    arm, pair_spec = spec.split("=", 1)
    runs[arm], arms[arm] = pair(arm, *pair_spec.split(","))
cls = {r["id"]: r["class"] for r in rows}
framing = {r["id"] for r in rows if r["wording"]["variation"]["check"]["framing_only"]}
for arm, (o, w) in arms.items():
    gaps = [(i, f) for i in cls for f in FORMS if (i, f) not in o or (i, f) not in w]
    if gaps:
        sys.exit(f"variation: {arm} has no score for {', '.join(f'{i}/{f}' for i, f in gaps[:5])}")


def headline(s, form, sample):
    total = sum(weights[c] for c in sample)
    return sum(weights[c] * sum(s[(i, form)] for i in qs) / len(qs) for c, qs in sample.items()) / total


def drop(arm, form, sample):
    o, w = arms[arm]
    return headline(o, form, sample) - headline(w, form, sample)


def stratum(ids):
    by_class = {c: [i for i in ids if cls[i] == c] for c in weights if any(cls[i] == c for i in ids)}
    rng = random.Random(SEED)
    samples = [{c: [rng.choice(qs) for _ in qs] for c, qs in by_class.items()} for _ in range(RESAMPLES)]

    def interval(fn):
        xs = sorted(fn(s) for s in samples)
        return [xs[int(0.025 * RESAMPLES)], xs[int(0.975 * RESAMPLES) - 1]]

    tested = next(iter(arms))
    report = {"questions": len(ids), "classes": {c: len(qs) for c, qs in by_class.items()}, "arms": {}, "versus": {}}
    for arm in arms:
        o, w = arms[arm]
        report["arms"][arm] = {}
        for f in FORMS:
            base, d = headline(o, f, by_class), drop(arm, f, by_class)
            report["arms"][arm][f] = {"original": base, "variation": headline(w, f, by_class), "drop": d,
                                      "drop_95": interval(lambda s: drop(arm, f, s)),
                                      "relative_drop": d / base if base else None}
    for arm in list(arms)[1:]:
        report["versus"][arm] = {}
        for f in FORMS:
            extra = drop(tested, f, by_class) - drop(arm, f, by_class)
            ci = interval(lambda s: drop(tested, f, s) - drop(arm, f, s))
            out_of_rule = f == "prompt" and arm in NO_PROMPT_RULE
            report["versus"][arm][f] = {"extra_drop": extra, "extra_drop_95": ci,
                                        "rule": None if out_of_rule else extra >= RULE_POINTS and ci[0] > 0,
                                        **({"not_in_rule": NO_PROMPT_RULE[arm]} if out_of_rule else {})}
    return report


under = {r["id"] for r in rows if r["wording"]["variation"]["check"]["under"]}
strata = {"all": [r["id"] for r in rows], "framing_only": [i for i in cls if i in framing],
          "other": [i for i in cls if i not in framing], "under_reworded": [i for i in cls if i in under]}
report = {"resamples": RESAMPLES, "seed": SEED, "tested": next(iter(arms)), "rule_points": RULE_POINTS,
          "no_prompt_rule": NO_PROMPT_RULE, "runs": runs, "overlap": {f: overlap(f) for f in FORMS},
          "strata": {name: stratum(ids) for name, ids in strata.items() if ids}}
Path(out).write_text(json.dumps(report, indent=2) + "\n")


def pct(x):
    return f"{100 * x:.0f}%"


def verdict(r):
    return "not in the rule" if r["rule"] is None else "holds" if r["rule"] else "fails"


print(f"term overlap with the original: query {pct(report['overlap']['query'])}, "
      f"prompt {pct(report['overlap']['prompt'])}")
for name, s in report["strata"].items():
    print(f"\n### {name}: {s['questions']} questions\n")
    print("| Arm | query: original | variation | drop (95%) | relative | prompt: original | variation | drop (95%) "
          "| relative |\n|---|---:|---:|---:|---:|---:|---:|---:|---:|")
    for arm, r in s["arms"].items():
        cells = [f"{pct(r[f]['original'])} | {pct(r[f]['variation'])} | {100 * r[f]['drop']:.1f} "
                 f"({100 * r[f]['drop_95'][0]:.1f} to {100 * r[f]['drop_95'][1]:.1f}) | "
                 + (pct(r[f]['relative_drop']) if r[f]['relative_drop'] is not None else "-") for f in FORMS]
        print(f"| {arm} | {' | '.join(cells)} |")
    print(f"\n| {report['tested']} drops more than | query (95%) | rule | prompt (95%) | rule |\n|---|---:|---|---:|---|")
    for arm, r in s["versus"].items():
        print(f"| {arm} | " + " | ".join(
            f"{100 * r[f]['extra_drop']:.1f} ({100 * r[f]['extra_drop_95'][0]:.1f} to "
            f"{100 * r[f]['extra_drop_95'][1]:.1f}) | {verdict(r[f])}" for f in FORMS) + " |")
for arm, why in NO_PROMPT_RULE.items():
    if arm in arms:
        print(f"\n{arm} is left out of the `prompt` rule: {why}.")
