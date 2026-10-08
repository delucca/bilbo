"""`l1 run` on the fixture dataset with the real arms, a real bilbo and the fake embedder."""

from __future__ import annotations

import argparse
import atexit
import json
import os
import pwd
import shutil
import stat
import tempfile
from dataclasses import dataclass
from pathlib import Path

import pytest

from bilbo_evals import arms as arms_pkg
from bilbo_evals import common, dataset, results, runner
from bilbo_evals.arms import ARMS
from fake_embedder import FakeEmbedder

FIXTURE = Path(__file__).resolve().parent / "fixtures/notes-fixture"
BILBO = Path(os.environ.get("BILBO_BIN") or Path(__file__).resolve().parents[2] / "target/debug/bilbo")


@dataclass
class Env:
    base: Path
    ds: Path
    fake: FakeEmbedder | None = None

    @property
    def runs(self) -> Path:
        return self.base / "runs"


def make_env(base: Path) -> Env:
    ds = base / "datasets/fixture/v1"
    shutil.copytree(FIXTURE, ds)
    return Env(base, ds)


def patch(mp: pytest.MonkeyPatch, env: Env) -> None:
    mp.setattr(common, "EVALS_ROOT", env.base)
    mp.setattr(common, "RUNS_DIR", env.runs)
    mp.setattr(common, "TEST_RUNS_LOG", env.base / "test-runs.jsonl")
    mp.setenv("TMPDIR", str(env.base / "tmp"))
    (env.base / "tmp").mkdir(exist_ok=True)


def run_args(env: Env, split: str = "dev", arms: str = "all", bilbo: Path = BILBO, **kw) -> argparse.Namespace:
    base = dict(split=split, arms=arms, bilbo=bilbo, draft=False, dataset=env.ds, model=None, llama_server="llama-server",
                run_id=None, keep_root=False, embedder_url=env.fake.url if env.fake else None)  # fmt: skip
    return argparse.Namespace(**{**base, **kw})


def digest_args(env: Env, split: str = "dev", bilbo: Path = BILBO, **kw) -> argparse.Namespace:
    base = dict(split=split, bilbo=bilbo, sweep=False, draft=False, dataset=env.ds, model=None, llama_server="llama-server",
                run=None, embedder_url=env.fake.url if env.fake else None)  # fmt: skip
    return argparse.Namespace(**{**base, **kw})


def fake_bilbo(base: Path, recall: str = "", index: str = "", digest: str = "") -> Path:
    """A wrapper that runs the real bilbo except for the verbs it is given shell snippets for."""
    script = base / "fake-bilbo"
    parts = ['#!/bin/sh', f'REAL="{BILBO}"']
    for verb, body in (("recall", recall), ("index", index), ("digest", digest)):
        if body:
            parts.append(f'if [ "$1" = {verb} ]; then\n{body}\nfi')
    parts.append('exec "$REAL" "$@"')
    script.write_text("\n".join(parts) + "\n")
    script.chmod(script.stat().st_mode | stat.S_IXUSR)
    return script


_SHARED: dict[str, Env] = {}


def shared_env() -> Env:
    """One dev run of all six arms and a digest run, built once per session and read by the report and compare tests."""
    if "dev" not in _SHARED:
        base = Path(tempfile.mkdtemp(prefix="bilbo-evals-h-"))
        atexit.register(shutil.rmtree, base, True)
        env = make_env(base)
        env.fake = FakeEmbedder().start()
        atexit.register(env.fake.stop)
        with pytest.MonkeyPatch.context() as mp:
            patch(mp, env)
            assert runner.cmd_run(run_args(env, run_id="shared-dev")) == 0
            from bilbo_evals import digest

            assert digest.cmd(digest_args(env, run=env.runs / "shared-dev")) == 0
        _SHARED["dev"] = env
    return _SHARED["dev"]


