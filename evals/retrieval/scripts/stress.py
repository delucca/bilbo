"""Build the judge's stress contexts from the gold. Usage: stress.py <corpus-dir> <selected.jsonl> <stress-search.jsonl>.
Each row is a search row whose form names the change, judged against the question's own facts:
  drop:<fact>      the evidence quotes of every other fact
  swap:<stale>     the same, with the older value's gold in place of the fact it was replaced by
  unrelated:<id>   another project's gold
Facts read as their quotes, not the gold's windows: a window around one fact's quote often restates its neighbour, so a
fact left out of the windows was still there. An older value reads as its gold, its own evidence lines around its
quote, since a quote often holds only part of it; a swap is skipped when the older value and the fact share evidence
lines, which cannot be told apart. The samples are seeded and spread over the classes, so the file is the same on
every run."""

import json
import random
import sys
from pathlib import Path

import controls

DROP = 60
UNRELATED = 40


def spread(questions, n, seed):
    """Up to n questions, taking each class in turn, each class in a seeded order."""
    by_class = {}
    for q in sorted(questions, key=lambda q: q["id"]):
        by_class.setdefault(q["class"], []).append(q)
    rng = random.Random(seed)
    queues = [rng.sample(qs, len(qs)) for _, qs in sorted(by_class.items())]
    out = []
    while len(out) < n and any(queues):
        for queue in queues:
            if queue and len(out) < n:
                out.append(queue.pop())
    return out


def lines_of(item):
    return {(ev["session"], n) for ev in item["evidence"] for n in range(ev["lines"][0], ev["lines"][1] + 1)}


def quotes(items):
    out = []
    for item in items:
        for ev in item["evidence"]:
            quote = controls.ws(ev["quote"])
            if quote not in out:
                out.append(quote)
    return out


def leaves(items, gone):
    """The quotes of items, without any that holds one of the quotes of the item left out."""
    cut = quotes([gone])
    return [{"text": t} for t in quotes(items) if not any(c in t or t in c for c in cut)]


def drop_row(q, fact):
    return {"id": q["id"], "form": f"drop:{fact['id']}", "results": leaves([f for f in q["facts"] if f["id"] != fact["id"]], fact)}


def swap_row(corpus, q, stale):
    replaced = next(f for f in q["facts"] if f["id"] == stale["replaces"])
    entries = {ev["session"]: controls.load(corpus, q["repo"], ev["session"]) for ev in stale["evidence"]}
    cut = quotes([replaced])
    results = []
    for f in q["facts"]:
        if f is replaced:
            for ev in stale["evidence"]:
                text = controls.gold_text(entries[ev["session"]], ev)
                for c in cut:
                    text = text.replace(c, "")
                if text.strip():
                    results.append({"text": text})
        else:
            results += leaves([f], replaced)
    return {"id": q["id"], "form": f"swap:{stale['id']}", "results": results}


def droppable(q):
    """Facts whose evidence lines no other fact shares, so no other fact carries their quotes."""
    return [f for f in q["facts"] if not lines_of(f) & set().union(*(lines_of(g) for g in q["facts"] if g is not f))]


def main():
    corpus, selected, out = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
    questions = [json.loads(line) for line in selected.read_text().split("\n") if line.strip()]
    rows = []

    pool = [q for q in questions if len(q["facts"]) > 1 and droppable(q)]
    for q in spread(pool, DROP, "drop"):
        rows.append(drop_row(q, random.Random(f"drop-{q['id']}").choice(droppable(q))))

    for q in questions:
        for stale in q.get("stale", []):
            replaced = next(f for f in q["facts"] if f["id"] == stale["replaces"])
            if not lines_of(stale) & lines_of(replaced):
                rows.append(swap_row(corpus, q, stale))

    for q in spread(questions, UNRELATED, "unrelated"):
        others = [o for o in questions if o["repo"] != q["repo"]]
        same = [o for o in others if o["class"] == q["class"]]
        other = random.Random(f"unrelated-{q['id']}").choice(sorted(same or others, key=lambda o: o["id"]))
        rows.append({"id": q["id"], "form": f"unrelated:{other['id']}", "results": controls.gold_row(corpus, other)["results"]})

    tmp = out.with_name(out.name + ".part")
    tmp.write_text("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in rows))
    tmp.rename(out)


if __name__ == "__main__":
    main()
