"""Lexical leakage filter: reject echoing queries, mark zero overlap, check alias bridges, count the shared tokens."""

from __future__ import annotations

import argparse
from pathlib import Path

from bilbo_evals import dataset
from bilbo_evals.common import append_jsonl, now, out, read_jsonl, write_json, write_jsonl
from bilbo_evals.schema import SPLITS

MAX_ATTEMPTS = 3


def _attempt(q: dict) -> int:
    try:
        return int(q["gen"]["call_id"].rsplit("/", 1)[1])
    except (KeyError, IndexError, ValueError, AttributeError):
        return 1


def _bucket(n: int) -> str:
    return "2+" if n >= dataset.LEAK_REJECT else str(n)


def _distribution(ds: dataset.Dataset, drops: list[dict]) -> dict[str, dict[str, dict[str, int]]]:
    """Per split and stratum, the queries sharing 0, 1 and 2 or more tokens; rejected ones count in 2+."""
    found: dict[str, dict[str, dict[str, int]]] = {}
    for q in ds.queries:
        if q["stratum"] in dataset.LEAK_STRATA or q["stratum"] == "library":
            cell = found.setdefault(q["split"], {}).setdefault(q["stratum"], {"0": 0, "1": 0, "2+": 0})
            cell[_bucket(len(dataset.leakage(ds, q)))] += 1
    for d in drops:
        if d["reason"] == "leakage":
            cell = found.setdefault(d["split"], {}).setdefault(d["stratum"], {"0": 0, "1": 0, "2+": 0})
            cell["2+"] += 1
    return found


def run(dir: Path, splits: list[str]) -> dict[str, int]:
    dataset.refuse_if_frozen(dir)
    ds = dataset.load(dir)
    drops_path = dir / "generation/drops.jsonl"
    kept, rejected = [], 0
    marked = 0
    for q in ds.queries:
        if q["split"] not in splits or q["stratum"] not in dataset.LEAK_STRATA:
            kept.append(q)
            continue
        shared = dataset.leakage(ds, q)
        alias = dataset.alias_problems(ds, q)
        base = {"item": q["id"], "split": q["split"], "stratum": q["stratum"], "attempt": _attempt(q), "time": now()}
        if len(shared) >= dataset.LEAK_REJECT:
            append_jsonl(drops_path, {**base, "reason": "leakage", "tokens": sorted(shared),
                                      "final": base["attempt"] >= MAX_ATTEMPTS})
            rejected += 1
        elif alias:
            append_jsonl(drops_path, {**base, "reason": "alias", "tokens": [], "detail": alias,
                                      "final": base["attempt"] >= MAX_ATTEMPTS})
            rejected += 1
        else:
            overlap = not shared
            if q["zero_overlap"] != overlap:
                marked += 1
            kept.append({**q, "zero_overlap": overlap})
    write_jsonl(dir / "queries.jsonl", kept)
    ds.queries = kept
    ds._df.clear()
    drops = read_jsonl(drops_path) if drops_path.is_file() else []
    write_json(dir / "generation/leakage.json", _distribution(ds, drops))
    dataset.build_qrels(dir)
    return {"kept": len(kept), "rejected": rejected, "marked": marked}


def cmd(args: argparse.Namespace) -> int:
    splits = [args.split] if args.split else list(SPLITS)
    result = run(args.dataset, splits)
    out(f"filter {','.join(splits)}: kept {result['kept']}, rejected {result['rejected']}, marked {result['marked']}")
    return 0
