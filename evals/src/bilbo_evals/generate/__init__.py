"""Helpers shared by the generation steps: templates, schemas, the stratum needs and a call loop with re-rolls."""

from __future__ import annotations

import json
import math
import re
from dataclasses import dataclass, field
from pathlib import Path
from string import Template
from typing import Callable

from bilbo_evals import common, llm
from bilbo_evals.common import Refused

HERE = Path(__file__).resolve().parent
NOTE_STRATA = ["known-item", "paraphrase", "pt-en", "alias", "supersession", "multi-hop", "kind-filter"]


def template(name: str) -> Template:
    return Template((HERE / "templates" / name).read_text(encoding="utf-8"))


def sections(name: str) -> dict[str, Template]:
    """A template file cut at its `## <word>` lines."""
    text = (HERE / "templates" / name).read_text(encoding="utf-8")
    parts = re.split(r"^## (\S+)\n", text, flags=re.M)
    return {parts[i]: Template(parts[i + 1].strip("\n")) for i in range(1, len(parts), 2)}


def schema(name: str) -> dict:
    return json.loads((HERE / "schemas" / name).read_text(encoding="utf-8"))


def read_json(path: Path, hint: str):
    try:
        return json.loads(Path(path).read_text(encoding="utf-8"))
    except FileNotFoundError as e:
        raise Refused(f"{Path(path).name} is missing: {hint}") from e


def needs(cfg: llm.GenConfig) -> dict[str, dict[str, int]]:
    """Note queries per split and stratum: the dev counts, and the test counts in dev proportions up to max_test_note_queries."""
    dev = {s: int(cfg.strata["dev"].get(s, 0)) for s in NOTE_STRATA}
    total = sum(dev.values()) or 1
    scale = int(cfg.world["max_test_note_queries"]) / total
    return {"dev": dev, "test": {s: math.ceil(n * scale) if n else 0 for s, n in dev.items()}}


@dataclass
class Solved:
    good: dict[str, dict] = field(default_factory=dict)
    unresolved: list[str] = field(default_factory=list)
    stop: Exception | None = None


def solve(
    ds: Path, cfg: llm.GenConfig, step: str, cli: str, wants: dict[str, tuple[str, dict]],
    valid: Callable[[str, dict], str | None], attempts: int = 3,
) -> Solved:
    """Call for every item until an output passes `valid`; a failing output is re-rolled as the next attempt.

    The first valid output of an item (attempt 1 upward) wins, so a rerun after a stop continues where it ended.
    """
    out = Solved()
    dead: set[str] = set()
    preflighted = False
    for _ in range(attempts):
        calls = []
        for item, (prompt, schema_) in wants.items():
            if item in out.good or item in dead:
                continue
            attempt = None
            for a in range(1, attempts + 1):
                path = llm.output_path(ds, step, item, a)
                if not path.is_file():
                    attempt = a
                    break
                value = json.loads(path.read_text(encoding="utf-8"))
                problem = valid(item, value)
                if problem is None:
                    out.good[item] = value
                    break
                common.err(f"{step}/{item} attempt {a}: {problem}")
            if item not in out.good and attempt is not None:
                calls.append(llm.Call(step, item, cli, prompt, schema_, attempt))
            elif item not in out.good:
                dead.add(item)
        if not calls or out.stop:
            break
        if not preflighted:
            llm.preflight(cli, ds, cfg, step)
            preflighted = True
        results = llm.run_many(calls, ds, cfg)
        out.stop = llm.fatal(results)
        dead.update(k for k in llm.failed(results) if not isinstance(results[k], Refused))
    for item, (prompt, schema_) in wants.items():
        if item in out.good:
            continue
        for a in range(1, attempts + 1):
            path = llm.output_path(ds, step, item, a)
            if path.is_file():
                value = json.loads(path.read_text(encoding="utf-8"))
                if valid(item, value) is None:
                    out.good[item] = value
                    break
    out.unresolved = [i for i in wants if i not in out.good]
    return out


def finish(step: str, solved: Solved, total: int, noun: str) -> None:
    """Report what is left and raise the stop (budget, limit, leak) or a Refused for items that kept failing."""
    if solved.stop is not None:
        common.err(f"{len(solved.unresolved)} of {total} {noun} left")
        raise solved.stop
    if solved.unresolved:
        raise Refused(f"{step}: {len(solved.unresolved)} of {total} {noun} still fail after the re-rolls: "
                      + ", ".join(solved.unresolved[:8]))
