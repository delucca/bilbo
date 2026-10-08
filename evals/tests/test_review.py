"""review.py: the seeded sample, the share check and the drops."""

from __future__ import annotations

from pathlib import Path

import pytest

from bilbo_evals import cli, dataset, review
from bilbo_evals.common import Refused, read_jsonl, write_jsonl
from test_dataset import add_fillers, queries, unfreeze

SHEET = "generation/review/sheet.jsonl"


@pytest.fixture
def draft(fixture_copy) -> Path:
    unfreeze(fixture_copy)
    (fixture_copy / SHEET).unlink()
    return fixture_copy


def sheet(ds_dir: Path) -> list[dict]:
    return read_jsonl(ds_dir / SHEET)


def clone_queries(ds_dir: Path, per_stratum: int) -> None:
    """Grow the test split to `per_stratum` queries per stratum, and its digest prompts to 12 positives."""
    rows = queries(ds_dir)
    extra = []
    for q in rows:
        if q["split"] == "test" and q["stratum"] != "library":
            extra += [{**q, "id": f"{q['id']}-c{i}"} for i in range(per_stratum - 1)]
    write_jsonl(ds_dir / "queries.jsonl", rows + extra)
    prompts = read_jsonl(ds_dir / "digest/prompts.jsonl")
    base = next(p for p in prompts if p["label"] == "positive" and p["split"] == "test")
    write_jsonl(ds_dir / "digest/prompts.jsonl", prompts + [{**base, "id": f"p-beta-c{i}"} for i in range(12)])


def mark(rows: list[dict], valid: int, invalid: int, resolution: str | None) -> list[dict]:
    out = []
    for i, r in enumerate(rows):
        if i < invalid:
            r = {**r, "verdict": "invalid", "reason": "wrong", "resolution": resolution, "reviewer": "x"}
        else:
            r = {**r, "verdict": "valid", "reviewer": "x"}
        out.append(r)
    return out


def test_sample_draws_every_group_of_the_fixture(draft):
    new, total = review.sample(draft, "all")
    assert new == total == 40
    rows = sheet(draft)
    assert {r["type"] for r in rows} == {"query", "prompt", "note"}
    assert len([r for r in rows if r["type"] == "note"]) == 12
    assert all(r["verdict"] is None and r["reviewer"] is None for r in rows)
    first = rows[0]
    assert set(first) == {"item", "type", "split", "group", "text", "gold", "decoys", "evidence_sets", "verdict", "reason", "resolution", "reviewer"}


def test_sample_quotas(draft):
    clone_queries(draft, 8)
    add_fillers(draft, 30)
    review.sample(draft, "all")
    rows = sheet(draft)
    count = lambda split, group: len([r for r in rows if r["split"] == split and r["group"] == group])
    assert count("test", "supersession") == 8 and count("test", "multi-hop") == 8
    assert count("test", "known-item") == 5 and count("test", "kind-filter") == 5
    assert count("dev", "supersession") == 1
    assert count("test", "positive") == 10
    assert len([r for r in rows if r["type"] == "note"]) == 20


def test_sample_is_seeded_and_appends_only_new_items(draft):
    clone_queries(draft, 8)
    review.sample(draft, "all")
    first = sheet(draft)
    again = review.sample(draft, "all")
    assert again[0] == 0 and sheet(draft) == first
    rows = [{**r, "verdict": "valid", "reviewer": "me"} if i == 0 else r for i, r in enumerate(first)]
    write_jsonl(draft / SHEET, rows)
    review.sample(draft, "all")
    assert sheet(draft)[0]["verdict"] == "valid"


def test_sample_by_split_keeps_the_other_split_out(draft):
    review.sample(draft, "dev")
    assert {r["split"] for r in sheet(draft) if r["type"] != "note"} == {"dev"}


def test_sample_is_deterministic_for_the_seed(draft, tmp_path):
    clone_queries(draft, 8)
    review.sample(draft, "all")
    one = sheet(draft)
    (draft / SHEET).unlink()
    review.sample(draft, "all")
    assert sheet(draft) == one


