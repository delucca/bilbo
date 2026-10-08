"""dataset.py: load, check, qrels, freeze, verify, the run gates and the pooled-item rules."""

from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path

import pytest

from bilbo_evals import cli, dataset
from bilbo_evals.common import Refused, read_jsonl, write_jsonl

FIXTURE_HASH_LEN = 64


def unfreeze(ds_dir: Path) -> Path:
    (ds_dir / "MANIFEST").unlink()
    (ds_dir / "FROZEN").unlink()
    return ds_dir


@pytest.fixture
def draft(fixture_copy) -> Path:
    return unfreeze(fixture_copy)


def queries(ds_dir: Path) -> list[dict]:
    return read_jsonl(ds_dir / "queries.jsonl")


def edit_query(ds_dir: Path, qid: str, **changes) -> None:
    write_jsonl(ds_dir / "queries.jsonl", [{**q, **changes} if q["id"] == qid else q for q in queries(ds_dir)])


def note_file(ds_dir: Path, nid: str) -> Path:
    return next(p for p in (ds_dir / "store/notes").glob("*.md") if f"id: {nid}\n" in p.read_text(encoding="utf-8"))


def check(ds_dir: Path, split: str = "all") -> list[str]:
    return dataset.check(dataset.load(ds_dir), split)


def add_fillers(ds_dir: Path, n: int = 80) -> None:
    """Notes of unrelated words, so that a stem found in one or two of them is under 2% of the notes."""
    for i in range(n):
        (ds_dir / "store/notes" / f"reference-filler-{i:03d}.md").write_text(
            f"---\nid: 01ZZZZZZZZZZZZZZZZZZZZZ{i:03d}\ncreated: 2026-01-01T00:00-03:00\n---\n\n# Filler {i}\n\nunrelated words{i}x\n",
            encoding="utf-8",
        )


def gold_note(ds_dir: Path, qid: str) -> Path:
    q = next(q for q in queries(ds_dir) if q["id"] == qid)
    return note_file(ds_dir, q["gold"][0])


def test_the_fixture_is_clean_and_verifies(fixture_dir):
    assert check(fixture_dir) == []
    tree_hash, problems = dataset.verify(fixture_dir)
    assert problems == [] and len(tree_hash) == FIXTURE_HASH_LEN


def test_load_reads_notes_sources_and_world(fixture_dir):
    ds = dataset.load(fixture_dir)
    assert len(ds.notes) == 12 and set(ds.sources) == {"demo/wal", "demo/busy"}
    note = next(n for n in ds.notes.values() if n.file == "plan-migracao-cache.md")
    assert (note.kind, note.project, note.lang, note.title) == ("plan", "alpha", "pt", "Plano de migração do cache")
    assert ds.sources["demo/wal"].licence == "public-domain" and ds.sources["demo/wal"].url.startswith("https://")
    assert ds.frozen and ds.tree_hash == (fixture_dir / "FROZEN").read_text().strip()
    assert ds.version == "notes-fixture"
    assert len(ds.queries_for("dev")) == 9 and len(ds.queries_for("test", library=True)) == 1
    assert {q["split"] for q in ds.queries_for("all", library=False)} == {"dev", "test"}


def test_every_stratum_is_in_the_fixture(fixture_dir):
    strata = {q["stratum"] for q in dataset.load(fixture_dir).queries}
    assert strata == {"known-item", "paraphrase", "pt-en", "alias", "supersession", "multi-hop", "kind-filter", "no-answer", "library"}


def test_the_fixture_store_is_a_real_bilbo_store(fixture_copy, bilbo_bin, tmp_path):
    config = tmp_path / "config"
    config.write_text("", encoding="utf-8")
    env = {**os.environ, "BILBO_HOME": str(fixture_copy / "store"), "BILBO_CONFIG": str(config)}
    done = subprocess.run([str(bilbo_bin), "check"], env=env, capture_output=True, text=True)
    assert (done.returncode, done.stdout) == (0, "")


def test_a_gold_id_that_is_not_a_note(draft):
    edit_query(draft, "q-alpha-001", gold=["01NOSUCHNOTE"])
    assert any("q-alpha-001" in p and "01NOSUCHNOTE" in p for p in check(draft))


