"""Read questions/questions.jsonl, where each row keeps its wordings under `wording`. Usage:
  questions.py flat <questions.jsonl> [--wording original|variation] [--ids id,id,...] [--limit N]
  questions.py key <questions.jsonl>
flat prints the answerable questions in the flat shape the arms, the judge and the scorers read: the chosen wording's
`prompt` and `query` beside the other fields, in the order the questions file had them before `wording` existed, so the
original wording's rows are byte for byte the earlier questions file's lines. --ids keeps those questions in the order
given; --limit keeps the first N. key prints the first 12 hex digits of the SHA-256 of the variation's text alone, so a
fixed fact never changes it."""

import argparse
import hashlib
import json
import sys
from pathlib import Path


def rows(path):
    return [json.loads(line) for line in Path(path).read_text(encoding="utf-8").split("\n") if line.strip()]


def answerable(path):
    return [r for r in rows(path) if not r["abstain"]]


def wording_of(r, wording):
    w = r["wording"].get(wording)
    if w is None:
        sys.exit(f"questions: {r['id']} has no {wording} wording; run vary.sh")
    return w


def flat(r, wording):
    w, out, placed = wording_of(r, wording), {}, False
    for k, v in r.items():
        if k == "wording":
            continue
        if not placed and k > "query":
            out["prompt"], out["query"], placed = w["prompt"], w["query"], True
        out[k] = v
    if not placed:
        out["prompt"], out["query"] = w["prompt"], w["query"]
    return out


def key(qs):
    text = {r["id"]: [wording_of(r, "variation")["prompt"], wording_of(r, "variation")["query"]] for r in qs}
    return hashlib.sha256(json.dumps(text, ensure_ascii=False, sort_keys=True).encode()).hexdigest()[:12]


def line(r, wording):
    return json.dumps(flat(r, wording), ensure_ascii=False) + "\n"


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("command", choices=("flat", "key"))
    ap.add_argument("questions")
    ap.add_argument("--wording", choices=("original", "variation"), default="original")
    ap.add_argument("--ids")
    ap.add_argument("--limit", type=int)
    args = ap.parse_args()

    qs = answerable(args.questions)
    if args.command == "key":
        print(key(qs))
        sys.exit()
    if args.ids:
        by_id = {r["id"]: r for r in qs}
        missing = [i for i in args.ids.split(",") if i not in by_id]
        if missing:
            sys.exit(f"questions: {', '.join(missing)} is not an answerable question")
        qs = [by_id[i] for i in args.ids.split(",")]
    if args.limit:
        qs = qs[:args.limit]
    sys.stdout.write("".join(line(r, args.wording) for r in qs))
