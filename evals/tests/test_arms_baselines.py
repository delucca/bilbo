"""random, ripgrep and bm25-ref on the fixture: what they rank, the kind filter, zero scores dropped."""

from __future__ import annotations

import re
from pathlib import Path

import pytest

from arms_helpers import make_context, note_id, query
from bilbo_evals import arms
from bilbo_evals.common import Refused

DOCS = Path(__file__).resolve().parents[1] / "docs/arms.md"


@pytest.fixture
def ctx(fixture_dir, monkeypatch, tmp_path):
    return make_context(fixture_dir, monkeypatch, tmp_path)


def test_arms_documented():
    text = DOCS.read_text(encoding="utf-8")
    sections = {l.removeprefix("## ").strip() for l in text.splitlines() if l.startswith("## ")}
    assert set(arms.ARMS) <= sections


def test_every_arm_resolves():
    assert [arms.get(name).name for name in arms.ARMS] == arms.ARMS


def test_an_unknown_arm_is_refused():
    with pytest.raises(Refused, match="no arm named"):
        arms.get("nope")


def test_random_gives_one_ranking_per_seed(ctx):
    arm = arms.get("random")
    arm.prepare(ctx)
    item = query(ctx.ds, "q-alpha-001")
    results = arm.rank(item, ctx)
    assert len(results) == 20
    assert all(sorted(r.ranking) == sorted(ctx.ds.notes) for r in results)
    assert len({tuple(r.ranking) for r in results}) > 1
    assert [r.ranking for r in results] == [r.ranking for r in arm.rank(item, ctx)]
    assert all(r.latency_ms is None for r in results)


def test_random_keeps_the_asked_kind_and_ranks_sources_for_library(ctx):
    arm = arms.get("random")
    kind_item = query(ctx.ds, "q-alpha-007")
    for r in arm.rank(kind_item, ctx):
        assert r.ranking and all(ctx.ds.notes[i].kind == "reference" for i in r.ranking)
    for r in arm.rank(query(ctx.ds, "q-lib-001"), ctx):
        assert sorted(r.ranking) == ["demo", "demo/busy", "demo/wal"]


def test_ripgrep_orders_by_distinct_words_then_matches(ctx):
    arm = arms.get("ripgrep")
    info = arm.prepare(ctx)
    assert info["versions"]["ripgrep"].startswith("ripgrep")
    result = arm.rank(query(ctx.ds, "q-alpha-001"), ctx)
    assert result.ranking[0] == note_id(ctx.ds, "gotcha-edge-cache-eviction.md")
    assert result.error is None and result.latency_ms > 0


def test_ripgrep_orders_like_a_manual_count(ctx):
    arm = arms.get("ripgrep")
    arm.prepare(ctx)
    result = arm.rank({"text": "edge cache ports", "stratum": "known-item", "kind": None}, ctx)
    scores = {}
    for n in ctx.ds.notes.values():
        text = (ctx.sb.store / "notes" / n.file).read_text().lower()
        counts = [len(re.findall(rf"(?<![a-z0-9]){w}(?![a-z0-9])", text)) for w in ("edge", "cache", "ports")]
        if any(counts):
            scores[n.id] = (-sum(c > 0 for c in counts), -sum(counts), n.file)
    assert result.ranking == sorted(scores, key=scores.get)
    assert len(result.ranking) > 2


def test_ripgrep_kind_and_library(ctx):
    arm = arms.get("ripgrep")
    arm.prepare(ctx)
    result = arm.rank(query(ctx.ds, "q-alpha-007"), ctx)
    assert result.ranking == [note_id(ctx.ds, "reference-edge-cache-ports.md")]
    lib = arm.rank(query(ctx.ds, "q-lib-001"), ctx)
    assert lib.ranking[0] == "demo/wal"
    assert all(i.startswith("demo") for i in lib.ranking)


def test_ripgrep_with_no_match_is_empty(ctx):
    arm = arms.get("ripgrep")
    arm.prepare(ctx)
    result = arm.rank({"text": "zzzqqq xxyyzz", "stratum": "no-answer", "kind": None}, ctx)
    assert result.ranking == [] and result.error is None


def test_bm25_ranks_the_gold_first_and_drops_zero_scores(ctx):
    arm = arms.get("bm25-ref")
    arm.prepare(ctx)
    result = arm.rank(query(ctx.ds, "q-alpha-001"), ctx)
    assert result.ranking[0] == note_id(ctx.ds, "gotcha-edge-cache-eviction.md")
    assert len(result.ranking) < len(ctx.ds.notes)
    assert result.latency_ms is None
    assert arm.rank({"text": "zzzqqq", "stratum": "no-answer", "kind": None}, ctx).ranking == []
    assert arm.rank({"text": "the of and", "stratum": "no-answer", "kind": None}, ctx).ranking == []


def test_bm25_kind_filter_removes_other_kinds(ctx):
    arm = arms.get("bm25-ref")
    arm.prepare(ctx)
    item = query(ctx.ds, "q-alpha-007")
    unfiltered = arm.rank({**item, "kind": None}, ctx).ranking
    assert any(ctx.ds.notes[i].kind != "reference" for i in unfiltered)
    filtered = arm.rank(item, ctx).ranking
    assert filtered == [i for i in unfiltered if ctx.ds.notes[i].kind == "reference"]


def test_bm25_ranks_sources_for_library(ctx):
    arm = arms.get("bm25-ref")
    arm.prepare(ctx)
    result = arm.rank(query(ctx.ds, "q-lib-002"), ctx)
    assert result.ranking[0] == "demo/busy"