def test_a_library_gold_that_is_not_landed(draft):
    edit_query(draft, "q-lib-001", gold=["demo/nope"])
    assert any("q-lib-001" in p and "demo/nope" in p for p in check(draft))


def test_an_alias_the_gold_note_uses(draft):
    path = gold_note(draft, "q-alpha-004")
    path.write_text(path.read_text(encoding="utf-8") + "\nIt was Lantern once.\n", encoding="utf-8")
    assert any("q-alpha-004" in p and "Lantern" in p for p in check(draft))


def test_an_alias_with_no_bridge(draft):
    bridge = draft / "store/notes/research-lantern-rename.md"
    bridge.write_text(bridge.read_text(encoding="utf-8").replace("edge-cache", "the cache"), encoding="utf-8")
    problems = check(draft)
    assert any("q-alpha-004" in p and "Lantern" in p and "edge-cache" in p for p in problems)


def test_a_reachable_alias_is_accepted(fixture_dir):
    ds = dataset.load(fixture_dir)
    q = next(q for q in ds.queries if q["id"] == "q-alpha-004")
    assert dataset.alias_problems(ds, q) == []


def test_a_project_in_both_splits(draft):
    edit_query(draft, "q-alpha-002", split="test")
    problems = check(draft)
    assert any("project alpha" in p and "q-alpha-002" in p and "q-alpha-001" in p for p in problems)


def test_a_project_in_one_split_is_accepted(fixture_dir):
    assert not any("both splits" in p for p in check(fixture_dir))


def test_a_row_that_breaks_the_schema(draft):
    rows = queries(draft)
    del rows[0]["stratum"]
    write_jsonl(draft / "queries.jsonl", rows)
    assert any("q-alpha-001" in p and "stratum" in p for p in check(draft))


def test_an_absolute_home_path(draft):
    (draft / "world/notes-extra.txt").write_text(f"see {Path.home()}/x", encoding="utf-8")
    assert any("world/notes-extra.txt" in p for p in check(draft))


def test_an_echoing_query_names_the_shared_tokens(draft):
    add_fillers(draft)
    path = gold_note(draft, "q-alpha-002")
    path.write_text(path.read_text(encoding="utf-8") + "\nThe checkpoint calls fsync twice.\n", encoding="utf-8")
    edit_query(draft, "q-alpha-002", text="when does the checkpoint call fsync", zero_overlap=False)
    problems = check(draft)
    assert any("q-alpha-002" in p and "checkpoint" in p and "fsync" in p for p in problems)


def test_one_natural_anchor_is_allowed(draft):
    add_fillers(draft)
    path = gold_note(draft, "q-alpha-003")
    path.write_text(path.read_text(encoding="utf-8") + "\nThe checkpoint runs often.\n", encoding="utf-8")
    edit_query(draft, "q-alpha-003", text="como funciona o checkpoint", zero_overlap=False)
    assert not any("q-alpha-003" in p for p in check(draft))


def test_zero_overlap_must_match_the_leakage(draft):
    add_fillers(draft)
    edit_query(draft, "q-alpha-003", zero_overlap=False)
    assert any("q-alpha-003" in p and "zero_overlap" in p for p in check(draft))


def test_leakage_helper_returns_the_union_over_gold_notes(draft):
    add_fillers(draft)
    path = gold_note(draft, "q-alpha-002")
    path.write_text(path.read_text(encoding="utf-8") + "\nThe checkpoint calls fsync.\n", encoding="utf-8")
    ds = dataset.load(draft)
    q = {**next(q for q in ds.queries if q["id"] == "q-alpha-002"), "text": "checkpoint fsync"}
    assert len(dataset.leakage(ds, q)) == 2


def test_check_by_split_ignores_the_other_split(draft):
    edit_query(draft, "q-beta-001", gold=["01NOSUCHNOTE"])
    assert check(draft, "dev") == []
    assert any("q-beta-001" in p for p in check(draft, "test"))


def test_a_stale_corpus_is_reported(draft):
    (draft / "store/notes/reference-extra.md").write_text(
        "---\nid: 01ZZZZZZZZZZZZZZZZZZZZZEXT\ncreated: 2026-01-01T00:00-03:00\n---\n\n# Extra\n", encoding="utf-8")
    assert any("corpus.jsonl" in p for p in check(draft))


