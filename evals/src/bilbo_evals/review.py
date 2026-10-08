"""The validity review: draw a seeded sample, check the reviewer's verdicts, apply the drops to the dataset."""

from __future__ import annotations

import argparse
import random
import tomllib
from pathlib import Path

from bilbo_evals import dataset
from bilbo_evals.common import Refused, append_jsonl, now, out, read_jsonl, write_jsonl
from bilbo_evals.schema import PROMPT_LABELS, SPLITS, STRATA

PER_STRATUM = 5
PER_LABEL = 10
NOTES = 20
MIN_VALID = 0.95
ALL_IN_TEST = ("supersession", "multi-hop")


def _sheet_path(dir: Path) -> Path:
    return dir / "generation/review/sheet.jsonl"


def _sheet(dir: Path) -> list[dict]:
    path = _sheet_path(dir)
    return read_jsonl(path) if path.is_file() else []


def _seed(dir: Path) -> int:
    path = dir / "generation/config.toml"
    if not path.is_file():
        return 0
    return int(tomllib.loads(path.read_text(encoding="utf-8")).get("seed", 0))


def _splits(split: str) -> list[str]:
    return list(SPLITS) if split == "all" else [split]


def _note_splits(ds: dataset.Dataset) -> dict[str, str]:
    """Split of each project, from the queries and prompts."""
    found = {}
    for row in [*ds.queries, *ds.prompts]:
        if row.get("project"):
            found.setdefault(row["project"], row["split"])
    return found


def _wanted(ds: dataset.Dataset, split: str) -> dict[tuple[str, str], int]:
    """Minimum number of reviewed items per (split, group) given what the dataset holds now."""
    need: dict[tuple[str, str], int] = {}
    for sp in _splits(split):
        for stratum in STRATA:
            n = len([q for q in ds.queries_for(sp) if q["stratum"] == stratum])
            quota = n if (sp == "test" and stratum in ALL_IN_TEST) else min(PER_STRATUM, n)
            if quota:
                need[(sp, stratum)] = quota
        for label in PROMPT_LABELS:
            n = len([p for p in ds.prompts if p["split"] == sp and p["label"] == label])
            if n:
                need[(sp, label)] = min(PER_LABEL, n)
    return need


def _row(item: str, type_: str, split: str, group: str, text: str, gold=(), decoys=(), evidence=()) -> dict:
    return {
        "item": item, "type": type_, "split": split, "group": group, "text": text, "gold": list(gold),
        "decoys": list(decoys), "evidence_sets": [list(e) for e in evidence],
        "verdict": None, "reason": None, "resolution": None, "reviewer": None,
    }


def _draw(ds: dataset.Dataset, split: str, seed: int) -> list[dict]:
    rng = random.Random(seed)
    drawn = []
    for sp in _splits(split):
        for stratum in STRATA:
            pool = sorted((q for q in ds.queries_for(sp) if q["stratum"] == stratum), key=lambda q: q["id"])
            quota = len(pool) if (sp == "test" and stratum in ALL_IN_TEST) else min(PER_STRATUM, len(pool))
            for q in rng.sample(pool, quota):
                drawn.append(_row(q["id"], "query", sp, stratum, q["text"], q["gold"], q["decoys"], q["evidence_sets"]))
        for label in PROMPT_LABELS:
            pool = sorted((p for p in ds.prompts if p["split"] == sp and p["label"] == label), key=lambda p: p["id"])
            for p in rng.sample(pool, min(PER_LABEL, len(pool))):
                drawn.append(_row(p["id"], "prompt", sp, label, p["prompt"], p["gold"]))
    splits_of = _note_splits(ds)
    candidates = sorted(
        (n for n in ds.notes.values() if split == "all" or splits_of.get(n.project) == split), key=lambda n: n.id
    )
    target = NOTES if split == "all" else NOTES // 2
    for n in rng.sample(candidates, min(target, len(candidates))):
        drawn.append(_row(n.id, "note", splits_of.get(n.project, ""), "note", n.text))
    return drawn


