"""`generate profile`: rounded aggregate numbers of a store, never a string from it."""

from __future__ import annotations

import re
from pathlib import Path

from bilbo_evals import common, llm
from bilbo_evals.common import Refused
from bilbo_evals.schema import NOTE_KINDS

QUANTILES = {"p10": 0.10, "p25": 0.25, "p50": 0.50, "p75": 0.75, "p90": 0.90}
PT_WORDS = frozenset("de que não para com uma os em é são foi isso como mais mas por dos das se um ao na no você também já".split())
EN_WORDS = frozenset("the and to of is that for with in it this are was not on be as we".split())
WIKI = re.compile(r"\[\[[^\]\n]+\]\]")


def _share(x: float) -> float:
    return round(round(x / 0.05) * 0.05, 2)


def _quantile(sorted_values: list[int], p: float) -> float:
    pos = p * (len(sorted_values) - 1)
    lo = int(pos)
    hi = min(lo + 1, len(sorted_values) - 1)
    return sorted_values[lo] + (sorted_values[hi] - sorted_values[lo]) * (pos - lo)


def _split(text: str) -> tuple[str, str]:
    lines = text.split("\n")
    if lines and lines[0].strip() == "---":
        for i in range(1, len(lines)):
            if lines[i].strip() == "---":
                return "\n".join(lines[1:i]), "\n".join(lines[i + 1:])
    return "", text


def _headings(body: str) -> int:
    count, fenced = 0, False
    for line in body.split("\n"):
        if line.lstrip().startswith("```"):
            fenced = not fenced
        elif not fenced and re.match(r"#{1,6} \S", line):
            count += 1
    return count


def _portuguese(body: str) -> bool:
    words = re.findall(r"[a-zà-ÿ]+", body.lower())
    pt = sum(w in PT_WORDS for w in words)
    return pt >= 3 and pt > sum(w in EN_WORDS for w in words)


def compute(store: Path) -> dict:
    """The profile of `store/notes/*.md`: only counts, shares and quantiles, rounded."""
    notes = Path(store) / "notes"
    if not notes.is_dir():
        raise Refused(f"{store} is not a bilbo store: it has no notes/ folder")
    files = sorted(notes.glob("*.md"))
    if not files:
        raise Refused(f"{store} holds no notes")
    kinds = dict.fromkeys(NOTE_KINDS, 0)
    lengths, headings = [], []
    flags = dict.fromkeys(("code_blocks", "sources", "portuguese", "wiki_links"), 0)
    for path in files:
        with open(path, encoding="utf-8", errors="replace") as f:
            text = f.read()
        front, body = _split(text)
        kind = path.name.split("-", 1)[0]
        if kind in kinds:
            kinds[kind] += 1
        lengths.append(len(text))
        headings.append(_headings(body))
        flags["code_blocks"] += "```" in body
        flags["sources"] += any(line.startswith("sources:") for line in front.split("\n"))
        flags["portuguese"] += _portuguese(body)
        flags["wiki_links"] += bool(WIKI.search(body))
    n = len(files)
    known = sum(kinds.values()) or 1
    lengths.sort()
    headings.sort()
    return {
        "schema_version": 1,
        "notes": int(round(n / 50) * 50),
        "kinds": {k: _share(v / known) for k, v in kinds.items()},
        "length_chars": {k: int(round(_quantile(lengths, p) / 100) * 100) for k, p in QUANTILES.items()},
        "headings": {k: int(round(_quantile(headings, p))) for k, p in QUANTILES.items()},
        "shares": {k: _share(v / n) for k, v in flags.items()},
    }


def cmd(args) -> int:
    ds = Path(args.dataset)
    llm.require_unfrozen(ds)
    store = Path(args.store).expanduser()
    profile = compute(store)
    common.write_json(ds / "world/profile.json", profile)
    common.out("wrote world/profile.json")
    return 0