def test_cmd_check_prints_ok_or_the_problems(fixture_copy, capsys):
    assert cli.main(["dataset", "check", "--dataset", str(fixture_copy)]) == 0
    assert capsys.readouterr().out.strip() == "ok: 18 queries, 10 prompts, 12 notes"
    edit_query(fixture_copy, "q-alpha-001", gold=["01NOSUCHNOTE"])
    assert cli.main(["dataset", "check", "--dataset", str(fixture_copy)]) == 1
    assert "q-alpha-001" in capsys.readouterr().out


# --- corpus and qrels ---------------------------------------------------------------------------------------------

def test_the_committed_corpus_and_qrels_are_what_the_builders_write(draft):
    before = {p: (draft / p).read_bytes() for p in ("corpus.jsonl", "qrels/dev.txt", "qrels/test.txt", "qrels/library-dev.txt")}
    dataset.build_corpus(draft)
    dataset.build_qrels(draft)
    assert before == {p: (draft / p).read_bytes() for p in before}


def test_corpus_rows(fixture_dir):
    rows = {r["_id"]: r for r in read_jsonl(fixture_dir / "corpus.jsonl")}
    assert len(rows) == 14
    wal = rows["demo/wal"]
    assert wal["metadata"]["kind"] == "source" and wal["metadata"]["source"]["licence"] == "public-domain"
    assert wal["metadata"]["path"] == "library/demo/wal.md" and wal["canary"].startswith("BENCHMARK DATA")
    note = next(r for r in rows.values() if r["metadata"]["path"] == "notes/plan-migracao-cache.md")
    assert note["metadata"]["lang"] == "pt" and note["metadata"]["project"] == "alpha" and note["metadata"]["source"] is None


def test_qrels_hold_gold_and_evidence_as_one_and_decoys_as_zero(fixture_dir):
    lines = (fixture_dir / "qrels/dev.txt").read_text().splitlines()
    ids = {q["id"]: q for q in dataset.load(fixture_dir).queries}
    sup = ids["q-alpha-005"]
    assert f"q-alpha-005 0 {sup['gold'][0]} 1" in lines and f"q-alpha-005 0 {sup['decoys'][0]} 0" in lines
    hop = ids["q-alpha-006"]
    assert all(f"q-alpha-006 0 {n} 1" in lines for n in hop["evidence_sets"][0])
    assert not any(line.startswith("q-alpha-na") for line in lines)
    assert lines == sorted(lines, key=lambda l: (l.split()[0], l.split()[2]))
    assert (fixture_dir / "qrels/library-test.txt").read_text() == "q-lib-002 0 demo/busy 1\n"


def test_judged_noes_enter_the_qrels_as_zero_and_rejected_yeses_too(draft):
    pool = draft / "generation/pool"
    other = "01OTHERNOTE"
    write_jsonl(pool / "judgments.jsonl", [
        {"item": "q-alpha-001", "candidate": "01NO", "answers": False, "completes_set": None, "passage": None, "quote_found": False, "flag": None, "call_id": "c1"},
        {"item": "q-alpha-001", "candidate": other, "answers": True, "completes_set": None, "passage": "x y", "quote_found": True, "flag": None, "call_id": "c2"},
        {"item": "q-alpha-001", "candidate": "01NOQUOTE", "answers": True, "completes_set": None, "passage": None, "quote_found": False, "flag": "yes_without_quote", "call_id": "c3"},
    ])
    write_jsonl(pool / "resolutions.jsonl", [
        {"item": "q-alpha-001", "candidate": other, "action": "reject", "evidence_set": None, "reason": "r", "reviewer": "x"}])
    dataset.build_qrels(draft)
    lines = (draft / "qrels/dev.txt").read_text().splitlines()
    assert "q-alpha-001 0 01NO 0" in lines and f"q-alpha-001 0 {other} 0" in lines and "q-alpha-001 0 01NOQUOTE 0" in lines


def test_build_refuses_a_frozen_dataset(fixture_copy):
    with pytest.raises(Refused, match="frozen"):
        dataset.build_qrels(fixture_copy)