@pytest.fixture
def env(tmp_path, monkeypatch):
    e = make_env(tmp_path)
    patch(monkeypatch, e)
    return e


@pytest.fixture
def live(env, fake_embedder):
    env.fake = fake_embedder
    return env


def rows_of(run: Path, arm: str) -> list[dict]:
    return common.read_jsonl(run / arm / "per_item.jsonl")


def test_a_dev_run_writes_one_folder_per_arm(live, capsys):
    assert runner.cmd_run(run_args(live, run_id="r1")) == 0
    run = live.runs / "r1"
    assert sorted(p.name for p in run.iterdir() if p.is_dir()) == sorted(ARMS)
    assert (run / "report.md").read_text().startswith(results.CLAIM)
    assert capsys.readouterr().out.startswith(results.CLAIM)
    queries = common.read_jsonl(live.ds / "queries.jsonl")
    dev = [q for q in queries if q["split"] == "dev"]
    for arm in ARMS:
        meta = json.loads((run / arm / "run.json").read_text())
        assert (meta["schema_version"], meta["layer"], meta["arm"], meta["split"], meta["draft"]) == (1, "L1", arm, "dev", False)
        assert meta["dataset"] == {"name": "fixture", "version": "v1", "tree_hash": (live.ds / "FROZEN").read_text().strip(),
                                 "path": "datasets/fixture/v1"}
        assert results.check_files(run / arm) == []
        rows = rows_of(run, arm)
        per_item = 20 if arm == "random" else 1
        assert len(rows) == len(dev) * per_item
        assert {r["item"] for r in rows} == {q["id"] for q in dev}
        assert all(set(r) == {"item", "stratum", "split", "trial", "ranking", "metrics", "latency_ms", "exit", "warnings",
                              "fallback", "error", "tokens_in", "tokens_out", "cost_usd"} for r in rows)  # fmt: skip
        assert all(r["tokens_in"] is None and r["cost_usd"] is None for r in rows)
    assert not (run / "random/run.trec").exists()
    assert (run / "bilbo-full/run.trec").is_file()


def test_random_has_twenty_seeds_and_no_ranking(live):
    runner.cmd_run(run_args(live, arms="random", run_id="r1"))
    meta = json.loads((live.runs / "r1/random/run.json").read_text())
    assert meta["seeds"] == list(range(20))
    rows = rows_of(live.runs / "r1", "random")
    assert {r["trial"] for r in rows} == set(range(20))
    assert all(r["ranking"] is None for r in rows)


def test_no_answer_items_carry_only_the_empty_metric(live):
    runner.cmd_run(run_args(live, arms="bilbo-keyword,bm25-ref", run_id="r1"))
    for arm in ("bilbo-keyword", "bm25-ref"):
        na = [r for r in rows_of(live.runs / "r1", arm) if r["stratum"] == "no-answer"]
        assert na and all(set(r["metrics"]) == {"empty"} for r in na)
        note = [r for r in rows_of(live.runs / "r1", arm) if r["stratum"] == "known-item"]
        assert set(note[0]["metrics"]) == {"success@5", "rr", "ndcg@10", "r@10", "judged@10", "evidence@10", "new_above_old", "empty"}


def test_latency_is_recorded_for_process_arms_only(live):
    runner.cmd_run(run_args(live, run_id="r1"))
    for arm in ARMS:
        latencies = [r["latency_ms"] for r in rows_of(live.runs / "r1", arm)]
        if arm in arms_pkg.PROCESS_ARMS:
            assert all(v is not None and v > 0 for v in latencies)
        else:
            assert all(v is None for v in latencies)


