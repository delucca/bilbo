"""pool: the candidates of every arm, the checking calls, the quote rule, the audit and the resolutions."""

from __future__ import annotations

import argparse
import shutil
from pathlib import Path

import pytest

from bilbo_evals import common, dataset, llm, pool, runner
from bilbo_evals.arms import ARMS
from bilbo_evals.common import Refused, read_jsonl, write_jsonl
from gen_e_helpers import CONFIG, DEFAULTS

FIXTURE = Path(__file__).resolve().parent / "fixtures/notes-fixture"
NA, MULTI, NEAR = "q-alpha-na-01", "q-alpha-006", "p-alpha-003"


@pytest.fixture
def ds(tmp_path, monkeypatch) -> Path:
    d = tmp_path / "datasets/notes-synth/v1"
    shutil.copytree(FIXTURE, d)
    (d / "MANIFEST").unlink()
    (d / "FROZEN").unlink()
    (d / "generation/config.toml").write_text(CONFIG.format(**DEFAULTS), encoding="utf-8")
    monkeypatch.setattr(common, "CACHE_DIR", tmp_path / "cache")
    monkeypatch.delenv("CODEX_HOME", raising=False)
    monkeypatch.setattr(llm, "preflight", lambda *a, **k: None)
    (Path.home() / ".codex").mkdir()
    (Path.home() / ".codex/auth.json").write_text("{}", encoding="utf-8")
    return d


def args(ds: Path, bilbo: Path, fake_url: str | None = None, split: str = "dev", apply: bool = False) -> argparse.Namespace:
    return argparse.Namespace(dataset=ds, bilbo=bilbo, model=None, llama_server="llama-server", split=split,
                              apply=apply, embedder_url=fake_url)


def first_line(ds: dataset.Dataset, cid: str) -> str:
    return next(line for line in pool.candidate_text(ds, cid, "x").splitlines() if len(line.strip()) > 10).strip()


def script(fake_llm, ds_dir: Path, yes: dict[str, dict[str, int | None]] | None = None, quote: dict | None = None) -> None:
    """A rule per pooled item: judgments for its candidates; `yes[item][cid]` makes it a yes (value: completes_set)."""
    ds = dataset.load(ds_dir)
    rules = []
    for row in read_jsonl(ds_dir / "generation/pool/candidates.jsonl"):
        out = []
        for c in row["candidates"]:
            said = (yes or {}).get(row["item"], {})
            if c["id"] in said:
                cs = said[c["id"]]
                text = (quote or {}).get((row["item"], c["id"]), first_line(ds, c["id"]))
                out.append({"id": c["id"], "answers": cs is None, "completes_set": cs, "passage": text})
            else:
                out.append({"id": c["id"], "answers": False, "completes_set": None, "passage": None})
        rules.append({"match": f"Item: {row['item']}\n", "output": {"judgments": out}})
    fake_llm.set_script(rules)


@pytest.fixture(scope="module")
def ranked(tmp_path_factory):
    """The candidates of the fixture's dev split from the real arms, ranked once for the module."""
    from fake_embedder import FakeEmbedder
    import os

    base = tmp_path_factory.mktemp("pool-ranked")
    d = base / "datasets/notes-synth/v1"
    shutil.copytree(FIXTURE, d)
    bilbo = Path(os.environ.get("BILBO_BIN") or Path(__file__).resolve().parents[2] / "target/debug/bilbo")
    fake = FakeEmbedder().start()
    try:
        rankings = runner.rank_all(d, "dev", list(ARMS), bilbo, None, None, fake.url)
    finally:
        fake.stop()
    loaded = dataset.load(d)
    items = pool.pooled_items(loaded, "dev")
    return {"rankings": rankings, "rows": [
        {"item": it["id"], "kind": it["kind"], "split": it["split"], "candidates": pool.top_candidates(loaded, it, rankings)}
        for it in items]}