# --- freeze, verify ------------------------------------------------------------------------------------------------

def test_freezing_the_unfrozen_fixture_gives_the_committed_hash(draft, fixture_dir):
    assert dataset.freeze(draft) == (fixture_dir / "FROZEN").read_text().strip()
    assert (draft / "MANIFEST").read_bytes() == (fixture_dir / "MANIFEST").read_bytes()


def test_manifest_lines_are_sorted_hash_two_spaces_path(fixture_dir):
    lines = (fixture_dir / "MANIFEST").read_text().splitlines()
    paths = [l.split("  ", 1)[1] for l in lines]
    assert paths == sorted(paths) and "MANIFEST" not in paths and "FROZEN" not in paths
    assert all(len(l.split("  ", 1)[0]) == 64 for l in lines)


def test_verify_a_clean_dataset(fixture_dir, capsys):
    assert cli.main(["dataset", "verify", str(fixture_dir)]) == 0
    assert capsys.readouterr().out.strip() == (fixture_dir / "FROZEN").read_text().strip()


def test_verify_names_an_edited_a_missing_and_an_extra_file(fixture_copy, capsys):
    with open(fixture_copy / "queries.jsonl", "a", encoding="utf-8") as f:
        f.write(" ")
    (fixture_copy / "README.md").unlink()
    (fixture_copy / "stray.txt").write_text("x", encoding="utf-8")
    assert cli.main(["dataset", "verify", str(fixture_copy)]) == 1
    out = capsys.readouterr().out
    assert "queries.jsonl: differs from MANIFEST" in out
    assert "README.md: listed in MANIFEST, missing" in out
    assert "stray.txt: not listed in MANIFEST, extra" in out


def test_verify_an_unfrozen_folder(draft):
    _hash, problems = dataset.verify(draft)
    assert problems == ["MANIFEST: missing", "FROZEN: missing"]


def test_freezing_twice_is_refused_and_changes_nothing(fixture_copy):
    before = (fixture_copy / "MANIFEST").read_bytes(), (fixture_copy / "FROZEN").read_bytes()
    with pytest.raises(Refused, match="already frozen"):
        dataset.freeze(fixture_copy)
    assert before == ((fixture_copy / "MANIFEST").read_bytes(), (fixture_copy / "FROZEN").read_bytes())


def test_freeze_needs_the_preregistration(draft):
    (draft / "preregistration.json").unlink()
    with pytest.raises(Refused, match="preregistration"):
        dataset.freeze(draft)
    assert not (draft / "FROZEN").exists()


def test_freeze_refuses_a_failing_check(draft):
    edit_query(draft, "q-alpha-001", gold=["01NOSUCHNOTE"])
    with pytest.raises(Refused, match="q-alpha-001"):
        dataset.freeze(draft)
    assert not (draft / "FROZEN").exists()


def test_freeze_refuses_a_failed_review(draft):
    rows = read_jsonl(draft / "generation/review/sheet.jsonl")
    for r in rows[:12]:
        r["verdict"], r["reason"] = "invalid", "bad"
    write_jsonl(draft / "generation/review/sheet.jsonl", rows)
    with pytest.raises(Refused, match="review"):
        dataset.freeze(draft)


def test_an_unresolved_yes_blocks_the_freeze(draft):
    write_jsonl(draft / "generation/pool/judgments.jsonl", [
        {"item": "q-alpha-001", "candidate": "01JUDGED", "answers": True, "completes_set": None, "passage": "a quote", "quote_found": True, "flag": None, "call_id": "c"}])
    with pytest.raises(Refused, match="q-alpha-001 01JUDGED"):
        dataset.freeze(draft)
    assert not (draft / "FROZEN").exists()


