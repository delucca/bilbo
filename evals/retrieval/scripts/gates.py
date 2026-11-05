"""The judge's two gates. Usage:
  gates.py stress <selected.jsonl> <gold-dir> <stress-dir> <out.json>
  gates.py sample <selected.jsonl> <out-contexts.jsonl> <arm>=<run-dir>...
  gates.py audit <selected.jsonl> <audit-dir> <out.json> <arm>=<run-dir>...
`stress` scores the gold and the stress contexts that stress.py built. `sample` draws the Opus verdicts a second judge
reads blind, spread over the arms, the classes and verdicts with and without a present item; its form is <arm>/<form>.
`audit` compares the second judge's marks with Opus's, item by item."""

import json
import random
import sys
from collections import defaultdict
from pathlib import Path

GOLD_MIN = 0.95
CAUGHT_MIN = 0.90
UNRELATED_MAX = 0.05
KAPPA_MIN = 0.75
SAMPLE = 150


def rows(path):
    return [json.loads(line) for line in Path(path).read_text().split("\n") if line.strip()] if Path(path).exists() else []


def verdicts(d, budget=4000):
    """(id, form) -> (context row, last verdict for that context)."""
    judged = {}
    for r in rows(Path(d) / "judged.jsonl"):
        judged[(r["id"], r["form"], r["ctx"])] = r
    out = {}
    for c in rows(Path(d) / f"contexts-{budget}.jsonl"):
        v = judged.get((c["id"], c["form"], c["sha"]))
        if v:
            out[(c["id"], c["form"])] = (c, v)
    return out


def share(q, v):
    return sum(v["facts"][f["id"]] for f in q["facts"]) / len(q["facts"])


def mean(xs):
    return sum(xs) / len(xs) if xs else None


def stress(selected, gold_dir, stress_dir, out):
    questions = {q["id"]: q for q in rows(selected)}
    gold = verdicts(gold_dir)
    missing = [i for i in questions if (i, "both") not in gold]
    if missing:
        sys.exit(f"gates: no gold verdict for {', '.join(missing)}")
    gold_scores = {i: share(q, gold[(i, "both")][1]) for i, q in questions.items()}

    drop, swap, unrelated = [], [], []
    expected = [(r["id"], r["form"]) for r in rows(Path(stress_dir) / "search.jsonl")]
    judged = verdicts(stress_dir)
    left = [f"{i}/{f}" for i, f in expected if (i, f) not in judged]
    if left:
        sys.exit(f"gates: no stress verdict for {', '.join(left)}")
    for i, form in expected:
        q, (c, v) = questions[i], judged[(i, form)]
        kind, ref = form.split(":", 1)
        if kind == "drop":
            rest = [f["id"] for f in q["facts"] if f["id"] != ref]
            drop.append({"id": i, "fact": ref, "caught": not v["facts"][ref],
                         "kept": sum(v["facts"][f] for f in rest) / len(rest), "tokens": c["tokens"]})
        elif kind == "swap":
            s = next(s for s in q["stale"] if s["id"] == ref)
            swap.append({"id": i, "stale": ref, "replaces": s["replaces"], "fact_absent": not v["facts"][s["replaces"]],
                         "stale_present": v["stale"][ref], "caught": not v["facts"][s["replaces"]] and v["stale"][ref]})
        else:
            unrelated.append({"id": i, "other": ref, "score": share(q, v)})

    checks = {
        "gold": {"value": mean(list(gold_scores.values())), "min": GOLD_MIN, "n": len(gold_scores)},
        "drop": {"value": mean([r["caught"] for r in drop]), "min": CAUGHT_MIN, "n": len(drop),
                 "kept": mean([r["kept"] for r in drop])},
        "swap": {"value": mean([r["caught"] for r in swap]), "min": CAUGHT_MIN, "n": len(swap),
                 "fact_absent": mean([r["fact_absent"] for r in swap]),
                 "stale_present": mean([r["stale_present"] for r in swap])},
        "unrelated": {"value": mean([r["score"] for r in unrelated]), "max": UNRELATED_MAX, "n": len(unrelated)},
    }
    for c in checks.values():
        c["pass"] = c["value"] >= c["min"] if "min" in c else c["value"] <= c["max"]
    report = {"pass": all(c["pass"] for c in checks.values()), "checks": checks,
              "gold_below_1": {i: s for i, s in sorted(gold_scores.items()) if s < 1},
              "drop_missed": [r for r in drop if not r["caught"]], "swap_missed": [r for r in swap if not r["caught"]],
              "unrelated_hits": [r for r in unrelated if r["score"] > 0]}
    Path(out).write_text(json.dumps(report, indent=2) + "\n")
    for name, c in checks.items():
        bound = f">= {c['min']}" if "min" in c else f"<= {c['max']}"
        print(f"{name}: {c['value']:.3f} (n {c['n']}, needs {bound}) {'pass' if c['pass'] else 'FAIL'}")
    print("stress:", "pass" if report["pass"] else "FAIL")