def test_sample_refuses_a_frozen_dataset(fixture_copy):
    with pytest.raises(Refused, match="frozen"):
        review.sample(fixture_copy)


def cover(draft: Path, rows: int) -> list[dict]:
    """A sheet that covers the sample and holds `rows` items in all."""
    review.sample(draft, "all")
    base = sheet(draft)
    pad = [{**base[0], "item": f"x-{i}"} for i in range(rows - len(base))]
    return base + pad


def test_check_passes_at_118_of_120(draft, capsys):
    write_jsonl(draft / SHEET, mark(cover(draft, 120), 118, 2, "fixed"))
    assert cli.main(["review", "check", "--dataset", str(draft)]) == 0
    assert capsys.readouterr().out.strip() == "118/120 valid (98.3%)"


def test_check_fails_at_9_invalid_of_120(draft, capsys):
    write_jsonl(draft / SHEET, mark(cover(draft, 120), 111, 9, "fixed"))
    assert cli.main(["review", "check", "--dataset", str(draft)]) == 1
    assert capsys.readouterr().out.splitlines()[0] == "111/120 valid (92.5%)"


def test_check_lists_open_items(draft, capsys):
    rows = mark(cover(draft, 120), 118, 2, None)
    rows[5] = {**rows[5], "verdict": None}
    write_jsonl(draft / SHEET, rows)
    assert cli.main(["review", "check", "--dataset", str(draft)]) == 1
    out = capsys.readouterr().out.splitlines()
    assert out[0] == "117/119 valid (98.3%)"
    assert f"open: {rows[0]['item']} invalid and neither fixed nor dropped" in out
    assert f"open: {rows[5]['item']} not reviewed" in out


def test_check_needs_a_reason_for_an_invalid_item(draft):
    rows = mark(cover(draft, 120), 119, 1, "fixed")
    rows[0]["reason"] = None
    write_jsonl(draft / SHEET, rows)
    assert any("no reason" in l for l in review.evaluate(draft)[1])


def test_check_needs_the_sample_to_be_there(draft):
    review.sample(draft, "all")
    rows = [r for r in sheet(draft) if r["group"] != "multi-hop"]
    write_jsonl(draft / SHEET, mark(rows, len(rows), 0, None))
    summary, open_items = review.evaluate(draft)
    assert any("multi-hop" in l for l in open_items) and summary.startswith(f"{len(rows)}/{len(rows)}")


def test_check_with_no_sheet_fails(draft, capsys):
    assert cli.main(["review", "check", "--dataset", str(draft)]) == 1
    assert capsys.readouterr().out.startswith("0/0 valid (0.0%)")


def test_check_by_split(draft):
    review.sample(draft, "all")
    rows = [r if r["split"] == "dev" or r["type"] == "note" else {**r, "verdict": None} for r in mark(sheet(draft), 40, 0, None)]
    write_jsonl(draft / SHEET, rows)
    assert review.evaluate(draft, "dev")[1] == []
    assert review.evaluate(draft, "test")[1] != []


def test_apply_drops_items_logs_them_and_rebuilds_the_qrels(draft):
    review.sample(draft, "all")
    rows = mark(sheet(draft), 40, 0, None)
    for r in rows:
        if r["item"] in ("q-alpha-005", "p-alpha-003"):
            r.update(verdict="invalid", reason="ambiguous", resolution="dropped")
    write_jsonl(draft / SHEET, rows)
    assert cli.main(["review", "apply", "--dataset", str(draft)]) == 0
    assert "q-alpha-005" not in {q["id"] for q in queries(draft)}
    assert "p-alpha-003" not in {p["id"] for p in read_jsonl(draft / "digest/prompts.jsonl")}
    assert "q-alpha-005" not in (draft / "qrels/dev.txt").read_text()
    applied = read_jsonl(draft / "generation/review/applied.jsonl")
    assert {r["item"] for r in applied} == {"q-alpha-005", "p-alpha-003"}
    cli.main(["review", "apply", "--dataset", str(draft)])
    assert len(read_jsonl(draft / "generation/review/applied.jsonl")) == 2