def seed_candidates(ds: Path, ranked) -> None:
    write_jsonl(ds / "generation/pool/candidates.jsonl", ranked["rows"])


def run(ds: Path, bilbo: Path, **kw) -> int:
    return pool.cmd(args(ds, bilbo, **kw))


def audit_done(ds: Path) -> None:
    rows = read_jsonl(ds / "generation/pool/audit.jsonl")
    write_jsonl(ds / "generation/pool/audit.jsonl", [{**r, "verdict": "agree", "reviewer": "x"} for r in rows])


def judgments(ds: Path) -> list[dict]:
    return read_jsonl(ds / "generation/pool/judgments.jsonl")


# --- candidates ---------------------------------------------------------------------------------------------------

def test_candidates_hold_every_arms_top_ten_not_gold(ranked):
    loaded = dataset.load(FIXTURE)
    items = {it["id"]: it for it in pool.pooled_items(loaded, "dev")}
    assert NEAR in items and "p-alpha-001" in items and "q-lib-001" in items
    assert not any(i.startswith("p-none") for i in items) and not any(i.startswith("p-beta") for i in items)
    for row in ranked["rows"]:
        gold = set(items[row["item"]]["row"]["gold"])
        assert gold.isdisjoint(c["id"] for c in row["candidates"])
        for arm in ARMS:
            non_gold = [i for i in ranked["rankings"][arm][row["item"]] if i not in gold and (i in loaded.notes or i in loaded.sources)]
            assert {i for i in non_gold[:10]} <= {c["id"] for c in row["candidates"]}
        for c in row["candidates"]:
            assert 1 <= c["best_rank"] <= 10 and c["arms"] and set(c["arms"]) <= set(ARMS)
    assert any("bilbo-full" in c["arms"] for r in ranked["rows"] for c in r["candidates"])
    assert any("random" in c["arms"] for r in ranked["rows"] for c in r["candidates"])


def test_a_library_query_pools_sources_with_their_best_passage(ranked, ds):
    loaded = dataset.load(ds)
    row = next(r for r in ranked["rows"] if r["item"] == "q-lib-001")
    assert {c["id"] for c in row["candidates"]} <= set(loaded.sources)
    text = pool.candidate_text(loaded, "demo/busy", "busy timeout")
    assert len(text) <= pool.SOURCE_CHARS and "Intro nav line." not in text and "timeout" in text
    assert pool.candidate_text(loaded, "demo/wal", "checkpoint log pages").endswith("It runs when the log passes 1000 pages.")


def test_note_text_is_cut(ds):
    loaded = dataset.load(ds)
    nid = next(iter(loaded.notes))
    loaded.notes[nid].text = "x" * 5000
    assert len(pool.candidate_text(loaded, nid, "q")) == pool.NOTE_CHARS


# --- the checking calls -------------------------------------------------------------------------------------------

def test_cmd_ranks_judges_and_audits(ds, fake_llm, fake_embedder, bilbo_bin, ranked):
    (ds / "generation/pool/candidates.jsonl").write_text("")
    # The rules come from the module's ranking; the command must rank again and find the same candidates.
    fake_llm.set_script([{"match": f"Item: {r['item']}\n", "output": {"judgments": [
        {"id": c["id"], "answers": False, "completes_set": None, "passage": None} for c in r["candidates"]]}}
        for r in ranked["rows"]])
    assert run(ds, bilbo_bin, fake_url=fake_embedder.url) == 0
    got = read_jsonl(ds / "generation/pool/candidates.jsonl")
    assert got == ranked["rows"]
    rows = judgments(ds)
    assert len(rows) == sum(len(r["candidates"]) for r in got)
    assert all(r["answers"] is False and r["flag"] is None and r["call_id"].startswith("pool/") for r in rows)
    audit = read_jsonl(ds / "generation/pool/audit.jsonl")
    assert len(audit) == min(len(rows), pool.AUDIT_MIN) and all(a["verdict"] is None for a in audit)
    calls = len(fake_llm.calls())
    assert calls == sum(1 for r in got if r["candidates"])
    assert run(ds, bilbo_bin, fake_url=fake_embedder.url) == 0 and len(fake_llm.calls()) == calls


