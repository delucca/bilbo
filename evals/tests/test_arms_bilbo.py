"""dense-ref and the bilbo arms against the real bilbo binary on the fixture, with the fake embedder."""

from __future__ import annotations

import stat

import numpy as np
import pytest

from arms_helpers import make_context, note_id, query
from bilbo_evals import arms, embedder, passages
from bilbo_evals.common import Refused
from fake_embedder import embed


@pytest.fixture
def ctx(fixture_dir, bilbo_bin, fake_embedder, monkeypatch, tmp_path):
    return make_context(fixture_dir, monkeypatch, tmp_path, bilbo=bilbo_bin, fake=fake_embedder)


def script(tmp_path, body: str):
    path = tmp_path / "fake-bilbo"
    path.write_text("#!/bin/sh\n" + body, encoding="utf-8")
    path.chmod(path.stat().st_mode | stat.S_IEXEC)
    return path


def test_keyword_finds_the_gold_and_times_the_process(ctx):
    arm = arms.get("bilbo-keyword")
    info = arm.prepare(ctx)
    assert info["bilbo_config"] == {} and info["embedder"] is None and info["parity"] is None
    assert "embedder" not in ctx.sb.config.read_text()
    result = arm.rank(query(ctx.ds, "q-alpha-001"), ctx)
    assert result.ranking[0] == note_id(ctx.ds, "gotcha-edge-cache-eviction.md")
    assert result.exit == 0 and result.error is None and not result.fallback and result.latency_ms > 0
    assert len(result.ranking) == len(set(result.ranking))


def test_nothing_matching_is_an_empty_ranking_without_error(ctx):
    arm = arms.get("bilbo-keyword")
    arm.prepare(ctx)
    result = arm.rank({"text": "zzzqqq", "stratum": "no-answer", "kind": None}, ctx)
    assert (result.ranking, result.exit, result.error) == ([], 1, None)
    lib = arm.rank({"text": "zzzqqq", "stratum": "library", "kind": None}, ctx)
    assert (lib.ranking, lib.exit, lib.error) == ([], 1, None)


def test_kind_and_library_flags(ctx):
    arm = arms.get("bilbo-keyword")
    arm.prepare(ctx)
    result = arm.rank(query(ctx.ds, "q-alpha-007"), ctx)
    assert result.ranking == [note_id(ctx.ds, "reference-edge-cache-ports.md")]
    lib = arm.rank(query(ctx.ds, "q-lib-001"), ctx)
    assert lib.ranking[0] == "demo/wal"


def test_a_crash_is_the_querys_error(ctx, tmp_path):
    ctx.bilbo = script(tmp_path, "echo 'thread panicked' >&2\nexit 101\n")
    result = arms.get("bilbo-keyword").rank(query(ctx.ds, "q-alpha-001"), ctx)
    assert result.ranking == [] and result.exit == 101
    assert result.error == "exit 101: thread panicked"


def test_a_keyword_fallback_is_marked(ctx, tmp_path):
    path = next(iter(ctx.path_to_id))
    ctx.bilbo = script(
        tmp_path,
        "echo 'bilbo: embedder unavailable (unreachable); keyword results only' >&2\n"
        f"printf '%s:6\\treference\\t2026-01-01T00:00-03:00\\ntitle\\ntext\\n' '{path}'\n",
    )
    result = arms.get("bilbo-full").rank(query(ctx.ds, "q-alpha-001"), ctx)
    assert result.fallback and result.error is None and len(result.ranking) == 1
    assert result.warnings == ["embedder unavailable (unreachable); keyword results only"]
    ctx.bilbo = script(tmp_path, "echo 'bilbo: 1 passage not indexed; run bilbo index' >&2\nexit 1\n")
    stale = arms.get("bilbo-keyword").rank(query(ctx.ds, "q-alpha-001"), ctx)
    assert stale.fallback and stale.error is not None


def test_full_indexes_every_note_then_ranks(ctx):
    arm = arms.get("bilbo-full")
    info = arm.prepare(ctx)
    assert info["parity"] == "ok" and info["index"]["inputs"] == len(set(passages.inputs(ctx.sb.store)))
    assert info["embedder"]["model"] == embedder.MODEL
    assert info["bilbo_config"] == {"embedder.model": embedder.MODEL}
    result = arm.rank(query(ctx.ds, "q-alpha-002"), ctx)
    assert result.error is None and not result.fallback and result.ranking
    assert result.latency_ms > 0
    assert ctx.index["parity"] == "ok"