def test_apply_leaves_fixed_items(draft):
    review.sample(draft, "all")
    rows = [{**r, "verdict": "invalid", "reason": "typo", "resolution": "fixed"} if r["item"] == "q-alpha-001" else r for r in sheet(draft)]
    write_jsonl(draft / SHEET, rows)
    before = queries(draft)
    review.cmd_apply(type("A", (), {"dataset": draft})())
    assert queries(draft) == before


def test_apply_cannot_drop_a_note(draft):
    review.sample(draft, "all")
    rows = [{**r, "verdict": "invalid", "reason": "bad", "resolution": "dropped"} if r["type"] == "note" else r for r in sheet(draft)]
    write_jsonl(draft / SHEET, rows)
    with pytest.raises(Refused, match="note"):
        review.cmd_apply(type("A", (), {"dataset": draft})())


def test_apply_refuses_a_frozen_dataset(fixture_copy):
    with pytest.raises(Refused, match="frozen"):
        review.cmd_apply(type("A", (), {"dataset": fixture_copy})())


def test_the_committed_sheet_passes_the_check(fixture_dir, capsys):
    assert cli.main(["review", "check", "--dataset", str(fixture_dir)]) == 0
    assert capsys.readouterr().out.strip() == "40/40 valid (100.0%)"


def test_check_names_a_dropped_item_still_in_the_dataset_until_apply(draft, capsys):
    review.sample(draft, "all")
    rows = mark(sheet(draft), 40, 0, None)
    for r in rows:
        if r["item"] == "q-alpha-005":
            r.update(verdict="invalid", reason="ambiguous", resolution="dropped")
    write_jsonl(draft / SHEET, rows)
    assert cli.main(["review", "check", "--dataset", str(draft)]) == 1
    assert "open: q-alpha-005 dropped but still in the dataset; run review apply" in capsys.readouterr().out
    assert cli.main(["review", "apply", "--dataset", str(draft)]) == 0
    assert review.evaluate(draft, "all")[1] == []


def class_fix_sheet(draft: Path, fixes: int, absent: bool = True) -> list[dict]:
    rows = mark(cover(draft, 125), 125, 0, None)
    for r in rows[:fixes]:
        r.update(verdict="invalid", reason="three notes", resolution=None, class_fix="three-note multi-hop")
        if absent:
            r["item"] = f"gone-{r['item']}"
    return rows


def test_class_fix_rows_leave_the_share_and_both_numbers_print(draft, capsys):
    rows = class_fix_sheet(draft, 6)
    # 117 valid of 125 raw would need 8 invalid: add two plain invalid-and-dropped ones
    for r in rows[6:8]:
        r.update(verdict="invalid", reason="bad", resolution="dropped", item=f"gone-{r['item']}")
    write_jsonl(draft / SHEET, rows)
    assert cli.main(["review", "check", "--dataset", str(draft)]) == 0
    assert capsys.readouterr().out.strip() == (
        "117/125 valid (93.6%) raw; 117/119 (98.3%) excluding class fixes: three-note multi-hop 6")


def test_the_bar_applies_to_the_share_excluding_class_fixes(draft, capsys):
    rows = class_fix_sheet(draft, 6)
    for r in rows[6:14]:
        r.update(verdict="invalid", reason="bad", resolution="dropped", item=f"gone-{r['item']}")
    write_jsonl(draft / SHEET, rows)
    assert cli.main(["review", "check", "--dataset", str(draft)]) == 1
    assert "111/119 (93.3%) excluding class fixes" in capsys.readouterr().out


def test_a_class_fix_row_must_be_invalid(draft):
    rows = class_fix_sheet(draft, 1)
    rows[0]["verdict"] = "valid"
    write_jsonl(draft / SHEET, rows)
    assert any("class_fix" in l and "invalid" in l for l in review.evaluate(draft)[1])


def test_a_class_fix_item_must_be_absent_from_the_dataset(draft):
    write_jsonl(draft / SHEET, class_fix_sheet(draft, 1, absent=False))
    assert any("class_fix" in l and "still in the dataset" in l for l in review.evaluate(draft)[1])