def test_a_yes_with_a_verbatim_quote_is_kept(ds, fake_llm, bilbo_bin, ranked):
    seed_candidates(ds, ranked)
    row = next(r for r in ranked["rows"] if r["item"] == "q-alpha-001")
    cid = row["candidates"][0]["id"]
    script(fake_llm, ds, {"q-alpha-001": {cid: None}})
    assert run(ds, bilbo_bin) == 0
    yes = next(j for j in judgments(ds) if (j["item"], j["candidate"]) == ("q-alpha-001", cid))
    assert yes["answers"] is True and yes["quote_found"] is True and yes["flag"] is None and yes["passage"]
    assert dataset.pool_open_items(ds)[0] == f"q-alpha-001 {cid}: pooled yes with no resolution"
    audit_done(ds)
    assert dataset.pool_open_items(ds) == [f"q-alpha-001 {cid}: pooled yes with no resolution"]


@pytest.mark.parametrize("passage", ["words the note never says", None, "   "])
def test_an_unquoted_yes_counts_as_a_no_and_is_flagged(ds, fake_llm, bilbo_bin, ranked, passage):
    seed_candidates(ds, ranked)
    cid = next(r for r in ranked["rows"] if r["item"] == "q-alpha-001")["candidates"][0]["id"]
    script(fake_llm, ds, {"q-alpha-001": {cid: None}}, quote={("q-alpha-001", cid): passage})
    assert run(ds, bilbo_bin) == 0
    bad = next(j for j in judgments(ds) if (j["item"], j["candidate"]) == ("q-alpha-001", cid))
    assert bad["answers"] is True and bad["quote_found"] is False and bad["flag"] == "yes_without_quote"
    audit_done(ds)
    assert dataset.pool_open_items(ds) == []
    dataset.build_qrels(ds)
    assert f"q-alpha-001 0 {cid} 0" in (ds / "qrels/dev.txt").read_text()


def test_a_quote_matches_after_folding_whitespace(ds, fake_llm, bilbo_bin, ranked):
    seed_candidates(ds, ranked)
    cid = next(r for r in ranked["rows"] if r["item"] == "q-alpha-001")["candidates"][0]["id"]
    words = first_line(dataset.load(ds), cid).split()
    script(fake_llm, ds, {"q-alpha-001": {cid: None}}, quote={("q-alpha-001", cid): "  ".join(words[:5]) + "\n"})
    run(ds, bilbo_bin)
    yes = next(j for j in judgments(ds) if (j["item"], j["candidate"]) == ("q-alpha-001", cid))
    assert yes["quote_found"] is True


def test_multi_hop_shows_the_evidence_sets_and_keeps_completes_set(ds, fake_llm, bilbo_bin, ranked):
    seed_candidates(ds, ranked)
    cid = next(r for r in ranked["rows"] if r["item"] == MULTI)["candidates"][0]["id"]
    other = next(r for r in ranked["rows"] if r["item"] == "q-alpha-001")["candidates"][0]["id"]
    script(fake_llm, ds, {MULTI: {cid: 0}, "q-alpha-001": {other: 0}})
    run(ds, bilbo_bin)
    prompt = next(c["prompt"] for c in fake_llm.calls() if f"Item: {MULTI}\n" in c["prompt"])
    assert "Evidence set 0:" in prompt and "01KRE0BGJ00TBN9E758M021ZV5" in prompt
    rows = {(j["item"], j["candidate"]): j for j in judgments(ds)}
    assert rows[(MULTI, cid)]["completes_set"] == 0 and rows[(MULTI, cid)]["answers"] is False
    assert rows[("q-alpha-001", other)]["completes_set"] is None
    assert not any("Evidence set" in c["prompt"] for c in fake_llm.calls() if "Item: q-alpha-001\n" in c["prompt"])