def test_full_marks_a_fallback_when_the_embedder_stops(ctx, fake_embedder):
    arm = arms.get("bilbo-full")
    arm.prepare(ctx)
    fake_embedder.stop()
    result = arm.rank(query(ctx.ds, "q-alpha-002"), ctx)
    assert result.fallback is True and result.exit == 0


def test_keyword_and_full_share_one_sandbox(ctx):
    keyword, full = arms.get("bilbo-keyword"), arms.get("bilbo-full")
    keyword.prepare(ctx)
    full.prepare(ctx)
    keyword.rank(query(ctx.ds, "q-alpha-001"), ctx)
    assert "embedder" not in ctx.sb.config.read_text()
    full.rank(query(ctx.ds, "q-alpha-001"), ctx)
    assert "embedder.url" in ctx.sb.config.read_text()


def test_dense_ref_ranks_every_note_by_best_cosine(ctx):
    arm = arms.get("dense-ref")
    info = arm.prepare(ctx)
    assert info["parity"] == "ok" and info["embedder"]["gguf_sha256"] is None
    item = query(ctx.ds, "q-alpha-002")
    result = arm.rank(item, ctx)
    assert len(result.ranking) == len(ctx.ds.notes) and result.latency_ms is None
    q = np.array(embed(embedder.QUERY_PREFIX + item["text"]))
    best = {}
    for n in ctx.ds.notes.values():
        text = (ctx.sb.store / "notes" / n.file).read_text()
        vs = [np.array(embed(t)) for p in passages.passages(text, n.file[:-3]) if (t := passages.embed_input(p))]
        best[n.id] = max(float(v @ q) for v in vs) if vs else float("-inf")
    assert result.ranking == sorted(best, key=lambda i: (-best[i], i))


def test_dense_ref_kind_filter_and_library(ctx):
    arm = arms.get("dense-ref")
    arm.prepare(ctx)
    result = arm.rank(query(ctx.ds, "q-alpha-007"), ctx)
    assert result.ranking and all(ctx.ds.notes[i].kind == "reference" for i in result.ranking)
    lib = arm.rank(query(ctx.ds, "q-lib-001"), ctx)
    assert sorted(lib.ranking) == ["demo", "demo/busy", "demo/wal"]


@pytest.mark.parametrize("name", ["dense-ref", "bilbo-full"])
def test_a_parity_failure_stops_dense_ref_and_bilbo_full(ctx, monkeypatch, name):
    monkeypatch.setattr(passages, "embed_input", lambda p: (p.text + " x") if p.text else None)
    with pytest.raises(Refused, match="parity failed"):
        arms.get(name).prepare(ctx)
    assert ctx.index["parity"] == "failed"
    with pytest.raises(Refused, match="parity failed"):
        arms.get("dense-ref" if name == "bilbo-full" else "bilbo-full").prepare(ctx)
    arms.get("bilbo-keyword").prepare(ctx)


def test_a_recorded_failed_parity_is_never_scored(ctx):
    ctx.index = {"embedded": 3, "inputs": 4, "parity": "failed", "differences": [{"side": "harness-only", "input": "lost passage"}]}
    for name in ("dense-ref", "bilbo-full"):
        with pytest.raises(Refused, match="harness-only|parity failed"):
            arms.get(name).prepare(ctx)


def test_a_withheld_passage_stops_the_arms(ctx, monkeypatch):
    def withheld(*a, **k):
        raise Refused("withheld 2 passages from a store: their scope allows only a loopback embedder")

    monkeypatch.setattr(embedder, "index_and_check", withheld)
    for name in ("dense-ref", "bilbo-full"):
        ctx.index = None
        with pytest.raises(Refused, match="withheld 2 passages"):
            arms.get(name).prepare(ctx)
    ctx.index = {"embedded": 4, "inputs": 4, "parity": "ok", "differences": [], "skipped": 1}
    with pytest.raises(Refused, match="skipped"):
        arms.get("bilbo-full").prepare(ctx)


def test_dense_ref_refuses_a_missing_vector(ctx):
    ctx.index = {"embedded": 4, "inputs": 4, "parity": "ok", "differences": []}
    with pytest.raises(Refused, match="vectors are missing"):
        arms.get("dense-ref").prepare(ctx)