def test_run_json_records_identity_embedder_and_folders(live):
    runner.cmd_run(run_args(live, arms="bilbo-full,bilbo-keyword", run_id="r1"))
    full = json.loads((live.runs / "r1/bilbo-full/run.json").read_text())
    keyword = json.loads((live.runs / "r1/bilbo-keyword/run.json").read_text())
    assert full["bilbo"]["version"] == runner.bilbo_identity(BILBO)["version"]
    assert full["bilbo"]["sha256"] == common.sha256_file(BILBO)
    assert full["bilbo"]["path"] == "target/debug/bilbo" or full["bilbo"]["path"] == "bilbo"
    assert full["embedder"]["model"] == "qwen3-embedding-0.6b"
    assert full["embedder"]["llama_server_version"] == "attached"
    assert full["parity"] == "ok" and full["index"]["embedded"] > 0
    assert full["bilbo_config"] == {"embedder.model": "qwen3-embedding-0.6b"}
    assert keyword["embedder"] is None and keyword["parity"] is None and keyword["bilbo_config"] == {}
    assert full["root"] == "$TMPDIR/bilbo-evals-r1"
    assert set(full["folders"]) == {"store", "config", "cache", "state"}
    assert all(p.startswith(full["root"] + "/") for p in full["folders"].values())
    assert set(full["versions"]) >= {"python", "bilbo_evals", "ir_measures", "bm25s", "PyStemmer", "numpy", "ripgrep", "llama_server"}
    assert set(full["host"]) == {"platform", "machine", "cpu"}
    assert full["started"] <= full["ended"]


def test_the_sandbox_is_destroyed_unless_kept(live):
    runner.cmd_run(run_args(live, arms="ripgrep", run_id="gone"))
    assert not (live.base / "tmp/bilbo-evals-gone").exists()
    runner.cmd_run(run_args(live, arms="ripgrep", run_id="kept", keep_root=True))
    assert (live.base / "tmp/bilbo-evals-kept/store/notes").is_dir()


def test_no_user_home_path_in_a_run_folder(live):
    runner.cmd_run(run_args(live, run_id="r1"))
    real_home = pwd.getpwuid(os.getuid()).pw_dir
    assert real_home not in "".join(p.read_text() for p in (live.runs / "r1").rglob("*") if p.is_file())
    with pytest.MonkeyPatch.context() as mp:
        mp.setenv("HOME", real_home)
        assert dataset._home_problems(live.runs / "r1") == []


def test_a_draft_test_run_is_refused_and_writes_nothing(live, capsys):
    with pytest.raises(common.Refused, match="draft"):
        runner.cmd_run(run_args(live, split="test", draft=True))
    assert not live.runs.exists()


def test_an_unfrozen_dataset_needs_draft(live):
    (live.ds / "FROZEN").unlink()
    with pytest.raises(common.Refused, match="not frozen"):
        runner.cmd_run(run_args(live))
    assert not live.runs.exists()
    assert runner.cmd_run(run_args(live, arms="ripgrep", draft=True, run_id="d")) == 0
    meta = json.loads((live.runs / "d/ripgrep/run.json").read_text())
    assert meta["draft"] is True and meta["dataset"]["tree_hash"] is None
    assert "DRAFT" in (live.runs / "d/report.md").read_text().splitlines()[2]


def test_a_changed_dataset_is_refused(live):
    (live.ds / "queries.jsonl").write_text((live.ds / "queries.jsonl").read_text() + "\n")
    with pytest.raises(common.Refused, match="MANIFEST"):
        runner.cmd_run(run_args(live))


def test_test_runs_are_logged_and_dev_runs_are_not(live):
    log = live.base / "test-runs.jsonl"
    runner.cmd_run(run_args(live, arms="ripgrep", run_id="dev1"))
    assert not log.exists()
    for i in (1, 2, 3):
        runner.cmd_run(run_args(live, split="test", arms="ripgrep", run_id=f"t{i}"))
    rows = common.read_jsonl(log)
    assert [r["run_id"] for r in rows] == ["t1", "t2", "t3"]
    assert rows[0]["tree_hash"] == (live.ds / "FROZEN").read_text().strip()
    assert set(rows[0]) == {"run_id", "tree_hash", "bilbo_version", "time"}
    assert "2 earlier test runs on this dataset" in (live.runs / "t3/report.md").read_text()
    assert "0 earlier test runs on this dataset" in (live.runs / "t1/report.md").read_text()


