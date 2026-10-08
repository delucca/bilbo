"""`report` on real runs of the fixture dataset and on hand-made runs whose statistics can be worked by hand."""

from __future__ import annotations

import json
import re

import pytest

from bilbo_evals import cli, common, results
from bilbo_evals.arms import PROCESS_ARMS
from test_runner import env, shared_env  # noqa: F401


def patched(monkeypatch):
    shared = shared_env()
    monkeypatch.setattr(common, "EVALS_ROOT", shared.base)
    monkeypatch.setattr(common, "RUNS_DIR", shared.runs)
    monkeypatch.setattr(common, "TEST_RUNS_LOG", shared.base / "test-runs.jsonl")
    return shared


@pytest.fixture
def shared(monkeypatch):
    return patched(monkeypatch)


def table(text: str, heading: str) -> list[list[str]]:
    """Rows of the first table after `## heading`, as lists of stripped cells (header and rule dropped)."""
    block = text.split(f"## {heading}\n", 1)[1].split("\n## ", 1)[0]
    lines = []
    for l in block.splitlines():
        if l.startswith("|"):
            lines.append(l)
        elif lines:
            break
    return [[c.strip() for c in l.strip("|").split("|")] for l in lines[2:]]


def meta(env, arm, run_id, split="test", draft=False, **kw):
    tree = (env.ds / "FROZEN").read_text().strip()
    base = {
        "schema_version": 1, "run_id": run_id, "layer": "L1", "arm": arm, "split": split, "draft": draft,
        "dataset": {"name": "fixture", "version": "v1", "tree_hash": None if draft else tree},
        "bilbo": {"version": "0.19.0", "commit": None, "sha256": "ab" * 32, "path": "bilbo"},
        "embedder": {"model": "qwen3-embedding-0.6b", "gguf_sha256": "cd" * 32, "llama_server": "llama-server",
                     "llama_server_version": "9190", "query_prefix": "Q: "},
        "bilbo_config": {}, "parity": None, "index": None, "versions": {}, "seeds": [],
        "host": {"platform": "Darwin", "machine": "arm64", "cpu": "M"}, "root": "/tmp/x", "folders": {},
        "started": "2026-10-07T00:00:00Z", "ended": "2026-10-07T00:01:00Z",
    }  # fmt: skip
    base.update(kw)
    return base


def synth(env, run_id, values: dict[str, float], split="test", draft=False, latency=None, **kw):
    """A run whose arms score every note query `values[arm]` on every metric (library: same), hand-writable."""
    queries = [q for q in common.read_jsonl(env.ds / "queries.jsonl") if q["split"] == split]
    run = env.runs / run_id
    for arm, v in values.items():
        trials = 20 if arm == "random" else 1
        rows = []
        for q in queries:
            na = q["stratum"] == "no-answer"
            for t in range(trials):
                val = v(t) if callable(v) else v
                m = {"empty": 1.0} if na else {k: val for k in ("success@5", "rr", "ndcg@10", "r@10", "judged@10")}
                if not na:
                    m.update({"evidence@10": None, "new_above_old": None, "empty": 0.0})
                rows.append({"item": q["id"], "stratum": q["stratum"], "split": split, "trial": t,
                             "ranking": None if arm == "random" else [], "metrics": m,
                             "latency_ms": latency if arm in PROCESS_ARMS else None, "exit": None, "warnings": [],
                             "fallback": False, "error": None, "tokens_in": None, "tokens_out": None, "cost_usd": None})  # fmt: skip
        results.write_arm(run, arm, meta(env, arm, run_id, split, draft, **kw), rows, None)
    return run


def test_the_claim_is_the_first_line_and_the_title_is_not_a_draft(shared, capsys):
    run = shared.runs / "shared-dev"
    assert results.cmd_report(type("A", (), {"run": str(run)})) == 0
    text = (run / "report.md").read_text()
    assert text.splitlines()[0] == "These numbers measure retrieval over curated synthetic notes."
    assert capsys.readouterr().out.strip() == text.strip()
    assert "DRAFT" not in text.splitlines()[2]


def test_identity_block(shared):
    text = results.report(shared.runs / "shared-dev")
    tree = (shared.ds / "FROZEN").read_text().strip()
    assert f"tree hash {tree}" in text and "fixture/v1" in text
    assert "binary sha256" in text and "qwen3-embedding-0.6b" in text


def test_the_stratum_table_has_every_arm_and_stratum_and_matches_the_records(shared):
    run = shared.runs / "shared-dev"
    rows = table(results.report(run), "Metrics by stratum")
    arms = {r[0] for r in rows}
    assert arms == {"random", "ripgrep", "bm25-ref", "dense-ref", "bilbo-keyword", "bilbo-full"}
    assert {r[1] for r in rows} == {"all note queries", "known-item", "paraphrase", "pt-en", "alias", "supersession", "multi-hop",
                                    "kind-filter"}  # fmt: skip
    for arm, stratum, n, s5, rr, nd, r10 in rows:
        recs = common.read_jsonl(run / arm / "per_item.jsonl")
        mine = [r for r in recs if (r["stratum"] == stratum if stratum != "all note queries" else r["stratum"] not in
                                    ("library", "no-answer"))]  # fmt: skip
        assert int(n) == len({r["item"] for r in mine})
        by_item: dict[str, list[float]] = {}
        for r in mine:
            by_item.setdefault(r["item"], []).append(r["metrics"]["success@5"])
        assert float(s5) == pytest.approx(sum(sum(v) / len(v) for v in by_item.values()) / len(by_item), abs=5e-4)


