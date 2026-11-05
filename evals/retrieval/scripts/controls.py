"""Build the two controls every arm is compared with: `gold`, the lines that state each fact, and `random`, random lines
of the same project. Usage: controls.py <corpus-dir> <selected.jsonl> <gold-out> <random-out>. Rows are appended for
the questions an output lacks."""

import functools
import json
import random
import re
import sys
from pathlib import Path

GOLD_WINDOW = 1500
UNIT_CHARS = 1200
RANDOM_UNITS = 40


def ws(s):
    return re.sub(r"\s+", " ", s).strip()


def string_leaves(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for v in value.values():
            yield from string_leaves(v)
    elif isinstance(value, list):
        for v in value:
            yield from string_leaves(v)


def result_text(content):
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "\n".join(x.get("text", "") if x.get("type") == "text" else "[image]" for x in content if isinstance(x, dict))
    return ""


def role_of(e):
    t = e.get("type")
    if t in ("user", "assistant"):
        return t
    if t is None and e.get("role") in ("user", "assistant"):
        return e["role"]
    return None


def texts(e):
    """The blocks of one entry that may be quoted; thinking, meta entries and compaction summaries never are."""
    role = role_of(e)
    if role is None or e.get("isMeta") or e.get("isCompactSummary"):
        return []
    content = (e.get("message") or {}).get("content")
    if isinstance(content, str):
        return [content]
    out = []
    for b in content or []:
        if not isinstance(b, dict):
            continue
        if b.get("type") == "text":
            out.append(b.get("text", ""))
        elif b.get("type") == "tool_use":
            out += list(string_leaves(b.get("input") or {}))
        elif b.get("type") == "tool_result":
            out.append(result_text(b.get("content")))
    return out


def stamp(e):
    t = e.get("timestamp")
    return f"[{t[:16].replace('T', ' ')} UTC] " if t else ""


def load(corpus, repo, session):
    (path,) = (corpus / repo).glob(f"*--{session}.jsonl")
    # Evidence lines count newlines only: splitlines() also breaks on U+0085 and U+2028, which sessions hold.
    lines = path.read_text(encoding="utf-8").split("\n")
    if lines[-1] == "":
        lines.pop()
    out = []
    for no, line in enumerate(lines, 1):
        try:
            out.append((no, json.loads(line)))
        except ValueError:
            out.append((no, {}))
    return out


def gold_text(entries, ev):
    lo, hi = ev["lines"]
    quote = ws(ev["quote"])
    parts = []
    for no, e in entries:
        if lo <= no <= hi:
            for t in texts(e):
                flat = ws(t)
                i = flat.find(quote)
                cut = flat[max(0, i - GOLD_WINDOW):i + len(quote) + GOLD_WINDOW] if i >= 0 else flat[:2 * GOLD_WINDOW]
                parts.append(stamp(e) + cut)
    return " ".join(parts)


def gold_row(corpus, q):
    cache, seen, results = {}, set(), []
    for fact in q["facts"]:
        for ev in fact["evidence"]:
            if ev["session"] not in cache:
                cache[ev["session"]] = load(corpus, q["repo"], ev["session"])
            entries = cache[ev["session"]]
            text = gold_text(entries, ev)
            if text and text not in seen:
                seen.add(text)
                results.append({"text": text})
    return {"id": q["id"], "form": "both", "results": results}


@functools.cache
def units_of(corpus, repo):
    """(session, line, text) of every block of the repo with at least 40 characters."""
    out = []
    for path in sorted((corpus / repo).glob("*.jsonl")):
        session = path.name.split("--", 1)[1][:-len(".jsonl")]
        for no, e in load(corpus, repo, session):
            for t in texts(e):
                flat = ws(t)
                if len(flat) >= 40:
                    out.append((session, no, stamp(e) + flat[:UNIT_CHARS]))
    return out


def random_row(q, units):
    spans = {(ev["session"], n) for group in (q["facts"], q.get("stale", [])) for item in group for ev in item["evidence"]
             for n in range(ev["lines"][0], ev["lines"][1] + 1)}
    pool = [text for session, no, text in units if (session, no) not in spans]
    picked = random.Random(f"random-{q['id']}").sample(pool, min(RANDOM_UNITS, len(pool)))
    return {"id": q["id"], "form": "both", "results": [{"text": t} for t in picked]}


def have(path):
    return {json.loads(line)["id"] for line in path.read_text().split("\n") if line.strip()} if path.exists() else set()


def main():
    corpus, selected, gold_out, random_out = (Path(a) for a in sys.argv[1:5])
    questions = [json.loads(line) for line in selected.read_text().split("\n") if line.strip()]
    for out, build in ((gold_out, lambda q: gold_row(corpus, q)),
                       (random_out, lambda q: random_row(q, units_of(corpus, q["repo"])))):
        out.parent.mkdir(parents=True, exist_ok=True)
        done = have(out)
        with out.open("a") as f:
            for q in questions:
                if q["id"] not in done:
                    f.write(json.dumps(build(q), ensure_ascii=False) + "\n")
                    f.flush()


if __name__ == "__main__":
    main()
