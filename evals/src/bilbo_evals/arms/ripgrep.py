"""ripgrep: notes ordered by distinct query words found, then total matches, then path."""

from __future__ import annotations

import shutil
from collections import Counter
from pathlib import Path

from bilbo_evals import words
from bilbo_evals.arms import Context, Result, library_mode, restrict, store_root
from bilbo_evals.common import Refused, run, tool_version


def _id_of(rel: Path, ctx: Context) -> str | None:
    if rel.parts[0] == "notes":
        return next((n.id for n in ctx.ds.notes.values() if n.file == rel.name), None)
    corpus, name = rel.parts[1], rel.stem
    if name == "guide":
        return corpus
    ref = f"{corpus}/{name}"
    return ref if ref in ctx.ds.sources else None


class Ripgrep:
    name = "ripgrep"

    def __init__(self) -> None:
        self.exe: str | None = None

    def prepare(self, ctx: Context) -> dict:
        self.exe = shutil.which("rg")
        if self.exe is None:
            raise Refused("`rg` is missing; put ripgrep on PATH")
        return {"parity": None, "index": None, "embedder": None, "bilbo_config": {},
                "versions": {"ripgrep": tool_version([self.exe])}}

    def rank(self, item: dict, ctx: Context) -> Result:
        root = store_root(ctx)
        if library_mode(item):
            targets = [str(d) for d in sorted((root / "library").iterdir()) if d.is_dir() and not d.name.startswith(".")]
        else:
            targets = [str(root / "notes")]
        env = ctx.sb.env if ctx.sb is not None else {"PATH": "/usr/bin:/bin"}
        query_words = sorted({w for w in words.words(item["text"]) if w not in words.STOPWORDS})
        found: dict[str, int] = Counter()
        total: dict[str, int] = Counter()
        wall, error = 0.0, None
        for word in query_words:
            p = run([self.exe, "--ignore-case", "--word-regexp", "--fixed-strings", "--count-matches", "--no-ignore",
                     "--with-filename", "-e", word, "--", *targets], env=env)
            wall += p.ms
            if p.exit not in (0, 1):
                error = f"exit {p.exit}: {p.stderr.strip()}"
                continue
            for line in p.stdout.splitlines():
                path, _, count = line.rpartition(":")
                found[path] += 1
                total[path] += int(count)
        ordered = sorted(found, key=lambda path: (-found[path], -total[path], path))
        ranking: list[str] = []
        for path in ordered:
            ident = _id_of(Path(path).relative_to(root), ctx)
            if ident is not None and ident not in ranking:
                ranking.append(ident)
        return Result(restrict(ranking, item, ctx), latency_ms=wall, error=error)


arm = Ripgrep()
