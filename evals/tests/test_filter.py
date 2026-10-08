"""generate/filter.py: reject echoing queries, mark zero overlap, check alias bridges, write the distribution."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from bilbo_evals import cli, dataset
from bilbo_evals.common import Refused, read_jsonl
from bilbo_evals.generate import filter as leak_filter
from test_dataset import edit_query, gold_note, queries, unfreeze


def add_common_fillers(ds_dir: Path, n: int = 80) -> None:
    """Notes that repeat the fixture's whole vocabulary, so its words are common and only planted words are distinctive."""
    vocabulary = "\n".join(p.read_text(encoding="utf-8") for p in sorted((ds_dir / "store/notes").glob("*.md")))
    body = "\n".join(l for l in vocabulary.splitlines() if not l.startswith(("id:", "created:", "---")))
    for i in range(n):
        (ds_dir / "store/notes" / f"reference-filler-{i:03d}.md").write_text(
            f"---\nid: 01ZZZZZZZZZZZZZZZZZZZZZ{i:03d}\ncreated: 2026-01-01T00:00-03:00\n---\n\n# Filler {i}\n\n{body}\n", encoding="utf-8")


@pytest.fixture
def draft(fixture_copy) -> Path:
    unfreeze(fixture_copy)
    add_common_fillers(fixture_copy)
    dataset.build_corpus(fixture_copy)
    return fixture_copy


def append_to_gold(ds_dir: Path, qid: str, text: str) -> None:
    path = gold_note(ds_dir, qid)
    path.write_text(path.read_text(encoding="utf-8") + text, encoding="utf-8")


def drops(ds_dir: Path) -> list[dict]:
    path = ds_dir / "generation/drops.jsonl"
    return read_jsonl(path) if path.is_file() else []


def leakage(ds_dir: Path) -> dict:
    return json.loads((ds_dir / "generation/leakage.json").read_text(encoding="utf-8"))


def test_an_echoing_query_is_rejected_and_logged(draft):
    append_to_gold(draft, "q-alpha-003", "\nThe checkpoint calls fsync twice.\n")
    edit_query(draft, "q-alpha-003", text="o checkpoint chama fsync", zero_overlap=None)
    result = leak_filter.run(draft, ["dev"])
    assert result["rejected"] == 1
    assert "q-alpha-003" not in {q["id"] for q in queries(draft)}
    (row,) = drops(draft)
    assert (row["item"], row["reason"], row["tokens"], row["attempt"], row["final"]) == ("q-alpha-003", "leakage", ["checkpoint", "fsync"], 1, False)
    assert "q-alpha-003" not in (draft / "qrels/dev.txt").read_text()


def test_the_third_rewrite_is_final(draft):
    append_to_gold(draft, "q-alpha-003", "\nThe checkpoint calls fsync twice.\n")
    row = next(q for q in queries(draft) if q["id"] == "q-alpha-003")
    edit_query(draft, "q-alpha-003", text="o checkpoint chama fsync", gen={**row["gen"], "call_id": "queries/q-alpha-003/3"})
    leak_filter.run(draft, ["dev"])
    assert drops(draft)[0]["attempt"] == 3 and drops(draft)[0]["final"] is True


def test_one_shared_token_is_kept_with_zero_overlap_false(draft):
    append_to_gold(draft, "q-alpha-003", "\nThe checkpoint runs often.\n")
    edit_query(draft, "q-alpha-003", text="como funciona o checkpoint")
    leak_filter.run(draft, ["dev"])
    kept = {q["id"]: q for q in queries(draft)}
    assert kept["q-alpha-003"]["zero_overlap"] is False
    assert drops(draft) == []
    assert kept["q-alpha-004"]["zero_overlap"] is True


def test_zero_overlap_is_only_set_for_the_leak_strata(draft):
    leak_filter.run(draft, ["dev", "test"])
    for q in queries(draft):
        assert (q["zero_overlap"] is None) == (q["stratum"] not in ("paraphrase", "pt-en", "alias"))


def test_the_distribution_is_written_per_split_and_stratum(draft):
    append_to_gold(draft, "q-alpha-003", "\nThe checkpoint runs often.\n")
    edit_query(draft, "q-alpha-003", text="como funciona o checkpoint")
    append_to_gold(draft, "q-alpha-002", "\nThe checkpoint calls fsync twice.\n")
    edit_query(draft, "q-alpha-002", text="when does the checkpoint call fsync")
    leak_filter.run(draft, ["dev", "test"])
    dist = leakage(draft)
    assert dist["dev"]["pt-en"] == {"0": 0, "1": 1, "2+": 0}
    assert dist["dev"]["paraphrase"] == {"0": 0, "1": 0, "2+": 1}
    assert dist["dev"]["alias"] == {"0": 1, "1": 0, "2+": 0}
    assert set(dist["dev"]["library"]) == {"0", "1", "2+"} and sum(dist["test"]["library"].values()) == 1


def test_a_rerun_changes_nothing(draft):
    append_to_gold(draft, "q-alpha-002", "\nThe checkpoint calls fsync twice.\n")
    edit_query(draft, "q-alpha-002", text="when does the checkpoint call fsync")
    leak_filter.run(draft, ["dev"])
    first = (queries(draft), leakage(draft), drops(draft))
    leak_filter.run(draft, ["dev"])
    assert (queries(draft), leakage(draft), drops(draft)) == first


def test_the_split_option_leaves_the_other_split_alone(draft):
    append_to_gold(draft, "q-beta-002", "\nThe checkpoint calls fsync twice.\n")
    edit_query(draft, "q-beta-002", text="when does the checkpoint call fsync")
    leak_filter.run(draft, ["dev"])
    assert "q-beta-002" in {q["id"] for q in queries(draft)}
    leak_filter.run(draft, ["test"])
    assert "q-beta-002" not in {q["id"] for q in queries(draft)}


def test_an_alias_the_gold_note_uses_is_rejected(draft):
    append_to_gold(draft, "q-alpha-004", "\nIt was Lantern once.\n")
    leak_filter.run(draft, ["dev"])
    (row,) = drops(draft)
    assert row["item"] == "q-alpha-004" and row["reason"] == "alias" and "Lantern" in row["detail"][0]


def test_an_alias_without_a_bridge_is_rejected(fixture_copy):
    draft = unfreeze(fixture_copy)
    bridge = draft / "store/notes/research-lantern-rename.md"
    bridge.write_text(bridge.read_text(encoding="utf-8").replace("edge-cache", "the cache"), encoding="utf-8")
    leak_filter.run(draft, ["dev"])
    assert [r["reason"] for r in drops(draft)] == ["alias"]


def test_the_filtered_dataset_checks_clean(draft):
    append_to_gold(draft, "q-alpha-002", "\nThe checkpoint calls fsync twice.\n")
    edit_query(draft, "q-alpha-002", text="when does the checkpoint call fsync")
    leak_filter.run(draft, ["dev", "test"])
    dataset.build_corpus(draft)
    assert dataset.check(dataset.load(draft)) == []


def test_the_command_prints_a_summary(draft, capsys):
    assert cli.main(["generate", "filter", "--dataset", str(draft), "--split", "dev"]) == 0
    assert capsys.readouterr().out.startswith("filter dev: kept 18, rejected 0")


def test_a_frozen_dataset_is_refused(fixture_copy, capsys):
    assert cli.main(["generate", "filter", "--dataset", str(fixture_copy)]) == 1
    assert "frozen" in capsys.readouterr().err
    with pytest.raises(Refused):
        leak_filter.run(fixture_copy, ["dev"])
