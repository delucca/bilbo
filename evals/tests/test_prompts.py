"""Digest prompt batches: allocation, ids, the four labels, resumption."""

from __future__ import annotations

import random
from argparse import Namespace
from pathlib import Path

import pytest

from bilbo_evals import common
from bilbo_evals.generate import prompts as p
from bilbo_evals.generate import queries as q
from test_queries import make_world

COUNTS = {"positive": 12, "noise": 10, "off-topic": 10, "near-miss": 10}


@pytest.fixture
def ds(tmp_path):
    return make_world(tmp_path / "ds")


def plan(ds, counts=COUNTS, split="dev"):
    return p.plan_batches(q.load_world(ds), split, counts, 7)


def test_allocate_is_capped_round_robin_and_seeded():
    got = p.allocate(7, {"a": 2, "b": None, "c": 2}, random.Random(1))
    assert got["a"] == 2 and got["c"] == 2 and got["b"] == 3
    assert p.allocate(9, {"a": 2, "b": 1}, random.Random(1)) == {"a": 2, "b": 1}
    assert p.allocate(7, {"a": None, "b": None}, random.Random(1)) == p.allocate(7, {"a": None, "b": None}, random.Random(1))


def test_batches_cover_every_label_in_tens_with_the_right_ownership(ds):
    batches, short = plan(ds)
    assert short == {}
    for label, n in COUNTS.items():
        mine = [b for b in batches if b.label == label]
        assert sum(b.n for b in mine) == n and all(b.n <= 10 for b in mine)
    none = [b for b in batches if b.label in ("noise", "off-topic")]
    assert all(b.project is None for b in none)
    assert all(b.project == "alpha" for b in batches if b.label in ("positive", "near-miss"))
    pos = [b for b in batches if b.label == "positive"]
    assert [len(b.gold) for b in pos] == [10, 2] and all(len(g) == 1 for b in pos for g in b.gold)
    assert all(n.startswith("NALPHA") for b in pos for g in b.gold for n in g)
    assert len({n for b in pos for g in b.gold for n in g}) == 12


def test_positive_notes_are_distinct_and_short_pools_are_reported(ds):
    batches, short = plan(ds, {"positive": 200})
    gold = [n for b in batches for g in b.gold for n in g]
    assert len(gold) == len(set(gold)) and short["positive"] == 200 - len(gold)


def test_ids_are_positional_in_fixed_blocks(ds):
    batches, _ = plan(ds)
    ids = {label: sorted(p.prompt_id(b, i) for b in batches if b.label == label for i in range(b.n)) for label in COUNTS}
    assert ids["positive"][0] == "p-alpha-001" and ids["positive"][-1] == "p-alpha-012"
    assert ids["near-miss"][0] == "p-alpha-101"
    assert ids["noise"][0] == "p-none-001" and ids["off-topic"][0] == "p-none-401"
    test, _ = p.plan_batches(q.load_world(ds), "test", COUNTS, 7)
    assert next(b for b in test if b.label == "noise").start == 200
    assert len({i for v in ids.values() for i in v}) == sum(COUNTS.values())


def test_prompts_name_the_label_and_never_hold_a_note_body(ds):
    batches, _ = plan(ds)
    templates = q.load_sections("prompts.md")
    for b in batches:
        text = p.build_prompt(b, templates)
        assert f"Batch: {b.key}" in text and f"exactly {b.n} strings" in text
        assert "BODYMARK" not in text
    pos = p.build_prompt(next(b for b in batches if b.label == "positive"), templates)
    assert "1. [" in pos
    near = p.build_prompt(next(b for b in batches if b.label == "near-miss"), templates)
    assert "Technologies: Go" in near and "none of the facts below answers" in near
    off = p.build_prompt(next(b for b in batches if b.label == "off-topic"), templates)
    assert "Alpha" not in off


def test_output_problem_and_rows(ds):
    batches, _ = plan(ds)
    pos = next(b for b in batches if b.label == "positive" and b.n == 10)
    assert p.output_problem(pos, {"prompts": ["x"] * 9})
    assert p.output_problem(pos, {"prompts": [f"task {i}" for i in range(10)]}) is None
    rows = p.make_rows(pos, {"prompts": [f"task {i}" for i in range(10)]})
    assert [r["gold"] for r in rows] == pos.gold and all(r["canary"] == common.CANARY for r in rows)
    noise = next(b for b in batches if b.label == "noise")
    rows = p.make_rows(noise, {"prompts": ["ok", "OK", "go on"] + [f"w{i}" for i in range(9)]})
    assert len(rows) == 9 and all(r["project"] is None and r["gold"] == [] for r in rows)
    extra = p.make_rows(noise, {"prompts": [f"w{i}" for i in range(14)]})
    assert len(extra) == noise.n


@pytest.fixture
def step(ds, fake_llm, monkeypatch):
    from bilbo_evals import dataset, llm

    monkeypatch.setattr(llm, "preflight", lambda *a, **k: None)
    monkeypatch.delenv("CODEX_HOME", raising=False)
    (Path.home() / ".codex").mkdir()
    (Path.home() / ".codex/auth.json").write_text("{}", encoding="utf-8")
    monkeypatch.setattr(dataset, "build_qrels", lambda d: None)
    batches, _ = plan(ds)
    rules = [{"match": f"Batch: {b.key}\n", "output": {"prompts": [f"{b.label} {b.key} {i}" for i in range(b.n)]}} for b in batches]
    fake_llm.set_script(rules)
    return Namespace(ds=ds, fake=fake_llm, batches=batches, run=lambda: p.cmd(Namespace(dataset=str(ds), split="dev")))


def test_cmd_writes_prompts_with_labels_and_is_resumable(step):
    assert step.run() == 0
    rows = common.read_jsonl(step.ds / "digest/prompts.jsonl")
    assert len(rows) == sum(COUNTS.values())
    assert {r["label"] for r in rows} == set(COUNTS)
    assert all(r["gold"] == [] for r in rows if r["label"] != "positive")
    assert all(len(r["gold"]) == 1 for r in rows if r["label"] == "positive")
    calls = len(step.fake.calls())
    assert calls == len(step.batches) and step.run() == 0 and len(step.fake.calls()) == calls


def test_cmd_retries_a_short_batch_once_more(step):
    pos = next(b for b in step.batches if b.label == "positive" and b.n == 10)
    good = {"prompts": [f"task {i}" for i in range(10)]}
    rules = [{"match": f"Batch: {pos.key}\n", "output": {"prompts": ["too few"]}, "times": 1},
             {"match": f"Batch: {pos.key}\n", "output": good}]
    rules += [{"match": f"Batch: {b.key}\n", "output": {"prompts": [f"{b.key} {i}" for i in range(b.n)]}} for b in step.batches if b is not pos]
    step.fake.set_script(rules)
    assert step.run() == 0
    rows = common.read_jsonl(step.ds / "digest/prompts.jsonl")
    assert {r["prompt"] for r in rows} >= {"task 0", "task 9"}
    assert (step.ds / f"generation/outputs/prompts/{pos.key}.2.json").exists()


def test_cmd_does_not_resurrect_a_prompt_a_review_dropped(step):
    step.run()
    rows = common.read_jsonl(step.ds / "digest/prompts.jsonl")
    gone = rows[0]["id"]
    common.write_jsonl(step.ds / "digest/prompts.jsonl", rows[1:])
    common.append_jsonl(step.ds / "generation/review/applied.jsonl", {"item": gone, "resolution": "dropped"})
    assert step.run() == 0
    assert gone not in {r["id"] for r in common.read_jsonl(step.ds / "digest/prompts.jsonl")}
