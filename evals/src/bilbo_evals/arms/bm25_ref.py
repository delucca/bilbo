"""bm25-ref: BM25 (Lucene variant) with Snowball stemming over whole notes, zero-score documents dropped."""

from __future__ import annotations

import bm25s
import Stemmer

from bilbo_evals.arms import LIMIT, Context, Result, candidates, library_mode, restrict

_STEMMER = Stemmer.Stemmer("english")


def _tokens(texts: list[str], ids: bool):
    return bm25s.tokenize(texts, stopwords="en", stemmer=_STEMMER, show_progress=False, return_ids=ids)


class Bm25Ref:
    name = "bm25-ref"

    def __init__(self) -> None:
        self.indexes: dict[bool, tuple[list[str], bm25s.BM25]] = {}

    def prepare(self, ctx: Context) -> dict:
        self.indexes = {}
        for library, probe in ((False, {"stratum": "known-item"}), (True, {"stratum": "library"})):
            docs = candidates(probe, ctx)
            if not docs:
                continue
            ids = sorted(docs)
            retriever = bm25s.BM25(method="lucene", k1=1.2, b=0.75)
            retriever.index(_tokens([docs[i] for i in ids], True), show_progress=False)
            self.indexes[library] = (ids, retriever)
        return {"parity": None, "index": None, "embedder": None, "bilbo_config": {},
                "versions": {"bm25s": bm25s.__version__}}

    def rank(self, item: dict, ctx: Context) -> Result:
        entry = self.indexes.get(library_mode(item))
        if entry is None:
            return Result([])
        ids, retriever = entry
        query = _tokens([item["text"]], False)
        if not query or not query[0]:
            return Result([])
        documents, scores = retriever.retrieve(query, k=min(LIMIT, len(ids)), show_progress=False)
        ranking = [ids[int(d)] for d, s in zip(documents[0], scores[0]) if s > 0]
        return Result(restrict(ranking, item, ctx))


arm = Bm25Ref()