def test_an_output_naming_the_wrong_ids_is_rerolled(ds, fake_llm, bilbo_bin, ranked):
    seed_candidates(ds, ranked)
    row = next(r for r in ranked["rows"] if r["item"] == "q-alpha-001")
    good = [{"id": c["id"], "answers": False, "completes_set": None, "passage": None} for c in row["candidates"]]
    rules = [{"match": "Item: q-alpha-001\n", "output": {"judgments": good[:-1]}, "times": 1},
             {"match": "Item: q-alpha-001\n", "output": {"judgments": good}}]
    rules += [{"match": f"Item: {r['item']}\n", "output": {"judgments": [
        {"id": c["id"], "answers": False, "completes_set": None, "passage": None} for c in r["candidates"]]}}
        for r in ranked["rows"] if r["item"] != "q-alpha-001"]
    fake_llm.set_script(rules)
    assert run(ds, bilbo_bin) == 0
    assert len([j for j in judgments(ds) if j["item"] == "q-alpha-001"]) == len(good)
    assert (ds / "generation/outputs/pool/q-alpha-001.2.json").is_file()


def test_the_pool_refuses_a_frozen_dataset(ds, bilbo_bin):
    (ds / "FROZEN").write_text("x\n")
    with pytest.raises(Refused, match="frozen"):
        run(ds, bilbo_bin)


def test_a_missing_codex_is_refused_before_any_ranking(ds, bilbo_bin, monkeypatch):
    monkeypatch.setenv("PATH", "/nonexistent")
    with pytest.raises(Refused, match="codex"):
        run(ds, bilbo_bin)
    assert not (ds / "generation/pool/candidates.jsonl").read_text()


# --- the audit ----------------------------------------------------------------------------------------------------

def noes(n: int) -> list[dict]:
    return [{"item": f"q{i // 7}", "candidate": f"n{i}", "answers": False, "completes_set": None, "passage": None,
             "quote_found": False, "flag": None, "call_id": "c"} for i in range(n)]


def test_the_audit_is_a_seeded_tenth_of_the_noes_with_a_floor_of_fifty():
    assert len(pool.audit_sample(noes(30), 1)) == 30
    assert len(pool.audit_sample(noes(300), 1)) == 50
    assert len(pool.audit_sample(noes(1000), 1)) == 100
    assert pool.audit_sample(noes(300), 1) == pool.audit_sample(noes(300), 1) != pool.audit_sample(noes(300), 2)


def test_the_audit_skips_yeses_and_keeps_unquoted_yeses():
    rows = noes(60)
    rows[0].update(answers=True, quote_found=True, passage="p")
    rows[1].update(answers=True, quote_found=False, flag="yes_without_quote")
    sample = {a["candidate"] for a in pool.audit_sample(rows, 1)}
    assert "n0" not in sample and len(sample) == 50
    assert {a["candidate"] for a in pool.audit_sample(rows[:2], 1)} == {"n1"}


def test_an_existing_audit_is_never_rewritten(ds, fake_llm, bilbo_bin, ranked):
    seed_candidates(ds, ranked)
    write_jsonl(ds / "generation/pool/audit.jsonl", [
        {"item": "q-alpha-001", "candidate": "keep", "verdict": "agree", "reason": None, "reviewer": "x"}])
    script(fake_llm, ds)
    run(ds, bilbo_bin)
    assert [a["candidate"] for a in read_jsonl(ds / "generation/pool/audit.jsonl")] == ["keep"]


# --- --apply ------------------------------------------------------------------------------------------------------

