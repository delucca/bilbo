"""`l1 digest` on the fixture dataset: the rows, the metrics, the stratification and the sweep."""

from __future__ import annotations

import json
import shutil

import pytest

from bilbo_evals import common, digest, results, runner
from test_runner import BILBO, digest_args, env, fake_bilbo, live, rows_of, run_args, shared_env  # noqa: F401


def row(label, injected, hit=None, ranking="meaning", error=None, config="meaning@0.55"):
    return {"item": "p", "label": label, "split": "dev", "config": config, "injected": injected, "shown": ["x"] * injected,
            "hit": hit, "ranking": ranking, "passed": 0, "error": error, "latency_ms": 1.0}  # fmt: skip


def test_summarize_by_hand():
    rows = (
        [row("noise", True), row("noise", False), row("noise", False), row("noise", False)]
        + [row("off-topic", False)] * 2
        + [row("near-miss", True), row("near-miss", True)]
        + [row("positive", True, True), row("positive", True, False), row("positive", False), row("positive", True, True)]
    )
    s = digest.summarize(rows)
    assert s["fir"] == {"noise": 0.25, "off-topic": 0.0, "near-miss": 1.0}
    assert s["coverage"] == 0.75
    assert s["hit_given_inject"] == pytest.approx(2 / 3)
    assert s["fir_all"] == pytest.approx(3 / 8)


def test_an_empty_denominator_is_none_not_zero():
    s = digest.summarize([row("noise", False)])
    assert s["coverage"] is None and s["hit_given_inject"] is None and s["fir"]["off-topic"] is None


def test_strata_split_ranking_and_error():
    rows = [row("noise", False)] * 3 + [row("noise", True, ranking="keywords", error="embedder unavailable")] * 7
    got = digest.by_stratum(rows)
    assert {k: v["n"] for k, v in got.items()} == {"meaning": 3, "keywords, error": 7}
    assert got["keywords, error"]["fir"]["noise"] == 1.0


def test_the_report_shows_seven_fallback_prompts_apart():
    rows = [row("noise", False)] * 3 + [row("noise", True, ranking="keywords", error="embedder unavailable")] * 7
    text = "\n".join(digest.section({"run": {}, "rows": rows, "sweep": None}))
    line = next(l for l in text.splitlines() if l.startswith("| meaning@0.55: keywords, error"))
    assert line.split("|")[2].strip() == "7"
    assert next(l for l in text.splitlines() if l.startswith("| meaning@0.55: meaning")).split("|")[2].strip() == "3"


def test_the_sweep_grid_is_twenty_one_points():
    assert digest.SWEEP[0] == 0.3 and digest.SWEEP[-1] == 0.8 and len(digest.SWEEP) == 21
    assert 0.55 in digest.SWEEP and 0.325 in digest.SWEEP


def test_the_shared_digest_run():
    shared = shared_env()
    folder = shared.runs / "shared-dev/digest"
    meta = json.loads((folder / "run.json").read_text())
    assert meta["split"] == "dev" and meta["bilbo_config"]["digest.log"] == "on"
    assert meta["bilbo_config"]["digest.min_similarity"] == "0.55"
    assert results.check_files(folder) == []
    rows = common.read_jsonl(folder / "per_item.jsonl")
    prompts = [p for p in common.read_jsonl(shared.ds / "digest/prompts.jsonl") if p["split"] == "dev"]
    assert len(rows) == 2 * len(prompts)
    assert {r["config"] for r in rows} == {"meaning@0.55", "keywords"}
    assert all(set(r) == {"item", "label", "split", "config", "injected", "shown", "hit", "ranking", "passed", "error",
                          "latency_ms"} for r in rows)  # fmt: skip
    assert {r["ranking"] for r in rows if r["config"] == "keywords"} <= {"keywords", "none"}
    assert any(r["ranking"] == "meaning" for r in rows if r["config"] == "meaning@0.55")
    assert not (folder / "sweep.jsonl").exists()
    positives = [r for r in rows if r["label"] == "positive"]
    assert all((r["hit"] is None) == (not r["injected"]) for r in positives)
    assert all(r["hit"] is None for r in rows if r["label"] != "positive")
    ids = set(__import__("bilbo_evals.dataset", fromlist=["x"]).load(shared.ds).notes)
    assert all(set(r["shown"]) <= ids for r in rows)


