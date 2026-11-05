"""Compare an arm's read rule with the agent's reading of the same questions. Usage:
compare.py <arm> <selected.jsonl> <rule-run-dir> <agent-run-dir>. Both sides are on the `query` form, judged by the same
judge on contexts packed to the same budget. Writes compare.json and compare.md into the agent's run dir and prints the
table: per question the two scores, what each opened, the agent's searches and whether it ran out of budget; overall,
the mean paired difference with a bootstrap interval and the overlap of what was opened."""

import json
import random
import re
import shlex
import statistics
import sys
from collections import defaultdict
from pathlib import Path

BUDGET = 4000
RESAMPLES = 10000
SEED = 20261105
SPENT = "reading budget is spent"

arm, selected, rule_dir, agent_dir = sys.argv[1], Path(sys.argv[2]), Path(sys.argv[3]), Path(sys.argv[4])
questions = [json.loads(line) for line in selected.read_text().splitlines() if line.strip()]


def rows(path):
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()] if path.exists() else []


def query_rows(d, name):
    return {r["id"]: r for r in rows(d / name) if r["form"] == "query"}


def verdicts(d):
    judged = defaultdict(list)
    for r in rows(d / "judged.jsonl"):
        judged[(r["id"], r["form"], r["ctx"])].append(r)
    return {c["id"]: (c, judged[(c["id"], c["form"], c["sha"])][-1])
            for c in rows(d / f"contexts-{BUDGET}.jsonl") if c["form"] == "query" and judged[(c["id"], c["form"], c["sha"])]}


def score(q, v):
    return sum(bool(v["facts"][f["id"]]) for f in q["facts"]) / len(q["facts"])


def rule_opened(row, packed):
    """The hits the rule opens whole; for auto-memory, which opens no k, the topic files that fit the budget."""
    if arm == "auto-memory":
        return [r["ref"].split("/", 1)[1] for r in row["results"][:packed] if not r["ref"].endswith("/MEMORY.md")]
    return [r["ref"] for r in row["results"] if r.get("read")]


def agent_opened(row):
    calls = [c for c in row["agent"]["calls"] if not c.get("error")]
    if arm.startswith("bilbo"):
        return [c["input"]["file_path"] for c in calls if c["tool"] == "Read"]
    if arm == "ai-memory":
        return [c["input"].get("path") or f"query:{c['input'].get('query')}" for c in calls
                if c["tool"].endswith("__memory_read_page")]
    if arm == "claude-mem":
        return [m for r in row["results"] for m in re.findall(r"^### \[([^\]]+)\]", "\n" + r["text"], re.M)]
    return [re.sub(r"^.*/memory/", "", c["input"]["file_path"]) for c in calls
            if c["tool"] == "Read" and not c["input"]["file_path"].endswith("/MEMORY.md")]


def searches(row):
    out = []
    for c in row["agent"]["calls"]:
        args = c["input"]
        if c["tool"] == "Bash" and args.get("command", "").startswith("bilbo recall"):
            words = shlex.split(args["command"])
            out.append(" ".join(words[words.index("--") + 1:]) if "--" in words else " ".join(words[2:]))
        elif c["tool"].endswith("__memory_query") or (c["tool"].endswith("__mem_search") and "continuation" not in args):
            out.append(args.get("query", ""))
        elif c["tool"] in ("Grep", "Glob"):
            out.append(f"{c['tool']} {args.get('pattern', '')}")
    return out


def bootstrap(diffs):
    rng = random.Random(SEED)
    means = sorted(statistics.fmean(rng.choices(diffs, k=len(diffs))) for _ in range(RESAMPLES))
    return means[int(0.025 * RESAMPLES)], means[int(0.975 * RESAMPLES) - 1]


rule_rows, agent_rows = query_rows(rule_dir, "search.jsonl"), query_rows(agent_dir, "search.jsonl")
rule_v, agent_v = verdicts(rule_dir), verdicts(agent_dir)
missing = [q["id"] for q in questions if q["id"] not in rule_v or q["id"] not in agent_v]
if missing:
    sys.exit(f"compare: no verdict on both sides for {', '.join(missing)}")