def arm_dirs(specs):
    return [tuple(s.split("=", 1)) for s in specs]


def sample(selected, out, specs):
    questions = {q["id"]: q for q in rows(selected)}
    seen, cells = set(), defaultdict(list)
    for arm, d in arm_dirs(specs):
        for (i, form), (c, v) in sorted(verdicts(d).items()):
            if i not in questions or v["prompt"] in seen or not c["text"].strip():
                continue
            seen.add(v["prompt"])
            hit = any(v["facts"].values()) or any(v["stale"].values())
            cells[(arm, questions[i]["class"], hit)].append({"id": i, "form": f"{arm}/{form}", "sha": c["sha"],
                                                              "tokens": c["tokens"], "text": c["text"]})
    rng = random.Random("audit")
    by_arm = defaultdict(list)
    for key in sorted(cells):
        by_arm[key[0]].append(rng.sample(cells[key], len(cells[key])))
    picked = []
    while len(picked) < SAMPLE and any(any(qs) for qs in by_arm.values()):
        for arm in sorted(by_arm):
            at = next((n for n, qs in enumerate(by_arm[arm]) if qs), None)
            if at is not None and len(picked) < SAMPLE:
                queue = by_arm[arm].pop(at)
                picked.append(queue.pop())
                by_arm[arm].append(queue)
    Path(out).write_text("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in picked))
    print(f"sample: {len(picked)} verdicts from {len(by_arm)} arms")


def kappa(pairs):
    n = len(pairs)
    po = sum(a == b for a, b in pairs) / n
    pa, pb = sum(a for a, _ in pairs) / n, sum(b for _, b in pairs) / n
    pe = pa * pb + (1 - pa) * (1 - pb)
    return (po - pe) / (1 - pe) if pe < 1 else 1.0, po


def audit(selected, audit_dir, out, specs):
    questions = {q["id"]: q for q in rows(selected)}
    opus = {}
    for arm, d in arm_dirs(specs):
        for (i, form), (_, v) in verdicts(d).items():
            opus[(i, f"{arm}/{form}")] = v
    second = verdicts(audit_dir)
    sampled = [(c["id"], c["form"]) for c in rows(Path(audit_dir) / "contexts-4000.jsonl")]
    left = [f"{i}/{f}" for i, f in sampled if (i, f) not in second]
    if left:
        sys.exit(f"gates: no second verdict for {', '.join(left)}")
    pairs, by_arm, diffs, judge = [], defaultdict(list), [], None
    for i, form in sampled:
        a, (_, b) = opus[(i, form)], second[(i, form)]
        for kind in ("facts", "stale"):
            for item, mark in a[kind].items():
                pairs.append((mark, b[kind][item]))
                by_arm[form.split("/")[0]].append((mark, b[kind][item]))
                if mark != b[kind][item]:
                    diffs.append({"id": i, "form": form, "item": item, "opus": mark, "second": b[kind][item]})
    k, po = kappa(pairs)
    report = {"pass": k >= KAPPA_MIN, "kappa": k, "min": KAPPA_MIN, "agreement": po, "verdicts": len(sampled),
              "items": len(pairs), "opus_present": mean([a for a, _ in pairs]), "second_present": mean([b for _, b in pairs]),
              "by_arm": {arm: {"items": len(ps), "agreement": kappa(ps)[1]} for arm, ps in sorted(by_arm.items())},
              "disagreements": diffs}
    Path(out).write_text(json.dumps(report, indent=2) + "\n")
    print(f"audit: kappa {k:.3f} (needs >= {KAPPA_MIN}), agreement {po:.3f} on {len(pairs)} items of {len(sampled)} "
          f"verdicts; present {report['opus_present']:.3f} Opus, {report['second_present']:.3f} second; "
          f"{'pass' if report['pass'] else 'FAIL'}")


if __name__ == "__main__":
    cmd, *a = sys.argv[1:]
    {"stress": lambda: stress(*a[:4]), "sample": lambda: sample(a[0], a[1], a[2:]),
     "audit": lambda: audit(a[0], a[1], a[2], a[3:])}[cmd]()
