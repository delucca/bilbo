"""TREC run and qrels files, ir_measures scores and the extra per-item metrics."""

from __future__ import annotations

from pathlib import Path

import ir_measures
from ir_measures import RR, Judged, Qrel, R, ScoredDoc, Success, nDCG

from bilbo_evals.common import Refused

METRICS = ["success@5", "rr", "ndcg@10", "r@10", "judged@10"]
_MEASURES = {
    "success@5": Success @ 5, "rr": RR, "ndcg@10": nDCG @ 10, "r@10": R @ 10, "judged@10": Judged @ 10,
}


def write_trec(path: Path, rankings: dict[str, list[str]], tag: str) -> None:
    """`<qid> Q0 <docid> <rank> <score> <tag>`; scores fall strictly with rank because trec_eval reorders ties by docid."""
    lines = [
        f"{qid} Q0 {doc} {rank} {1000 - rank} {tag}"
        for qid in sorted(rankings)
        for rank, doc in enumerate(rankings[qid], start=1)
    ]
    path.write_text("".join(f"{l}\n" for l in lines), encoding="utf-8")


def read_qrels(ds_dir: Path, split: str, library: bool) -> list[Qrel]:
    prefix = "library-" if library else ""
    splits = ["dev", "test"] if split == "all" else [split]
    qrels: list[Qrel] = []
    for s in splits:
        path = Path(ds_dir) / "qrels" / f"{prefix}{s}.txt"
        if not path.is_file():
            raise Refused(f"no qrels file at {path}")
        qrels += list(ir_measures.read_trec_qrels(str(path)))
    return qrels


def score(rankings: dict[str, list[str]], qrels: list[Qrel]) -> dict[str, dict[str, float]]:
    """Every queried id gets every metric; an empty or absent ranking scores 0."""
    run = [
        ScoredDoc(qid, doc, float(1000 - rank))
        for qid, docs in rankings.items()
        for rank, doc in enumerate(docs, start=1)
    ]
    out = {qid: {m: 0.0 for m in METRICS} for qid in rankings}
    names = {str(measure): key for key, measure in _MEASURES.items()}
    for metric in ir_measures.iter_calc(list(_MEASURES.values()), qrels, run):
        if metric.query_id in out:
            out[metric.query_id][names[str(metric.measure)]] = float(metric.value)
    return out


def extras(item: dict, ranking: list[str]) -> dict[str, float | None]:
    """Evidence@10 for multi-hop, new-above-old for supersession, and whether the ranking is empty."""
    found: dict[str, float | None] = {"evidence@10": None, "new_above_old": None, "empty": float(not ranking)}
    if item["stratum"] == "multi-hop":
        top = set(ranking[:10])
        found["evidence@10"] = float(any(ev and set(ev) <= top for ev in item["evidence_sets"]))
    if item["stratum"] == "supersession":
        pos = {doc: i for i, doc in enumerate(ranking)}
        gold = min((pos[g] for g in item["gold"] if g in pos), default=None)
        found["new_above_old"] = float(gold is not None and all(d not in pos or pos[d] > gold for d in item["decoys"]))
    return found