def sample(dir: Path, split: str = "all") -> tuple[int, int]:
    """Append the drawn items the sheet lacks; returns (new items, items in the sheet)."""
    dataset.refuse_if_frozen(dir)
    ds = dataset.load(dir)
    rows = _sheet(dir)
    have = {r["item"] for r in rows}
    new = [r for r in _draw(ds, split, _seed(dir)) if r["item"] not in have]
    write_jsonl(_sheet_path(dir), [*rows, *new])
    return len(new), len(rows) + len(new)


def evaluate(dir: Path, split: str = "all") -> tuple[str, list[str]]:
    """The share line, and the open items; the review passes when there are none."""
    ds = dataset.load(dir)
    rows = [r for r in _sheet(dir) if split == "all" or r["split"] == split or r["type"] == "note"]
    reviewed = [r for r in rows if r["verdict"] in ("valid", "invalid")]
    valid = len([r for r in reviewed if r["verdict"] == "valid"])
    pct = 100.0 * valid / len(reviewed) if reviewed else 0.0
    summary = f"{valid}/{len(reviewed)} valid ({pct:.1f}%)"
    open_items = []
    for r in rows:
        if r["verdict"] is None:
            open_items.append(f"open: {r['item']} not reviewed")
        elif r["verdict"] == "invalid":
            if not r["reason"]:
                open_items.append(f"open: {r['item']} invalid with no reason")
            if r["resolution"] not in ("fixed", "dropped"):
                open_items.append(f"open: {r['item']} invalid and neither fixed nor dropped")
    counts: dict[tuple[str, str], int] = {}
    for r in rows:
        counts[(r["split"], r["group"])] = counts.get((r["split"], r["group"]), 0) + 1
    for (sp, group), need in sorted(_wanted(ds, split).items()):
        if counts.get((sp, group), 0) < need:
            open_items.append(f"open: {sp} {group} has {counts.get((sp, group), 0)} of {need} sampled items")
    if split == "all":
        notes = len([r for r in rows if r["type"] == "note"])
        if notes < min(NOTES, len(ds.notes)):
            open_items.append(f"open: notes has {notes} of {min(NOTES, len(ds.notes))} sampled items")
    if not reviewed:
        open_items.append("open: no item reviewed")
    elif pct < 100 * MIN_VALID:
        open_items.append(f"open: {pct:.1f}% valid is under {100 * MIN_VALID:.0f}%")
    return summary, open_items


def cmd_sample(args: argparse.Namespace) -> int:
    new, total = sample(args.dataset, args.split)
    out(f"{new} new items, {total} in {_sheet_path(args.dataset).relative_to(args.dataset)}")
    return 0


def cmd_check(args: argparse.Namespace) -> int:
    summary, open_items = evaluate(args.dataset, args.split)
    out(summary)
    for line in open_items:
        out(line)
    return 1 if open_items else 0


def cmd_apply(args: argparse.Namespace) -> int:
    dir = args.dataset
    dataset.refuse_if_frozen(dir)
    log = dir / "generation/review/applied.jsonl"
    done = {r["item"] for r in read_jsonl(log)} if log.is_file() else set()
    dropped = [r for r in _sheet(dir) if r["resolution"] == "dropped" and r["item"] not in done]
    notes = [r["item"] for r in dropped if r["type"] == "note"]
    if notes:
        raise Refused("a note cannot be dropped by the review; regenerate the dataset: " + ", ".join(notes))
    ds = dataset.load(dir)
    gone = {r["item"] for r in dropped}
    if gone:
        write_jsonl(dir / "queries.jsonl", [q for q in ds.queries if q["id"] not in gone])
        write_jsonl(dir / "digest/prompts.jsonl", [p for p in ds.prompts if p["id"] not in gone])
    for r in dropped:
        append_jsonl(log, {"item": r["item"], "type": r["type"], "action": "dropped", "reason": r["reason"], "time": now()})
    dataset.build_qrels(dir)
    out(f"dropped {len(dropped)} items")
    return 0