def test_pool_open_items(draft):
    pool = draft / "generation/pool"
    yes = {"answers": True, "completes_set": None, "passage": "a quote", "quote_found": True, "flag": None, "call_id": "c"}
    write_jsonl(pool / "judgments.jsonl", [
        {"item": "q1", "candidate": "n1", **yes}, {"item": "q1", "candidate": "n2", **yes},
        {"item": "q1", "candidate": "n3", **{**yes, "quote_found": False, "flag": "yes_without_quote"}}])
    write_jsonl(pool / "audit.jsonl", [
        {"item": "q1", "candidate": "n4", "verdict": "disagree", "reason": "r", "reviewer": "x"},
        {"item": "q1", "candidate": "n5", "verdict": "agree", "reason": None, "reviewer": "x"}])
    assert [i.split(":")[0] for i in dataset.pool_open_items(draft)] == ["q1 n1", "q1 n2", "q1 n4"]
    write_jsonl(pool / "resolutions.jsonl", [
        {"item": "q1", "candidate": "n1", "action": "add-gold", "evidence_set": None, "reason": "r", "reviewer": "x"},
        {"item": "q1", "candidate": "n2", "action": "reject", "evidence_set": None, "reason": "r", "reviewer": "x"},
        {"item": "q1", "candidate": "n4", "action": "drop", "evidence_set": None, "reason": "r", "reviewer": "x"}])
    assert [i.split(":")[0] for i in dataset.pool_open_items(draft)] == ["q1 n1"]
    write_jsonl(pool / "applied.jsonl", [{"item": "q1", "candidate": "n1"}])
    assert dataset.pool_open_items(draft) == []


# --- gates, materialize --------------------------------------------------------------------------------------------

def test_require_ready_on_a_frozen_dataset(fixture_dir):
    assert dataset.require_ready(fixture_dir, False, "test") == (fixture_dir / "FROZEN").read_text().strip()


def test_require_ready_refuses_an_unfrozen_dataset_and_suggests_draft(draft):
    with pytest.raises(Refused, match="not frozen.*--draft"):
        dataset.require_ready(draft, False, "dev")


def test_require_ready_draft_runs_on_dev_only(draft):
    assert dataset.require_ready(draft, True, "dev") is None
    with pytest.raises(Refused, match="test split"):
        dataset.require_ready(draft, True, "test")


def test_require_ready_names_an_edited_file(fixture_copy):
    with open(fixture_copy / "queries.jsonl", "a", encoding="utf-8") as f:
        f.write(" ")
    with pytest.raises(Refused, match="queries.jsonl"):
        dataset.require_ready(fixture_copy, False, "dev")


def test_materialize_copies_and_maps_paths_to_ids(fixture_dir, tmp_path):
    ds = dataset.load(fixture_dir)
    store = tmp_path / "sandbox/store"
    store.mkdir(parents=True)
    mapping = dataset.materialize(ds, store)
    note = next(n for n in ds.notes.values() if n.file == "gotcha-edge-cache-eviction.md")
    assert mapping[str(store.resolve() / "notes" / note.file)] == note.id
    assert mapping[str(store.resolve() / "library/demo/wal.md")] == "demo/wal"
    assert all(Path(p).is_file() and not Path(p).is_symlink() for p in mapping)
    assert len(mapping) >= 14
    assert (store / ".bilbo/captures").is_dir()


def test_an_unreviewed_audit_row_blocks_the_freeze_until_it_has_a_verdict(draft):
    row = {"item": "q-alpha-001", "candidate": "01AUDITED", "verdict": None, "reason": None, "reviewer": None}
    write_jsonl(draft / "generation/pool/audit.jsonl", [row, {**row, "candidate": "01AUDITED2"}])
    with pytest.raises(Refused, match="2 sampled noes have no reviewer verdict"):
        dataset.freeze(draft)
    assert not (draft / "FROZEN").exists()
    write_jsonl(draft / "generation/pool/audit.jsonl", [{**row, "verdict": "agree", "reviewer": "x"}, {**row, "candidate": "01AUDITED2", "verdict": "agree", "reviewer": "x"}])
    assert len(dataset.freeze(draft)) == FIXTURE_HASH_LEN


def test_the_small_fixture_exercises_the_leakage_rules(fixture_dir):
    ds = dataset.load(fixture_dir)
    got = {q["id"]: dataset.leakage(ds, q) for q in ds.queries if q["stratum"] in dataset.LEAK_STRATA}
    assert got["q-alpha-004"] == {"listen"} and got["q-alpha-003"] == set()
    flags = {q["id"]: q["zero_overlap"] for q in ds.queries if q["stratum"] in dataset.LEAK_STRATA}
    assert flags["q-alpha-004"] is False and flags["q-alpha-003"] is True
