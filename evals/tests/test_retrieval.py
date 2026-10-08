"""TREC files, ir_measures scores, missing queries and the extra metrics."""

from __future__ import annotations

import math

import ir_measures
import pytest

from bilbo_evals import retrieval
from bilbo_evals.common import Refused

QRELS = [
    ir_measures.Qrel("q1", "a", 1), ir_measures.Qrel("q1", "b", 0), ir_measures.Qrel("q1", "c", 0),
    ir_measures.Qrel("q2", "x", 1), ir_measures.Qrel("q3", "m", 1),
]


def test_trec_scores_fall_strictly_with_rank(tmp_path):
    path = tmp_path / "run.trec"
    retrieval.write_trec(path, {"q2": ["z", "a"], "q1": ["b", "a", "c"], "q0": []}, "bm25-ref")
    rows = [l.split() for l in path.read_text().splitlines()]
    assert rows[0] == ["q1", "Q0", "b", "1", "999", "bm25-ref"]
    assert [r[0] for r in rows] == ["q1", "q1", "q1", "q2", "q2"]
    q1 = [float(r[4]) for r in rows if r[0] == "q1"]
    assert q1 == sorted(q1, reverse=True) and len(set(q1)) == 3


def test_scores_match_hand_values():
    got = retrieval.score({"q1": ["b", "a", "c"]}, QRELS)["q1"]
    assert got["success@5"] == 1.0 and got["rr"] == 0.5 and got["r@10"] == 1.0
    assert got["ndcg@10"] == pytest.approx(1 / math.log2(3))
    assert got["judged@10"] == 1.0
    assert set(got) == set(retrieval.METRICS)


def test_empty_and_unjudged_rankings_count_as_zero():
    got = retrieval.score({"q1": [], "q2": ["u1", "u2"], "q3": ["m"]}, QRELS)
    assert all(v == 0.0 for v in got["q1"].values())
    assert got["q2"]["success@5"] == 0.0 and got["q2"]["judged@10"] == 0.0
    assert got["q3"]["rr"] == 1.0
    assert set(got) == {"q1", "q2", "q3"}


def test_a_query_without_qrels_still_gets_every_metric():
    got = retrieval.score({"q9": ["a"]}, QRELS)
    assert got["q9"] == {m: 0.0 for m in retrieval.METRICS}


def test_read_qrels_from_the_fixture(fixture_dir):
    notes = retrieval.read_qrels(fixture_dir, "dev", library=False)
    assert {q.query_id for q in notes} >= {"q-alpha-001", "q-alpha-007"}
    assert all(not q.query_id.startswith("q-lib") for q in notes)
    lib = retrieval.read_qrels(fixture_dir, "dev", library=True)
    assert [(q.query_id, q.doc_id, q.relevance) for q in lib] == [("q-lib-001", "demo/wal", 1)]
    both = retrieval.read_qrels(fixture_dir, "all", library=False)
    assert {q.query_id for q in both} > {q.query_id for q in notes}
    with pytest.raises(Refused, match="no qrels"):
        retrieval.read_qrels(fixture_dir / "nope", "dev", library=False)


def test_scoring_a_fixture_ranking(fixture_dir):
    qrels = retrieval.read_qrels(fixture_dir, "dev", library=False)
    gold = next(q.doc_id for q in qrels if q.query_id == "q-alpha-001" and q.relevance > 0)
    got = retrieval.score({"q-alpha-001": ["other", gold]}, qrels)
    assert got["q-alpha-001"]["rr"] == 0.5


def test_evidence_at_10():
    item = {"stratum": "multi-hop", "evidence_sets": [["a", "b"], ["c", "d"]], "gold": [], "decoys": []}
    assert retrieval.extras(item, ["c", "x", "d"])["evidence@10"] == 1.0
    assert retrieval.extras(item, ["a", "c", "d"] + ["f"] * 7 + ["b"])["evidence@10"] == 1.0
    assert retrieval.extras(item, ["a", "c"] + ["f"] * 8 + ["b"])["evidence@10"] == 0.0
    assert retrieval.extras(item, [])["evidence@10"] == 0.0
    assert retrieval.extras({**item, "stratum": "known-item"}, ["a", "b"])["evidence@10"] is None


def test_new_above_old():
    item = {"stratum": "supersession", "gold": ["new"], "decoys": ["old1", "old2"], "evidence_sets": []}
    assert retrieval.extras(item, ["new", "old1"])["new_above_old"] == 1.0
    assert retrieval.extras(item, ["old1", "new"])["new_above_old"] == 0.0
    assert retrieval.extras(item, ["new"])["new_above_old"] == 1.0
    assert retrieval.extras(item, ["old1"])["new_above_old"] == 0.0
    assert retrieval.extras(item, [])["new_above_old"] == 0.0


def test_empty_share():
    item = {"stratum": "no-answer", "gold": [], "decoys": [], "evidence_sets": []}
    assert retrieval.extras(item, [])["empty"] == 1.0
    assert retrieval.extras(item, ["a"])["empty"] == 0.0
    assert retrieval.extras(item, [])["new_above_old"] is None