def test_an_empty_ranking_counts_as_zero_in_n(env):
    run = synth(env, "r1", {"bilbo-keyword": 0.0, "bilbo-full": 1.0}, split="dev")
    rows = table(results.report(run), "Metrics by stratum")
    keyword = next(r for r in rows if r[0] == "bilbo-keyword" and r[1] == "paraphrase")
    assert keyword[2:] == ["1", "0.000", "0.000", "0.000", "0.000"]


def test_zero_overlap_and_library_tables(shared):
    text = results.report(shared.runs / "shared-dev")
    zero = table(text, "Zero-overlap queries")
    assert {r[0] for r in zero} == {"random", "ripgrep", "bm25-ref", "dense-ref", "bilbo-keyword", "bilbo-full"}
    queries = common.read_jsonl(shared.ds / "queries.jsonl")
    expected = sum(1 for q in queries if q["split"] == "dev" and q["stratum"] != "library" and q.get("zero_overlap"))
    assert expected > 0
    assert {r[2] for r in zero} == {str(expected)}
    lib = table(text, "Library")
    assert {r[1] for r in lib} == {"library"} and {r[2] for r in lib} == {"1"}


def test_judged_extras_and_health_tables(shared):
    text = results.report(shared.runs / "shared-dev")
    assert len(table(text, "Judged@10")) == 6
    extras = table(text, "Multi-hop, supersession and no-answer")
    assert len(extras) == 6
    health = table(text, "Errors and fallbacks")
    assert all(r[2] == "0" and r[3] == "0" for r in health)


def test_latency_only_for_process_arms(shared):
    rows = {r[0]: r[1:] for r in table(results.report(shared.runs / "shared-dev"), "Latency")}
    assert set(rows) == {"random", "ripgrep", "bm25-ref", "dense-ref", "bilbo-keyword", "bilbo-full"}
    for arm, cells in rows.items():
        if arm in PROCESS_ARMS:
            assert float(cells[0]) <= float(cells[1]) and float(cells[0]) > 0
        else:
            assert cells == ["-", "-"]


def test_a_dev_report_labels_every_p_value_exploratory(shared):
    text = results.report(shared.runs / "shared-dev")
    stats = table(text, "Paired statistics")
    assert stats
    assert all(r[6].endswith("exploratory") for r in stats)
    assert "principal, exploratory" in {r[1] for r in stats}
    assert next(r for r in stats if r[0] == "bm25-ref")[1] == "principal, exploratory"


def test_the_digest_section_is_there_and_names_no_auroc(shared):
    text = results.report(shared.runs / "shared-dev")
    assert "## Digest" in text and "### By ranking and error" in text
    assert "auroc" not in text.lower()


def test_the_principal_test_and_the_random_floor_by_hand(env):
    run = synth(env, "t1", {"bilbo-full": 1.0, "bm25-ref": 0.0, "ripgrep": 0.0, "random": lambda t: float(t % 2)})
    text = results.report(run)
    stats = {r[0]: r for r in table(text, "Paired statistics")}
    principal = stats["bm25-ref"]
    assert principal[1] == "principal" and principal[2] == "7"
    assert principal[5] == "+1.000 [+1.000, +1.000]"
    assert principal[6] == "0.1250" and principal[7] == "-"
    assert stats["ripgrep"][1] == "secondary" and stats["ripgrep"][6] == "0.1250" and stats["ripgrep"][7] == "0.2500"
    floor = stats["random"]
    assert floor[3] == "1.000" and floor[4] == "0.500" and floor[5] == "+0.500 [+0.500, +0.500]"
    assert floor[6] == "0.1250" and floor[7] == "0.2500"
    assert "exploratory" not in text


def test_a_test_report_counts_earlier_runs(env):
    tree = (env.ds / "FROZEN").read_text().strip()
    for i in (1, 2):
        results.log_test_run(f"old{i}", tree, "0.18.0")
    results.log_test_run("t1", tree, "0.19.0")
    run = synth(env, "t1", {"bilbo-full": 1.0, "bm25-ref": 0.0})
    assert "- 2 earlier test runs on this dataset" in results.report(run)
    results.log_test_run("t2", tree, "0.19.0")
    assert "- 2 earlier test runs on this dataset" in results.report(run)
    assert results.log_test_run("t3", tree, "0.19.0") == 4


def test_a_dev_report_does_not_count_test_runs(shared):
    assert "earlier test run" not in results.report(shared.runs / "shared-dev")


def test_a_draft_report_says_draft_in_the_title(env):
    run = synth(env, "d1", {"bilbo-full": 1.0, "bm25-ref": 0.0}, split="dev", draft=True)
    text = results.report(run)
    assert "DRAFT" in text.splitlines()[2] and "none (draft)" in text


def test_the_report_without_bilbo_full_still_prints(env):
    run = synth(env, "t1", {"bm25-ref": 0.5})
    assert "need `bilbo-full`" in results.report(run)


def test_a_folder_without_run_json_is_not_a_run(tmp_path, capsys):
    assert cli.main(["report", str(tmp_path)]) == 1
    assert "not a run" in capsys.readouterr().err


def test_a_run_whose_dataset_changed_is_refused(env):
    run = synth(env, "t1", {"bilbo-full": 1.0})
    (env.ds / "queries.jsonl").write_text((env.ds / "queries.jsonl").read_text() + "\n")
    with pytest.raises(common.Refused, match="no longer matches"):
        results.report(run)


def test_run_json_hashes_every_other_file(env):
    run = synth(env, "t1", {"bilbo-full": 1.0})
    meta_ = json.loads((run / "bilbo-full/run.json").read_text())
    assert set(meta_["files"]) == {"per_item.jsonl"}
    assert meta_["files"]["per_item.jsonl"] == common.sha256_file(run / "bilbo-full/per_item.jsonl")
    assert re.fullmatch(r"[0-9a-f]{64}", meta_["files"]["per_item.jsonl"])
