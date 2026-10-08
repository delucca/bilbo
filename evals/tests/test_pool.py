"""pool: the candidates of every arm, the checking calls, the quote rule, the audit and the resolutions."""

from __future__ import annotations

import argparse
import hashlib
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


def sha(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def clear_pool(ds: Path) -> None:
    """Empty the pool files the fixture ships (candidates for every item, no judgments), so `pool` ranks from scratch."""
    for name in ("candidates", "judgments", "audit", "resolutions"):
        (ds / f"generation/pool/{name}.jsonl").write_text("", encoding="utf-8")


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
        hits: dict = {}
        rankings = runner.rank_all(d, "dev", list(ARMS), bilbo, None, None, fake.url, hits=hits)
    finally:
        fake.stop()
    loaded = dataset.load(d)
    items = pool.pooled_items(loaded, "dev")
    return {"rankings": rankings, "rows": [
        {"item": it["id"], "kind": it["kind"], "split": it["split"], "text_sha256": sha(it["text"]),
         "candidates": pool.top_candidates(loaded, it, rankings, hits)}
        for it in items], "hits": hits}


def seed_candidates(ds: Path, ranked) -> None:
    """The ranked dev rows in place of the fixture's; the test split keeps its (empty) rows."""
    path = ds / "generation/pool/candidates.jsonl"
    dev = {r["item"] for r in ranked["rows"]}
    write_jsonl(path, sorted([r for r in read_jsonl(path) if r["item"] not in dev] + ranked["rows"], key=lambda r: r["item"]))


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


LATE = "The retention window is exactly 41 days."


def long_note(ds: Path, loaded: dataset.Dataset, late: str = LATE, sections: int = 12) -> tuple[str, int]:
    """A note of `sections` filler sections and a last one holding `late`, written over the first note; (id, line of the last)."""
    nid = next(iter(loaded.notes))
    path = ds / "store/notes" / loaded.notes[nid].file
    head = path.read_text(encoding="utf-8").split("\n# ", 1)[0]
    body = "\n".join(f"## Section {n}\n{'Filler words about sync and pairing. ' * 30}\n" for n in range(sections))
    text = f"{head}\n# {loaded.notes[nid].title}\n\n{body}\n## Archive tail\n{late}\n"
    path.write_text(text, encoding="utf-8")
    return nid, len(text.split("\n")) - 1


def test_a_late_hit_passage_reaches_the_view(ds):
    nid, line = long_note(ds, dataset.load(ds))
    loaded = dataset.load(ds)
    assert len(loaded.notes[nid].text) > 3000 and loaded.notes[nid].text.index(LATE) > 3000
    cand = {"id": nid, "arms": ["bilbo-full"], "best_rank": 1, "lines": {"bilbo-full": line}}
    assert LATE in pool._view(loaded, cand, "unrelated request")
    assert LATE not in pool._view(loaded, {"id": nid}, "unrelated request")


def test_a_late_overlap_passage_reaches_the_view(ds):
    nid, _ = long_note(ds, dataset.load(ds))
    loaded = dataset.load(ds)
    assert LATE in pool.candidate_text(loaded, nid, "how many days is the retention window")


def test_the_view_holds_the_title_the_outline_and_respects_the_caps(ds):
    nid, line = long_note(ds, dataset.load(ds), late="Archive text. " * 400, sections=30)
    loaded = dataset.load(ds)
    view = pool.note_view(loaded, nid, "filler words sync pairing", [line, 1, 3, 40])
    assert view.startswith(f"Title: {loaded.notes[nid].title}\nOutline: ") and "Section 29" in view.split("\n")[1]
    assert len(view) <= pool.VIEW_CHARS and len(view.split("\n")[1]) <= pool.OUTLINE_CHARS
    assert all(len(part) <= pool.PASSAGE_CHARS + 60 for part in view.split("\n## ")[1:])
    assert len(pool.note_view(loaded, nid, "archive text", [line])) <= pool.VIEW_CHARS


def test_the_prompt_shows_the_view_not_the_first_3000_characters(ds, fake_llm, bilbo_bin, ranked):
    seed_candidates(ds, ranked)
    nid, line = long_note(ds, dataset.load(ds))
    rows = read_jsonl(ds / "generation/pool/candidates.jsonl")
    for r in rows:
        if r["item"] == "q-alpha-001":
            r["candidates"] = [{"id": nid, "arms": ["bilbo-full"], "best_rank": 1, "lines": {"bilbo-full": line}}]
    write_jsonl(ds / "generation/pool/candidates.jsonl", rows)
    script(fake_llm, ds, {"q-alpha-001": {nid: None}}, quote={("q-alpha-001", nid): LATE})
    run(ds, bilbo_bin)
    prompt = next(c["prompt"] for c in fake_llm.calls() if "Item: q-alpha-001\n" in c["prompt"])
    assert LATE in prompt and f"=== {nid} ===\nTitle: " in prompt
    row = next(j for j in judgments(ds) if j["item"] == "q-alpha-001" and j["candidate"] == nid)
    assert row["answers"] is True and row["quote_found"] is True


def test_the_hits_of_the_arms_are_kept_as_lines(ranked):
    with_lines = [c for r in ranked["rows"] for c in r["candidates"] if "lines" in c]
    assert with_lines and all(set(c["lines"]) <= set(c["arms"]) and all(n >= 1 for n in c["lines"].values()) for c in with_lines)
    assert {"bilbo-keyword", "bilbo-full", "dense-ref"} & {a for c in with_lines for a in c["lines"]}


def test_an_item_over_the_prompt_cap_splits_into_calls_and_merges(ds, fake_llm, bilbo_bin, ranked, monkeypatch):
    seed_candidates(ds, ranked)
    loaded = dataset.load(ds)
    item = next(it for it in pool.pooled_items(loaded, "dev") if it["id"] == "q-alpha-001")
    cands = next(r for r in ranked["rows"] if r["item"] == "q-alpha-001")["candidates"]
    monkeypatch.setattr(pool, "PROMPT_CHARS", 1)
    groups = pool.chunks(loaded, item, cands, llm.load_config(ds).seed)
    split = pool.chunks
    monkeypatch.setattr(pool, "chunks", lambda d, it, cs, seed: split(d, it, cs, seed) if it["id"] == "q-alpha-001" else [cs])
    assert len(groups) == len(cands) > 1 and pool.call_ids(item, len(groups))[1] == "q-alpha-001--b2"
    yes = groups[1][0]["id"]
    rules = []
    for cid, group in zip(pool.call_ids(item, len(groups)), groups):
        rules.append({"match": f"Item: {cid}\n", "output": {"judgments": [
            {"id": c["id"], "answers": c["id"] == yes, "completes_set": None,
             "passage": first_line(loaded, c["id"]) if c["id"] == yes else None} for c in group]}})
    rules += [{"match": f"Item: {r['item']}\n", "output": {"judgments": [
        {"id": c["id"], "answers": False, "completes_set": None, "passage": None} for c in r["candidates"]]}}
        for r in ranked["rows"] if r["item"] != "q-alpha-001"]
    fake_llm.set_script(rules)
    run(ds, bilbo_bin)
    mine = [j for j in judgments(ds) if j["item"] == "q-alpha-001"]
    assert sorted(j["candidate"] for j in mine) == sorted(c["id"] for c in cands)
    assert [j["candidate"] for j in mine if j["answers"]] == [yes] and all(j["quote_found"] for j in mine if j["answers"])
    assert mine[0]["call_id"].count(";") == len(cands) - 1 and "pool/q-alpha-001--b1/1" in mine[0]["call_id"]
    assert (ds / "generation/outputs/pool/q-alpha-001--b2.1.json").is_file()
    assert not (ds / "generation/outputs/pool/q-alpha-001.1.json").exists()


def test_set_aside_moves_the_outputs_of_split_calls(ds, ranked):
    seed_candidates(ds, ranked)
    out = ds / "generation/outputs/pool"
    out.mkdir(parents=True, exist_ok=True)
    for name in ("q-alpha-001.1.json", "q-alpha-001--b1.1.json", "q-alpha-001--b2.2.json", "q-alpha-002.1.json"):
        (out / name).write_text("{}", encoding="utf-8")
    rows = read_jsonl(ds / "queries.jsonl")
    for q in rows:
        if q["id"] == "q-alpha-001":
            q["text"] += " again"
    write_jsonl(ds / "queries.jsonl", rows)
    pool.set_aside(ds, pool.pooled_items(dataset.load(ds), "dev"))
    assert sorted(p.name for p in out.glob("*.json")) == ["q-alpha-002.1.json"]
    assert len(list((out / "superseded").glob("q-alpha-001*"))) == 3


def test_a_pool_prompt_stays_out_of_the_dataset_and_its_hash_matches_the_log(ds, fake_llm, bilbo_bin, ranked):
    seed_candidates(ds, ranked)
    script(fake_llm, ds)
    run(ds, bilbo_bin)
    assert not (ds / "generation/prompts").exists()
    log = [r for r in read_jsonl(ds / "generation/calls.jsonl") if r["step"] == "pool"]
    assert log and all(r["prompt_file"] is None for r in log)
    for r in log:
        cached = llm.cached_prompt_path(ds, r["prompt_sha256"])
        assert cached.is_file() and common.sha256_bytes(cached.read_bytes()) == r["prompt_sha256"]


# --- the checking calls -------------------------------------------------------------------------------------------

def test_cmd_ranks_judges_and_audits(ds, fake_llm, fake_embedder, bilbo_bin, ranked):
    clear_pool(ds)
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
    assert len(audit) == min(len(rows), pool.AUDIT_MIN) and all(a["verdict"] is None and a["split"] == "dev" for a in audit)
    assert all(r["text_sha256"] == sha(next(i["text"] for i in pool.pooled_items(dataset.load(ds), "dev") if i["id"] == r["item"]))
               for r in rows)
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
def test_an_unquoted_yes_counts_as_a_no_and_is_an_open_item_until_resolved(ds, fake_llm, bilbo_bin, ranked, passage):
    seed_candidates(ds, ranked)
    cid = next(r for r in ranked["rows"] if r["item"] == "q-alpha-001")["candidates"][0]["id"]
    script(fake_llm, ds, {"q-alpha-001": {cid: None}}, quote={("q-alpha-001", cid): passage})
    assert run(ds, bilbo_bin) == 0
    bad = next(j for j in judgments(ds) if (j["item"], j["candidate"]) == ("q-alpha-001", cid))
    assert bad["answers"] is True and bad["quote_found"] is False and bad["flag"] == "yes_without_quote"
    audit_done(ds)
    assert dataset.pool_open_items(ds) == [f"q-alpha-001 {cid}: unquoted yes with no resolution"]
    dataset.build_qrels(ds)
    assert f"q-alpha-001 0 {cid} 0" in (ds / "qrels/dev.txt").read_text()
    resolve(ds, ("q-alpha-001", cid, "reject", None))
    assert dataset.pool_open_items(ds) == []
    dataset.build_qrels(ds)
    assert f"q-alpha-001 0 {cid} 0" in (ds / "qrels/dev.txt").read_text()


def test_an_unquoted_yes_resolved_add_gold_is_gold(ds, fake_llm, bilbo_bin, ranked):
    seed_candidates(ds, ranked)
    cid = next(r for r in ranked["rows"] if r["item"] == "q-alpha-001")["candidates"][0]["id"]
    script(fake_llm, ds, {"q-alpha-001": {cid: None}}, quote={("q-alpha-001", cid): None})
    run(ds, bilbo_bin)
    audit_done(ds)
    resolve(ds, ("q-alpha-001", cid, "add-gold", None))
    assert applied(ds, bilbo_bin) == 0
    assert f"q-alpha-001 0 {cid} 1" in (ds / "qrels/dev.txt").read_text()


def test_a_quote_without_the_markdown_the_model_dropped_is_found(ds):
    loaded = dataset.load(ds)
    nid = next(n.id for n in loaded.notes.values() if n.file == "reference-importer-paths.md")
    item = {"id": "q-x", "kind": "query", "split": "dev", "text": "q", "row": {"stratum": "known-item"}}
    value = {"judgments": [{"id": nid, "answers": True, "completes_set": None, "passage": "reads files from /srv/beta/inbox"}]}
    [row] = pool.judgment_rows(loaded, item, [{"id": nid}], value, 1)
    assert row["quote_found"] is True and row["flag"] is None and row["text_sha256"] == sha("q")
    value["judgments"][0]["passage"] = "moves finished ones to /srv/beta/done."
    assert pool.judgment_rows(loaded, item, [{"id": nid}], value, 1)[0]["quote_found"] is True
    value["judgments"][0]["passage"] = "reads files from /srv/beta/outbox"
    assert pool.judgment_rows(loaded, item, [{"id": nid}], value, 1)[0]["quote_found"] is False


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
    clear_pool(ds)
    monkeypatch.setenv("PATH", "/nonexistent")
    with pytest.raises(Refused, match="codex"):
        run(ds, bilbo_bin)
    assert not (ds / "generation/pool/candidates.jsonl").read_text()


# --- the audit ----------------------------------------------------------------------------------------------------

def noes(n: int) -> list[dict]:
    return [{"item": f"q{i // 7}", "candidate": f"n{i}", "answers": False, "completes_set": None, "passage": None,
             "quote_found": False, "flag": None, "call_id": "c"} for i in range(n)]


def test_the_audit_is_a_seeded_tenth_of_the_noes_with_a_floor_of_fifty():
    assert len(pool.audit_topup(noes(30), [], 1, "dev")) == 30
    assert len(pool.audit_topup(noes(300), [], 1, "dev")) == 50
    assert len(pool.audit_topup(noes(1000), [], 1, "dev")) == 100
    assert pool.audit_topup(noes(300), [], 1, "dev") == pool.audit_topup(noes(300), [], 1, "dev")
    assert pool.audit_topup(noes(300), [], 1, "dev") != pool.audit_topup(noes(300), [], 2, "dev")
    assert pool.audit_topup(noes(300), [], 1, "dev") != pool.audit_topup(noes(300), [], 1, "test")


def test_the_audit_skips_yeses_and_keeps_unquoted_yeses():
    rows = noes(60)
    rows[0].update(answers=True, quote_found=True, passage="p")
    rows[1].update(answers=True, quote_found=False, flag="yes_without_quote")
    sample = {a["candidate"] for a in pool.audit_topup(rows, [], 1, "dev")}
    assert "n0" not in sample and len(sample) == 50
    assert {a["candidate"] for a in pool.audit_topup(rows[:2], [], 1, "dev")} == {"n1"}


def test_the_audit_tops_up_and_keeps_existing_verdicts():
    first = pool.audit_topup(noes(300), [], 1, "dev")
    done = [{**a, "verdict": "agree", "reviewer": "x"} for a in first]
    assert pool.audit_topup(noes(300), done, 1, "dev") == []
    more = pool.audit_topup(noes(1000), done, 1, "dev")
    assert len(more) == 50 and not {a["candidate"] for a in more} & {a["candidate"] for a in first}
    assert all(a["split"] == "dev" and a["verdict"] is None for a in more)


def test_an_existing_audit_keeps_its_verdicts_and_is_topped_up(ds, fake_llm, bilbo_bin, ranked):
    seed_candidates(ds, ranked)
    script(fake_llm, ds)
    run(ds, bilbo_bin)
    audit_done(ds)
    kept = read_jsonl(ds / "generation/pool/audit.jsonl")
    run(ds, bilbo_bin)
    assert read_jsonl(ds / "generation/pool/audit.jsonl") == kept
    write_jsonl(ds / "generation/pool/audit.jsonl", kept[:10])
    run(ds, bilbo_bin)
    got = read_jsonl(ds / "generation/pool/audit.jsonl")
    assert len(got) == len(kept) and [a for a in got if a["verdict"] == "agree"] == kept[:10]


def test_pooling_dev_then_test_audits_each_split_on_its_own(ds, fake_llm, fake_embedder, bilbo_bin, ranked, monkeypatch):
    seed_candidates(ds, ranked)
    script(fake_llm, ds)
    run(ds, bilbo_bin)
    audit_done(ds)
    dev_audit = read_jsonl(ds / "generation/pool/audit.jsonl")
    assert dev_audit and {a["split"] for a in dev_audit} == {"dev"}
    test_rows = pool.pooled_items(dataset.load(ds), "test")
    test_cands = [{"item": it["id"], "kind": it["kind"], "split": "test", "text_sha256": sha(it["text"]),
                   "candidates": [{"id": nid, "arms": ["random"], "best_rank": 1}]}
                  for it, nid in zip(test_rows, sorted(dataset.load(ds).notes))]
    rows = {r["item"]: r for r in read_jsonl(ds / "generation/pool/candidates.jsonl")}
    rows.update({r["item"]: r for r in test_cands})
    write_jsonl(ds / "generation/pool/candidates.jsonl", [rows[k] for k in sorted(rows)])
    fake_llm.set_script([{"match": f"Item: {r['item']}\n", "output": {"judgments": [
        {"id": c["id"], "answers": False, "completes_set": None, "passage": None} for c in r["candidates"]]}}
        for r in test_cands])
    run(ds, bilbo_bin, split="test")
    got = read_jsonl(ds / "generation/pool/audit.jsonl")
    assert [a for a in got if a["split"] == "dev"] == dev_audit
    test_audit = [a for a in got if a["split"] == "test"]
    assert len(test_audit) == len(test_rows) and all(a["verdict"] is None for a in test_audit)


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


def test_a_drop_closes_the_other_yeses_of_the_item_once_applied(ds, bilbo_bin):
    nid = "01KM0DBEF0FH2PWDT59CHH4W9A"
    write_jsonl(ds / "generation/pool/judgments.jsonl", [yes_row(NEAR, nid), yes_row(NEAR, "OTHER")])
    resolve(ds, (NEAR, nid, "drop", None))
    before = dataset.pool_open_items(ds)
    assert f"{NEAR} OTHER: pooled yes with no resolution" in before
    assert f"{NEAR}: drop not applied; run pool --apply" in before
    assert applied(ds, bilbo_bin) == 0
    assert dataset.pool_open_items(ds) == []


def test_a_text_change_sets_the_item_aside_and_pools_it_again(ds, fake_llm, bilbo_bin, ranked, monkeypatch, capsys):
    seed_candidates(ds, ranked)
    script(fake_llm, ds)
    run(ds, bilbo_bin)
    audit_done(ds)
    made = len(fake_llm.calls())
    old = next(r for r in read_jsonl(ds / "generation/pool/candidates.jsonl") if r["item"] == "q-alpha-001")
    old_judged = [j for j in judgments(ds) if j["item"] == "q-alpha-001"]
    rows = read_jsonl(ds / "queries.jsonl")
    for q in rows:
        if q["id"] == "q-alpha-001":
            q["text"] += " again"
    write_jsonl(ds / "queries.jsonl", rows)
    seen = []
    monkeypatch.setattr(runner, "rank_all", lambda *a, **k: seen.append(a) or k["hits"].update(ranked["hits"]) or ranked["rankings"])
    capsys.readouterr()
    assert dataset.pool_open_items(ds)[0] == "q-alpha-001: not pooled for its current text; run pool"
    run(ds, bilbo_bin)
    assert len(seen) == 1
    calls = fake_llm.calls()
    assert len(calls) == made + 1 and "again" in calls[-1]["prompt"]
    assert "q-alpha-001" in capsys.readouterr().err
    new = next(r for r in read_jsonl(ds / "generation/pool/candidates.jsonl") if r["item"] == "q-alpha-001")
    assert new["text_sha256"] == sha(old_text(rows)) and new["candidates"] == old["candidates"]
    assert {j["text_sha256"] for j in judgments(ds) if j["item"] == "q-alpha-001"} == {new["text_sha256"]}
    sup = read_jsonl(ds / "generation/pool/superseded.jsonl")
    assert [r["row"] for r in sup if r["file"] == "candidates.jsonl"] == [old]
    assert [r["row"] for r in sup if r["file"] == "judgments.jsonl"] == old_judged
    assert all(r["time"] for r in sup)


def old_text(rows: list[dict]) -> str:
    return next(q["text"] for q in rows if q["id"] == "q-alpha-001")


def test_reject_and_rewrite_change_nothing(ds, bilbo_bin, capsys):
    nid = "01KM0DBEF0FH2PWDT59CHH4W9A"
    before = (ds / "queries.jsonl").read_bytes()
    write_jsonl(ds / "generation/pool/judgments.jsonl", [yes_row("q-alpha-001", nid), yes_row("q-alpha-002", nid)])
    resolve(ds, ("q-alpha-001", nid, "reject", None), ("q-alpha-002", nid, "rewrite", None))
    assert applied(ds, bilbo_bin) == 1
    assert (ds / "queries.jsonl").read_bytes() == before
    captured = capsys.readouterr()
    assert "q-alpha-002: the reviewer asked for a rewrite" in captured.err
    assert f"q-alpha-002 {nid}: rewrite asked; edit the item, then run pool" in captured.out
    assert "q-alpha-001" not in captured.out


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


# --- one call per item, rebuildable prompts, judgments bound to the note ---------------------------------------------

def test_an_item_of_thirty_candidates_at_the_view_cap_is_one_call(ds, monkeypatch):
    loaded = dataset.load(ds)
    item = next(it for it in pool.pooled_items(loaded, "dev") if it["id"] == "q-alpha-001")
    cands = [{"id": f"n{n:02d}"} for n in range(30)]
    monkeypatch.setattr(pool, "_view", lambda d, c, q: "x" * pool.VIEW_CHARS)
    groups = pool.chunks(loaded, item, cands, 1)
    assert len(groups) == 1 and len(groups[0]) == 30
    assert pool.call_ids(item, len(groups)) == ["q-alpha-001"]
    monkeypatch.setattr(pool, "_view", lambda d, c, q: "x" * (pool.PROMPT_CHARS // 2 + 1))
    assert len(pool.chunks(loaded, item, cands[:2], 1)) == 2


def pool_calls(ds: Path) -> dict[str, str]:
    return {r["item"]: r["prompt_sha256"] for r in read_jsonl(ds / "generation/calls.jsonl")
            if r["step"] == "pool" and r["status"] == "ok"}


def test_a_pool_prompt_is_rebuilt_from_its_record_after_pool_apply_changed_the_evidence(ds, fake_llm, bilbo_bin, ranked):
    seed_candidates(ds, ranked)
    cid = next(r for r in ranked["rows"] if r["item"] == MULTI)["candidates"][0]["id"]
    script(fake_llm, ds)
    run(ds, bilbo_bin)
    records = {r["call_id"]: r for r in read_jsonl(ds / "generation/pool/records.jsonl")}
    assert set(records) == set(pool_calls(ds)) and records[MULTI]["template"] == "pool.md"
    assert records[MULTI]["template_sha256"] == sha((Path(pool.__file__).parent / "generate/templates/pool.md").read_text(encoding="utf-8"))
    resolve(ds, (MULTI, cid, "add-evidence", 0))
    applied(ds, bilbo_bin)
    loaded = dataset.load(ds)
    assert cid in next(q for q in loaded.queries if q["id"] == MULTI)["evidence_sets"][0]
    item = next(it for it in pool.pooled_items(loaded, "dev") if it["id"] == MULTI)
    naive = pool.build_prompt(loaded, item, next(r for r in ranked["rows"] if r["item"] == MULTI)["candidates"], llm.load_config(ds).seed)
    assert sha(naive) != pool_calls(ds)[MULTI]
    for call_id, record in records.items():
        assert sha(pool.rebuild_prompt(loaded, record)) == pool_calls(ds)[call_id]


def test_a_rebuild_refuses_a_template_that_changed(ds, fake_llm, bilbo_bin, ranked):
    seed_candidates(ds, ranked)
    script(fake_llm, ds)
    run(ds, bilbo_bin)
    record = read_jsonl(ds / "generation/pool/records.jsonl")[0]
    with pytest.raises(Refused, match="template"):
        pool.rebuild_prompt(dataset.load(ds), {**record, "template_sha256": "0" * 64})


def test_a_judgment_goes_back_to_pending_when_its_note_text_changed(ds, fake_llm, bilbo_bin, ranked):
    seed_candidates(ds, ranked)
    script(fake_llm, ds)
    run(ds, bilbo_bin)
    audit_done(ds)
    row = next(j for j in judgments(ds) if j["item"] == "q-alpha-001")
    loaded = dataset.load(ds)
    assert row["note_sha256"] == sha(loaded.notes[row["candidate"]].text)
    assert not [line for line in dataset.pool_open_items(ds) if "not judged" in line]
    path = ds / "store/notes" / loaded.notes[row["candidate"]].file
    path.write_text(path.read_text(encoding="utf-8") + "\nA line added in review.\n", encoding="utf-8")
    assert f"q-alpha-001 {row['candidate']}: not judged; run pool" in dataset.pool_open_items(ds)
    made = len(fake_llm.calls())
    run(ds, bilbo_bin)
    calls = fake_llm.calls()
    assert len(calls) == made + len({j["item"] for j in judgments(ds) if j["candidate"] == row["candidate"]})
    assert all("Item: q-alpha-001\n" in c["prompt"] or row["candidate"] in c["prompt"] for c in calls[made:])
    new = next(j for j in judgments(ds) if j["item"] == "q-alpha-001" and j["candidate"] == row["candidate"])
    assert new["note_sha256"] == sha(dataset.load(ds).notes[row["candidate"]].text) != row["note_sha256"]
    sup = [r["row"] for r in read_jsonl(ds / "generation/pool/superseded.jsonl") if r["file"] == "judgments.jsonl"]
    assert row in sup
    assert not [line for line in dataset.pool_open_items(ds) if "not judged" in line]


def test_a_multi_hop_judgment_goes_back_to_pending_when_an_evidence_note_changed(ds, fake_llm, bilbo_bin, ranked):
    seed_candidates(ds, ranked)
    script(fake_llm, ds)
    run(ds, bilbo_bin)
    loaded = dataset.load(ds)
    evidence = sorted({i for ev in next(q for q in loaded.queries if q["id"] == MULTI)["evidence_sets"] for i in ev})
    record = next(r for r in read_jsonl(ds / "generation/pool/records.jsonl") if r["item"] == MULTI)
    assert record["evidence_sha256"] == {i: sha(loaded.notes[i].text) for i in evidence}
    assert all(j["evidence_sha256"] == record["evidence_sha256"] for j in judgments(ds) if j["item"] == MULTI)
    path = ds / "store/notes" / loaded.notes[evidence[0]].file
    path.write_text(path.read_text(encoding="utf-8") + "\nA line added in review.\n", encoding="utf-8")
    assert any(line.startswith(f"{MULTI} ") and "not judged" in line for line in dataset.pool_open_items(ds))
    made = len(fake_llm.calls())
    run(ds, bilbo_bin)
    assert any(f"Item: {MULTI}\n" in c["prompt"] for c in fake_llm.calls()[made:])
    sup = [r for r in read_jsonl(ds / "generation/pool/superseded.jsonl") if r["row"]["item"] == MULTI]
    assert {r["file"] for r in sup} >= {"judgments.jsonl", "records.jsonl"}
    assert not [line for line in dataset.pool_open_items(ds) if line.startswith(f"{MULTI} ") and "not judged" in line]
