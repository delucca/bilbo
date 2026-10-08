"""`compare`: the refusals, the warning, the deltas by hand and the tamper check."""

from __future__ import annotations

import json
import shutil
from pathlib import Path

import pytest

from bilbo_evals import cli, common, results
from test_report import patched, synth, table  # noqa: F401
from test_runner import env, shared_env  # noqa: F401


def compare(base: Path, run: Path) -> int:
    return cli.main(["compare", str(base), str(run)])


def edit_meta(run: Path, fn) -> None:
    for p in run.glob("*/run.json"):
        m = json.loads(p.read_text())
        fn(m)
        p.write_text(json.dumps(m))


@pytest.fixture
def pair(env):
    base = synth(env, "base", {"bilbo-full": 0.5, "bm25-ref": 0.0, "ripgrep": 1.0}, latency=10.0)
    new = synth(env, "new", {"bilbo-full": 1.0, "bm25-ref": 0.0, "ripgrep": 1.0}, latency=12.0)
    return env, base, new


def test_two_runs_of_one_tree_print_deltas_and_exit_0(pair, capsys):
    _, base, new = pair
    assert compare(base, new) == 0
    out = capsys.readouterr().out
    line = next(l for l in out.splitlines() if l.startswith("| bilbo-full | all note queries |"))
    cells = [c.strip() for c in line.strip("|").split("|")]
    assert cells[2] == "7"
    assert cells[3] == "+0.500 [+0.500, +0.500]"
    same = next(l for l in out.splitlines() if l.startswith("| bm25-ref | all note queries |"))
    assert "+0.000 [+0.000, +0.000]" in same
    assert "0.19.0 to 0.19.0" in out


def test_deltas_are_paired_by_item_over_families(pair, capsys):
    env, base, new = pair
    rows = common.read_jsonl(new / "bilbo-full/per_item.jsonl")
    for r in rows:
        if r["item"] == "q-beta-001":
            r["metrics"]["success@5"] = 0.0
    results.write_arm(new, "bilbo-full", json.loads((new / "bilbo-full/run.json").read_text()), rows, None)
    assert compare(base, new) == 0
    line = next(l for l in capsys.readouterr().out.splitlines() if l.startswith("| bilbo-full | known-item |"))
    assert "-0.500 [-0.500, -0.500]" in line


def test_the_latency_delta_needs_the_same_host(pair, capsys):
    _, base, new = pair
    assert compare(base, new) == 0
    out = capsys.readouterr().out
    assert "| ripgrep | +2.0 | +2.0 |" in out and "| bm25-ref |" not in out.split("## Latency")[1]
    edit_meta(new, lambda m: m["host"].update(cpu="other"))
    assert compare(base, new) == 0
    out = capsys.readouterr().out
    assert "## Latency" not in out and "Latency deltas are omitted" in out


def test_different_trees_are_refused_with_both_hashes(pair, capsys):
    _, base, new = pair
    edit_meta(new, lambda m: m["dataset"].update(tree_hash="ff" * 32))
    assert compare(base, new) == 1
    err = capsys.readouterr().err
    assert "ff" * 32 in err and (pair[0].ds / "FROZEN").read_text().strip() in err


@pytest.mark.parametrize("key", ["model", "gguf_sha256", "query_prefix"])
def test_different_embedders_are_refused(pair, capsys, key):
    _, base, new = pair
    edit_meta(new, lambda m: m["embedder"].update({key: "other"}))
    assert compare(base, new) == 1
    assert key in capsys.readouterr().err


def test_only_the_llama_server_version_differing_warns(pair, capsys):
    _, base, new = pair
    edit_meta(new, lambda m: m["embedder"].update(llama_server_version="9999"))
    assert compare(base, new) == 0
    cap = capsys.readouterr()
    assert "warning" in cap.err and "9190" in cap.err and "9999" in cap.err
    assert "all note queries" in cap.out


def test_an_edited_baseline_is_named_and_refused(pair, capsys):
    _, base, new = pair
    path = base / "bilbo-full/per_item.jsonl"
    path.write_text(path.read_text().replace("0.5", "0.6", 1))
    assert compare(base, new) == 1
    assert str(path) in capsys.readouterr().err


def test_an_edited_run_is_refused_too(pair, capsys):
    _, base, new = pair
    path = new / "ripgrep/per_item.jsonl"
    path.write_text(path.read_text() + "\n")
    assert compare(base, new) == 1
    assert str(path) in capsys.readouterr().err


def test_a_stray_file_in_an_arm_folder_is_refused(pair, capsys):
    _, base, new = pair
    (base / "ripgrep/notes.txt").write_text("x")
    assert compare(base, new) == 1
    assert "notes.txt" in capsys.readouterr().err


def test_drafts_cannot_be_compared(env, capsys):
    a = synth(env, "a", {"bilbo-full": 1.0}, split="dev", draft=True)
    b = synth(env, "b", {"bilbo-full": 1.0}, split="dev", draft=True)
    assert compare(a, b) == 1


def test_a_real_run_compares_with_a_copy_of_itself(monkeypatch, tmp_path, capsys):
    shared = patched(monkeypatch)
    copy = tmp_path / "copy"
    shutil.copytree(shared.runs / "shared-dev", copy)
    assert compare(shared.runs / "shared-dev", copy) == 0
    out = capsys.readouterr().out
    assert out.startswith(results.CLAIM)
    assert "| bilbo-full | all note queries |" in out


def test_a_tampered_digest_folder_is_refused(monkeypatch, tmp_path, capsys):
    shared = patched(monkeypatch)
    copy = tmp_path / "copy"
    shutil.copytree(shared.runs / "shared-dev", copy)
    path = copy / "digest/per_item.jsonl"
    path.write_text(path.read_text().replace("noise", "noyse", 1))
    assert compare(shared.runs / "shared-dev", copy) == 1
    assert str(path) in capsys.readouterr().err


def test_a_folder_that_is_not_a_run_is_refused(tmp_path, capsys):
    assert compare(tmp_path, tmp_path) == 1
    assert "not a run" in capsys.readouterr().err
