"""bilbo-keyword and bilbo-full: `bilbo recall --limit 100` in the sandbox, with or without the pinned embedder."""

from __future__ import annotations

import re

from bilbo_evals import embedder, sandbox
from bilbo_evals.arms import LIMIT, Context, Result, library_mode
from bilbo_evals.arms.dense_ref import ensure_index
from bilbo_evals.common import Refused, tool_version

NOTE_HEADER = re.compile(r"^(?P<path>.+\.md):\d+\t[a-z]+\t")
SOURCE_HEADER = re.compile(r"^(?P<path>.+\.md):\d+\t(?:source|guide)\t(?P<ref>[^\t]+)\t")
EMPTY = ("no notes match", "no sources match")
FALLBACK = ("embedder unavailable", "not indexed")


def parse(stdout: str, ctx: Context, library: bool) -> tuple[list[str], list[str]]:
    """Ids of the hits in order, once each, and the header paths no id was found for."""
    ranking: list[str] = []
    unknown: list[str] = []
    for line in stdout.splitlines():
        m = (SOURCE_HEADER if library else NOTE_HEADER).match(line)
        if not m:
            continue
        ident = m["ref"] if library else ctx.path_to_id.get(m["path"])
        if ident is None:
            unknown.append(m["path"])
        elif ident not in ranking:
            ranking.append(ident)
    return ranking, unknown


class Bilbo:
    def __init__(self, name: str, full: bool) -> None:
        self.name, self.full = name, full

    def _config(self, ctx: Context) -> None:
        sandbox.write_config(ctx.sb, ctx.server.url if self.full else None)

    def prepare(self, ctx: Context) -> dict:
        if ctx.sb is None or ctx.bilbo is None:
            raise Refused(f"{self.name} needs a sandbox and a bilbo binary")
        versions = {"bilbo": tool_version([str(ctx.bilbo)])}
        if not self.full:
            self._config(ctx)
            return {"parity": None, "index": None, "embedder": None, "bilbo_config": {}, "versions": versions}
        if ctx.server is None:
            raise Refused("bilbo-full needs the embedder server")
        index = ensure_index(ctx)
        self._config(ctx)
        return {"parity": "ok", "index": {"embedded": index["embedded"], "inputs": index["inputs"]},
                "embedder": embedder.record(ctx.server), "bilbo_config": {"embedder.model": embedder.MODEL},
                "versions": versions}

    def rank(self, item: dict, ctx: Context) -> Result:
        library = library_mode(item)
        args = ["recall", "--limit", str(LIMIT)]
        if item.get("kind") and not library:
            args += ["--kind", item["kind"]]
        if library:
            args.append("--library")
        self._config(ctx)
        p = sandbox.bilbo(ctx.sb, ctx.bilbo, [*args, "--", item["text"]])
        warnings = [l.removeprefix("bilbo: ") for l in p.stderr.splitlines() if l.strip()]
        fallback = any(w in p.stderr for w in FALLBACK)
        if p.exit == 1 and any(e in p.stderr for e in EMPTY):
            return Result([], p.ms, p.exit, warnings, fallback)
        if p.exit != 0:
            return Result([], p.ms, p.exit, warnings, fallback, error=f"exit {p.exit}: {p.stderr.strip()}")
        ranking, unknown = parse(p.stdout, ctx, library)
        warnings += [f"no id for {path}" for path in unknown]
        return Result(ranking[:LIMIT], p.ms, p.exit, warnings, fallback)


keyword = Bilbo("bilbo-keyword", full=False)
full = Bilbo("bilbo-full", full=True)