def test_a_withheld_passage_stops_only_the_embedder_arms(live, capsys, tmp_path):
    bilbo = fake_bilbo(tmp_path, index='"$REAL" index; echo "bilbo: withheld 2 passages" >&2; exit 0')
    code = runner.cmd_run(run_args(live, bilbo=bilbo, run_id="r1"))
    assert code == 1
    assert "withheld 2 passages" in capsys.readouterr().err
    scored = {p.name for p in (live.runs / "r1").iterdir() if p.is_dir()}
    assert scored == set(ARMS) - {"bilbo-full", "dense-ref"}


def test_a_failing_parity_stops_the_embedder_arms(live, capsys, monkeypatch):
    from bilbo_evals import passages

    monkeypatch.setattr(passages, "PART_BYTES", 10)
    code = runner.cmd_run(run_args(live, arms="bilbo-full,dense-ref,ripgrep", run_id="r1"))
    assert code == 1
    assert "parity failed" in capsys.readouterr().err
    assert {p.name for p in (live.runs / "r1").iterdir() if p.is_dir()} == {"ripgrep"}


def test_a_bilbo_crash_is_that_querys_error(live):
    bilbo = fake_bilbo(live.base, recall='echo "boom" >&2; exit 101')
    assert runner.cmd_run(run_args(live, arms="bilbo-keyword", bilbo=bilbo, run_id="r1")) == 0
    rows = rows_of(live.runs / "r1", "bilbo-keyword")
    assert all(r["error"] == "exit 101: boom" and r["exit"] == 101 and r["ranking"] == [] for r in rows)
    assert "| bilbo-keyword | " in (live.runs / "r1/report.md").read_text()


def test_nothing_matching_is_an_empty_ranking_with_no_error(live):
    bilbo = fake_bilbo(live.base, recall='echo "bilbo: no notes match" >&2; exit 1')
    runner.cmd_run(run_args(live, arms="bilbo-keyword", bilbo=bilbo, run_id="r1"))
    rows = [r for r in rows_of(live.runs / "r1", "bilbo-keyword") if r["stratum"] not in ("library", "no-answer")]
    assert rows and all(r["ranking"] == [] and r["error"] is None and r["metrics"]["success@5"] == 0.0 for r in rows)
    na = [r for r in rows_of(live.runs / "r1", "bilbo-keyword") if r["stratum"] == "no-answer"]
    assert all(r["metrics"] == {"empty": 1.0} for r in na)


def test_a_fallback_is_marked_and_counted(live):
    count = {"n": 0}
    arm = arms_pkg.get("bilbo-full")
    real = arm.rank

    def rank(item, ctx):
        count["n"] += 1
        if count["n"] == 3:
            live.fake.stop()
        return real(item, ctx)

    with pytest.MonkeyPatch.context() as mp:
        mp.setattr(arm, "rank", rank)
        runner.cmd_run(run_args(live, arms="bilbo-full", run_id="r1"))
    fell = [r["fallback"] for r in rows_of(live.runs / "r1", "bilbo-full")]
    assert not any(fell[:2]) and any(fell[3:])
    health = [l for l in (live.runs / "r1/report.md").read_text().splitlines() if l.startswith("| bilbo-full | ")][-1]
    assert health.split("|")[4].strip() == str(sum(fell))
    live.fake = None


def test_rank_all_ranks_queries_and_prompts_without_writing_records(live):
    got = runner.rank_all(live.ds, "dev", ["random", "ripgrep", "bm25-ref", "bilbo-full"], BILBO, None, None, live.fake.url)
    ids = {q["id"] for q in common.read_jsonl(live.ds / "queries.jsonl") if q["split"] == "dev"}
    ids |= {p["id"] for p in common.read_jsonl(live.ds / "digest/prompts.jsonl") if p["split"] == "dev"}
    assert set(got) == {"random", "ripgrep", "bm25-ref", "bilbo-full"}
    assert all(set(r) == ids for r in got.values())
    assert all(len(v) <= 100 for r in got.values() for v in r.values())
    assert not live.runs.exists()