def resolve(ds: Path, *rows: tuple) -> None:
    write_jsonl(ds / "generation/pool/resolutions.jsonl", [
        {"item": i, "candidate": c, "action": a, "evidence_set": e, "reason": "r", "reviewer": "x"} for i, c, a, e in rows])


def yes_row(item: str, cid: str) -> dict:
    return {"item": item, "candidate": cid, "answers": True, "completes_set": None, "passage": "q", "quote_found": True,
            "flag": None, "call_id": "c"}


def applied(ds: Path, bilbo: Path) -> int:
    return run(ds, bilbo, apply=True)


def test_a_second_answering_note_becomes_gold(ds, fake_llm, bilbo_bin, ranked, capsys):
    seed_candidates(ds, ranked)
    cid = next(r for r in ranked["rows"] if r["item"] == "q-alpha-001")["candidates"][0]["id"]
    script(fake_llm, ds, {"q-alpha-001": {cid: None}})
    run(ds, bilbo_bin)
    audit_done(ds)
    assert applied(ds, bilbo_bin) == 1
    assert f"q-alpha-001 {cid}: pooled yes with no resolution" in capsys.readouterr().out
    with pytest.raises(Refused, match=f"q-alpha-001 {cid}"):
        dataset.freeze(ds)
    assert not (ds / "FROZEN").exists()
    resolve(ds, ("q-alpha-001", cid, "add-gold", None))
    assert applied(ds, bilbo_bin) == 0
    q = next(q for q in read_jsonl(ds / "queries.jsonl") if q["id"] == "q-alpha-001")
    assert q["gold"] == ["01KM0DBEF0FH2PWDT59CHH4W9A", cid]
    assert f"q-alpha-001 0 {cid} 1" in (ds / "qrels/dev.txt").read_text()
    assert [r["candidate"] for r in read_jsonl(ds / "generation/pool/applied.jsonl")] == [cid]
    assert applied(ds, bilbo_bin) == 0
    assert len(read_jsonl(ds / "generation/pool/applied.jsonl")) == 1
    assert dataset.check(dataset.load(ds)) == []


def test_judged_noes_enter_the_qrels_at_zero(ds, fake_llm, bilbo_bin, ranked):
    seed_candidates(ds, ranked)
    script(fake_llm, ds)
    run(ds, bilbo_bin)
    audit_done(ds)
    assert applied(ds, bilbo_bin) == 0
    row = next(r for r in ranked["rows"] if r["item"] == "q-alpha-001")
    lines = (ds / "qrels/dev.txt").read_text().splitlines()
    assert all(f"q-alpha-001 0 {c['id']} 0" in lines for c in row["candidates"])


def test_add_evidence_extends_a_set_and_add_gold_on_multi_hop_adds_a_set(ds, bilbo_bin):
    write_jsonl(ds / "generation/pool/judgments.jsonl", [yes_row(MULTI, "NEWA"), yes_row(MULTI, "NEWB")])
    resolve(ds, (MULTI, "NEWA", "add-evidence", 0), (MULTI, "NEWB", "add-gold", None))
    # the candidates must exist as notes for the dataset check; here only the rows are under test
    assert applied(ds, bilbo_bin) == 0
    q = next(q for q in read_jsonl(ds / "queries.jsonl") if q["id"] == MULTI)
    assert q["evidence_sets"] == [["01KRE0BGJ00TBN9E758M021ZV5", "01KM649AP0FB8A6G5FC1V5PYH9", "NEWA"], ["NEWB"]]
    assert q["gold"][-2:] == ["NEWA", "NEWB"]


def test_add_evidence_needs_a_set_that_exists(ds, bilbo_bin):
    write_jsonl(ds / "generation/pool/judgments.jsonl", [yes_row(MULTI, "NEWA")])
    resolve(ds, (MULTI, "NEWA", "add-evidence", 5))
    with pytest.raises(Refused, match="evidence_set"):
        applied(ds, bilbo_bin)
    assert not (ds / "generation/pool/applied.jsonl").exists()


