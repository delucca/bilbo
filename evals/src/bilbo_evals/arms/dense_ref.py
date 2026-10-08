"""dense-ref: notes ordered by the best cosine of their passages to the query, no floor, vectors from the index run."""

from __future__ import annotations

from pathlib import Path

import numpy as np

from bilbo_evals import embedder, passages
from bilbo_evals.arms import Context, Result, library_mode, restrict
from bilbo_evals.common import Refused

QUERY_BYTES = 2000


def ensure_index(ctx: Context) -> dict:
    """Index the sandbox's store through the checking proxy once per run; refuse unless every passage was embedded as bilbo does."""
    if ctx.index is None:
        if ctx.sb is None or ctx.bilbo is None or ctx.server is None:
            raise Refused("this arm needs a sandbox, a bilbo binary and an embedder server")
        ctx.index = embedder.index_and_check(ctx.sb, ctx.bilbo, ctx.server, ctx.ds)
    index = ctx.index
    if index.get("parity") != "ok":
        shown = "; ".join(repr(d["input"][:60]) for d in index.get("differences", [])[:3])
        raise Refused(f"parity failed: {len(index.get('differences', []))} inputs differ ({shown}); not scoring")
    for key in ("withheld", "skipped"):
        if index.get(key):
            raise Refused(f"bilbo index reported {key} passages; not scoring")
    return index


def _unit(vectors) -> np.ndarray:
    m = np.asarray(vectors, dtype=np.float32)
    norms = np.linalg.norm(m, axis=1, keepdims=True)
    return m / np.where(norms == 0, 1.0, norms)


class DenseRef:
    name = "dense-ref"

    def __init__(self) -> None:
        self.cache: embedder.VectorCache | None = None
        self.docs: dict[bool, dict[str, np.ndarray]] = {}

    def prepare(self, ctx: Context) -> dict:
        index = ensure_index(ctx)
        server = ctx.server
        self.cache = embedder.VectorCache(server.cache_dir())
        store = ctx.ds.dir / "store"
        self.docs = {False: self._vectors({n.id: store / "notes" / n.file for n in ctx.ds.notes.values()}, ctx, strict=True)}
        files = {s.ref: store / s.file for s in ctx.ds.sources.values()}
        files |= {g.parent.name: g for g in sorted((store / "library").glob("*/guide.md"))}
        if files and ctx.ds.queries_for(ctx.split, library=True):
            self.docs[True] = self._vectors(files, ctx, strict=False)
        return {"parity": "ok", "index": {"embedded": index["embedded"], "inputs": index["inputs"]},
                "embedder": embedder.record(server), "bilbo_config": {}, "versions": {}}

    def _vectors(self, files: dict[str, Path], ctx: Context, strict: bool) -> dict[str, np.ndarray]:
        per_doc: dict[str, list[str]] = {}
        for ident, path in files.items():
            text = path.read_bytes().decode("utf-8", errors="replace")
            per_doc[ident] = [t for p in passages.passages(text, path.stem) if (t := passages.embed_input(p)) is not None]
        missing = sorted({t for ts in per_doc.values() for t in ts if self.cache.get(t) is None})
        if missing and strict:
            raise Refused(f"{len(missing)} passage vectors are missing from the cache; the index run did not store them")
        for i in range(0, len(missing), embedder.BATCH):
            batch = missing[i : i + embedder.BATCH]
            for text, vec in zip(batch, ctx.server.embed(batch)):
                self.cache.put(text, vec)
        return {i: _unit([self.cache.get(t) for t in ts]) for i, ts in per_doc.items() if ts}

    def _query(self, text: str, ctx: Context) -> np.ndarray:
        data = (embedder.QUERY_PREFIX + text).encode()[:QUERY_BYTES].decode("utf-8", errors="ignore")
        vec = self.cache.get(data)
        if vec is None:
            (vec,) = ctx.server.embed([data])
            self.cache.put(data, vec)
            vec = self.cache.get(data)
        return _unit([vec])[0]

    def rank(self, item: dict, ctx: Context) -> Result:
        docs = self.docs.get(library_mode(item))
        if not docs:
            return Result([])
        q = self._query(item["text"], ctx)
        best = {ident: float((m @ q).max()) for ident, m in docs.items()}
        ranking = sorted(best, key=lambda i: (-best[i], i))
        return Result(restrict(ranking, item, ctx))


arm = DenseRef()