per = []
for q in questions:
    (rc, rv), (ac, av) = rule_v[q["id"]], agent_v[q["id"]]
    ra, aa = rule_rows[q["id"]], agent_rows[q["id"]]
    r_open, a_open = rule_opened(ra, rc["results"]), agent_opened(aa)
    calls = aa["agent"]["calls"]
    per.append({
        "id": q["id"], "class": q["class"], "rule": score(q, rv), "agent": score(q, av),
        "rule_opened": r_open, "agent_opened": a_open, "both_opened": [r for r in r_open if r in a_open],
        "searches": searches(aa), "rule_query": q["query"], "calls": len(calls),
        "out_of_budget": any(SPENT in (c.get("said") or "") for c in calls),
        "refused": sum(1 for c in calls if c.get("error") and SPENT not in (c.get("said") or "")),
        "rule_tokens": rc["tokens"], "agent_tokens": ac["tokens"], "agent_cut": ac["cut"],
        "stop": aa["agent"]["stop"], "seconds": aa["agent"]["seconds"],
    })

diffs = [p["agent"] - p["rule"] for p in per]
low, high = bootstrap(diffs)
k = [len(p["rule_opened"]) for p in per]
summary = {
    "arm": arm, "questions": len(per), "model": next(iter(agent_rows.values()))["agent"]["model"],
    "rule": statistics.fmean(p["rule"] for p in per), "agent": statistics.fmean(p["agent"] for p in per),
    "diff": statistics.fmean(diffs), "diff_95": [low, high],
    "agent_better": sum(d > 0 for d in diffs), "rule_better": sum(d < 0 for d in diffs), "same": sum(d == 0 for d in diffs),
    "rule_opened_mean": statistics.fmean(k), "agent_opened_mean": statistics.fmean(len(p["agent_opened"]) for p in per),
    "both_opened_median": statistics.median(len(p["both_opened"]) for p in per),
    "agent_opened_none": sum(not p["agent_opened"] for p in per),
    "searches_mean": statistics.fmean(len(p["searches"]) for p in per),
    "own_words": sum(bool(p["searches"]) and p["searches"][0].strip().lower() != p["rule_query"].strip().lower()
                     for p in per),
    "out_of_budget": sum(p["out_of_budget"] for p in per),
    "rule_tokens_mean": statistics.fmean(p["rule_tokens"] for p in per),
    "agent_tokens_mean": statistics.fmean(p["agent_tokens"] for p in per),
}
(agent_dir / "compare.json").write_text(json.dumps({"summary": summary, "questions": per}, indent=2,
                                                   ensure_ascii=False) + "\n")


def pct(x):
    return f"{100 * x:.0f}%"


s = summary
lines = [f"{arm}: rule {pct(s['rule'])}, agent ({s['model']}) {pct(s['agent'])}, agent minus rule "
         f"{100 * s['diff']:+.0f} points (95% {100 * low:+.0f} to {100 * high:+.0f}); agent better on {s['agent_better']}, "
         f"rule on {s['rule_better']}, same on {s['same']} of {s['questions']}",
         f"opened: rule {s['rule_opened_mean']:.1f}, agent {s['agent_opened_mean']:.1f} (none on {s['agent_opened_none']}), "
         f"both a median of {s['both_opened_median']:g}; agent searches {s['searches_mean']:.1f} per question, in its own "
         f"words on {s['own_words']}; out of budget on {s['out_of_budget']}; tokens read: rule {s['rule_tokens_mean']:.0f}, "
         f"agent {s['agent_tokens_mean']:.0f}",
         "", "| Question | Class | Rule | Agent | Opened: rule / agent / both | Searches | Tokens r/a | Budget |",
         "|---|---|---:|---:|---|---:|---|---|"]
for p in per:
    lines.append(f"| {p['id']} | {p['class']} | {pct(p['rule'])} | {pct(p['agent'])} | {len(p['rule_opened'])} / "
                 f"{len(p['agent_opened'])} / {len(p['both_opened'])} | {len(p['searches'])} | {p['rule_tokens']}/"
                 f"{p['agent_tokens']}{' cut' if p['agent_cut'] else ''} | {'spent' if p['out_of_budget'] else ''} |")
text = "\n".join(lines) + "\n"
(agent_dir / "compare.md").write_text(text)
print(text, end="")