def test_a_no_answer_query_cannot_take_gold(ds, bilbo_bin):
    write_jsonl(ds / "generation/pool/judgments.jsonl", [yes_row(NA, "NEWA")])
    resolve(ds, (NA, "NEWA", "add-gold", None))
    with pytest.raises(Refused, match="no-answer"):
        applied(ds, bilbo_bin)


def test_a_near_miss_prompt_that_a_note_answers_becomes_positive(ds, bilbo_bin):
    nid = "01KM0DBEF0FH2PWDT59CHH4W9A"
    write_jsonl(ds / "generation/pool/judgments.jsonl", [yes_row(NEAR, nid)])
    resolve(ds, (NEAR, nid, "add-gold", None))
    assert applied(ds, bilbo_bin) == 0
    p = next(p for p in read_jsonl(ds / "digest/prompts.jsonl") if p["id"] == NEAR)
    assert (p["label"], p["gold"]) == ("positive", [nid])
    assert dataset.check(dataset.load(ds)) == []


def test_drop_removes_the_item_and_closes_its_other_resolutions(ds, bilbo_bin):
    nid = "01KM0DBEF0FH2PWDT59CHH4W9A"
    write_jsonl(ds / "generation/pool/judgments.jsonl", [yes_row(NEAR, nid), yes_row(NEAR, "OTHER")])
    resolve(ds, (NEAR, nid, "drop", None), (NEAR, "OTHER", "add-gold", None))
    assert applied(ds, bilbo_bin) == 0
    assert NEAR not in {p["id"] for p in read_jsonl(ds / "digest/prompts.jsonl")}
    q_before = {q["id"] for q in read_jsonl(ds / "queries.jsonl")}
    resolve(ds, ("q-alpha-001", "X", "drop", None))
    write_jsonl(ds / "generation/pool/judgments.jsonl", [yes_row("q-alpha-001", "X")])
    assert applied(ds, bilbo_bin) == 0
    assert {q["id"] for q in read_jsonl(ds / "queries.jsonl")} == q_before - {"q-alpha-001"}


def test_reject_and_rewrite_change_nothing(ds, bilbo_bin, capsys):
    nid = "01KM0DBEF0FH2PWDT59CHH4W9A"
    before = (ds / "queries.jsonl").read_bytes()
    write_jsonl(ds / "generation/pool/judgments.jsonl", [yes_row("q-alpha-001", nid), yes_row("q-alpha-002", nid)])
    resolve(ds, ("q-alpha-001", nid, "reject", None), ("q-alpha-002", nid, "rewrite", None))
    assert applied(ds, bilbo_bin) == 0
    assert (ds / "queries.jsonl").read_bytes() == before
    assert "q-alpha-002: the reviewer asked for a rewrite" in capsys.readouterr().err


def test_a_disagreed_audit_stays_open_until_resolved(ds, bilbo_bin, capsys):
    write_jsonl(ds / "generation/pool/audit.jsonl", [
        {"item": "q-alpha-001", "candidate": "N1", "verdict": "disagree", "reason": "it answers", "reviewer": "x"},
        {"item": "q-alpha-001", "candidate": "N2", "verdict": None, "reason": None, "reviewer": None}])
    assert applied(ds, bilbo_bin) == 1
    captured = capsys.readouterr()
    assert "q-alpha-001 N1: disagreed audit with no resolution" in captured.out
    assert "audit: 1 sampled noes have no reviewer verdict" in captured.out


def test_apply_reports_the_unreviewed_audit_count(ds, fake_llm, bilbo_bin, ranked, capsys):
    seed_candidates(ds, ranked)
    script(fake_llm, ds)
    run(ds, bilbo_bin)
    capsys.readouterr()
    assert applied(ds, bilbo_bin) == 1
    assert f"audit: {pool.AUDIT_MIN} sampled noes have no reviewer verdict" in capsys.readouterr().out
