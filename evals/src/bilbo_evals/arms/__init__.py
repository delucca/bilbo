"""The six arms and what they share: the context a run builds, a ranking result, the candidate documents of an item."""

from __future__ import annotations

import importlib
from dataclasses import dataclass, field
from pathlib import Path
from typing import Protocol

from bilbo_evals.common import Refused
from bilbo_evals.dataset import Dataset
from bilbo_evals.embedder import Server
from bilbo_evals.sandbox import Sandbox

ARMS = ["random", "ripgrep", "bm25-ref", "dense-ref", "bilbo-keyword", "bilbo-full"]
PROCESS_ARMS = {"ripgrep", "bilbo-keyword", "bilbo-full"}
LIMIT = 100


@dataclass
class Context:
    ds: Dataset
    split: str
    sb: Sandbox | None
    bilbo: Path | None
    server: Server | None
    path_to_id: dict[str, str]
    seeds: range = range(20)
    index: dict | None = None  # the index_and_check record; the first arm that needs it fills it in


@dataclass
class Result:
    ranking: list[str]
    latency_ms: float | None = None
    exit: int | None = None
    warnings: list[str] = field(default_factory=list)
    fallback: bool = False
    error: str | None = None


class Arm(Protocol):
    name: str

    def prepare(self, ctx: Context) -> dict: ...

    def rank(self, item: dict, ctx: Context) -> Result | list[Result]: ...


def get(name: str) -> Arm:
    if name not in ARMS:
        raise Refused(f"no arm named {name!r}; the arms are {', '.join(ARMS)}")
    return importlib.import_module(f"bilbo_evals.arms.{name.replace('-', '_')}").arm


def library_mode(item: dict) -> bool:
    return item["stratum"] == "library"


def store_root(ctx: Context) -> Path:
    return ctx.sb.store if ctx.sb is not None else ctx.ds.dir / "store"


def library_files(ctx: Context) -> dict[str, str]:
    """Id to text of every library file of the dataset: sources by reference, a corpus guide by the corpus name."""
    docs = {ref: s.text for ref, s in ctx.ds.sources.items()}
    for guide in sorted((ctx.ds.dir / "store/library").glob("*/guide.md")):
        docs[guide.parent.name] = guide.read_text(encoding="utf-8")
    return docs


def candidates(item: dict, ctx: Context) -> dict[str, str]:
    """Id to text of what a non-bilbo arm ranks for `item`, notes narrowed to the asked kind."""
    if library_mode(item):
        return library_files(ctx)
    kind = item.get("kind")
    return {n.id: f"{n.title}\n{n.text}" for n in ctx.ds.notes.values() if not kind or n.kind == kind}


def restrict(ranking: list[str], item: dict, ctx: Context) -> list[str]:
    """`ranking` without the notes of another kind than the item asks for, cut to LIMIT."""
    kind = item.get("kind")
    if kind and not library_mode(item):
        ranking = [i for i in ranking if ctx.ds.notes[i].kind == kind]
    return ranking[:LIMIT]