def test_the_report_has_the_operating_point_and_no_auroc():
    text = (shared_env().runs / "shared-dev/report.md").read_text()
    assert "## Digest" in text and "false injection: noise" in text and "meaning@0.55" in text and "keywords" in text
    assert "auroc" not in text.lower()
    assert "Approximate operating curve" not in text


def test_a_sweep_is_labelled_approximate_marks_the_default_and_names_no_auroc(live, capsys):
    assert digest.cmd(digest_args(live, sweep=True)) == 0
    run = next(live.runs.iterdir())
    sweep = common.read_jsonl(run / "digest/sweep.jsonl")
    prompts = [p for p in common.read_jsonl(live.ds / "digest/prompts.jsonl") if p["split"] == "dev"]
    assert len(sweep) == 21 * len(prompts)
    assert [t for t in sorted({r["config"] for r in sweep})] == sorted(digest.config_name(t) for t in digest.SWEEP)
    text = capsys.readouterr().out
    assert "### Approximate operating curve" in text
    assert "| 0.55 * |" in text and text.count(" * |") == 1
    assert "auroc" not in text.lower()
    assert (run / "report.md").read_text().strip() == text.strip()
    assert results.check_files(run / "digest") == []


def test_a_fallback_is_not_hidden(live):
    stopped = []
    real = digest.run_config

    def run_config(sess, prompts, threshold):
        if threshold == digest.THRESHOLD and not stopped:
            live.fake.stop()
            stopped.append(True)
        return real(sess, prompts, threshold)

    with pytest.MonkeyPatch.context() as mp:
        mp.setattr(digest, "run_config", run_config)
        assert digest.cmd(digest_args(live)) == 0
    live.fake = None
    run = next(live.runs.iterdir())
    rows = [r for r in common.read_jsonl(run / "digest/per_item.jsonl") if r["config"] == "meaning@0.55"]
    assert rows and all(r["ranking"] == "keywords" and r["error"] for r in rows)
    text = (run / "report.md").read_text()
    assert f"| meaning@0.55: keywords, error | {len(rows)} |" in text


def test_every_prompt_gets_a_fresh_session_id(live, tmp_path):
    log = tmp_path / "stdin.log"
    bilbo = fake_bilbo(tmp_path, digest=f'tee -a "{log}" | "$REAL" digest; echo >> "{log}"; exit 0')
    assert digest.cmd(digest_args(live, bilbo=bilbo)) == 0
    sent = [json.loads(l) for l in log.read_text().splitlines() if l.strip()]
    prompts = [p for p in common.read_jsonl(live.ds / "digest/prompts.jsonl") if p["split"] == "dev"]
    assert len(sent) == 2 * len(prompts)
    assert len({s["session_id"] for s in sent}) == len(sent)
    assert {s["prompt"] for s in sent} == {p["prompt"] for p in prompts}


def test_a_draft_test_digest_is_refused(live):
    with pytest.raises(common.Refused, match="draft"):
        digest.cmd(digest_args(live, split="test", draft=True))
    assert not live.runs.exists()


def test_a_withheld_passage_refuses_the_digest(live, tmp_path):
    bilbo = fake_bilbo(tmp_path, index='"$REAL" index; echo "bilbo: withheld 2 passages" >&2; exit 0')
    with pytest.raises(common.Refused, match="withheld 2 passages"):
        digest.cmd(digest_args(live, bilbo=bilbo))
    assert not live.runs.exists()


def test_a_digest_joins_an_existing_run_only_once(live):
    runner.cmd_run(run_args(live, arms="ripgrep", run_id="r1"))
    assert digest.cmd(digest_args(live, run=live.runs / "r1")) == 0
    assert results.load_run(live.runs / "r1")["digest"] is not None
    with pytest.raises(common.Refused, match="digest run already"):
        digest.cmd(digest_args(live, run=live.runs / "r1"))


def test_a_digest_refuses_a_run_of_another_split(live):
    runner.cmd_run(run_args(live, split="test", arms="ripgrep", run_id="t1"))
    with pytest.raises(common.Refused, match="not dev"):
        digest.cmd(digest_args(live, run=live.runs / "t1"))