def test_a_second_run_with_the_same_id_is_refused(live):
    runner.cmd_run(run_args(live, arms="ripgrep", run_id="r1"))
    with pytest.raises(common.Refused, match="exists"):
        runner.cmd_run(run_args(live, arms="ripgrep", run_id="r1"))


def test_unknown_arms_are_a_usage_error(live):
    with pytest.raises(common.UsageError, match="nope"):
        runner.cmd_run(run_args(live, arms="ripgrep,nope"))


def test_no_expanded_tmpdir_in_a_run_folder(live, monkeypatch):
    tmp = live.base / "home/tmp"
    tmp.mkdir(parents=True)
    monkeypatch.setenv("TMPDIR", str(tmp))
    runner.cmd_run(run_args(live, run_id="r1", keep_root=True))
    run = live.runs / "r1"
    text = "".join(p.read_text() for p in run.rglob("*") if p.is_file())
    assert str(Path.home()) not in text
    assert dataset._home_problems(run) == []
    for arm in ARMS:
        meta = json.loads((run / arm / "run.json").read_text())
        assert meta["root"] == "$TMPDIR/bilbo-evals-r1"
        assert all(p.startswith("$TMPDIR/bilbo-evals-r1/") for p in meta["folders"].values())


def test_a_run_records_its_dataset_folder_and_report_finds_it(live):
    elsewhere = live.base / "elsewhere/ds"
    elsewhere.parent.mkdir()
    live.ds.rename(elsewhere)
    live.ds = elsewhere
    assert runner.cmd_run(run_args(live, arms="ripgrep", run_id="r1")) == 0
    meta = json.loads((live.runs / "r1/ripgrep/run.json").read_text())
    assert meta["dataset"]["path"] == "elsewhere/ds"
    assert results.report(live.runs / "r1")


def test_a_dataset_outside_evals_is_refused_and_nothing_is_written(live, tmp_path_factory):
    outside = tmp_path_factory.mktemp("outside") / "ds"
    shutil.copytree(live.ds, outside)
    with pytest.raises(common.Refused, match="inside evals/"):
        runner.cmd_run(run_args(live, arms="ripgrep", run_id="r1", dataset=outside))
    assert not (live.runs / "r1").exists()


def test_rank_all_refuses_an_arm_error(live, monkeypatch):
    real = arms_pkg.get("ripgrep")

    class Broken:
        def prepare(self, ctx):
            return real.prepare(ctx)

        def rank(self, item, ctx):
            if item["id"] == first:
                return arms_pkg.Result([], error="exit 101: boom")
            return real.rank(item, ctx)

    first = next(q["id"] for q in common.read_jsonl(live.ds / "queries.jsonl") if q["split"] == "dev")
    monkeypatch.setattr(arms_pkg, "get", lambda name: Broken())
    with pytest.raises(common.Refused, match=rf"ripgrep {first}: exit 101: boom"):
        runner.rank_all(live.ds, "dev", ["ripgrep"], BILBO, None, None)


def test_rank_all_refuses_a_fallback(live, monkeypatch):
    real = arms_pkg.get("ripgrep")

    class Fell:
        def prepare(self, ctx):
            return real.prepare(ctx)

        def rank(self, item, ctx):
            r = real.rank(item, ctx)
            r.fallback = True
            return r

    monkeypatch.setattr(arms_pkg, "get", lambda name: Fell())
    with pytest.raises(common.Refused, match="ripgrep .*: fallback"):
        runner.rank_all(live.ds, "dev", ["ripgrep"], BILBO, None, None)
