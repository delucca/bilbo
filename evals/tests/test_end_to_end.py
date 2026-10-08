"""The whole L1 flow on the fixture with the fake embedder, offline, and the generation rows through the real build_qrels."""

from __future__ import annotations

import socket
from argparse import Namespace
from pathlib import Path

import pytest

from bilbo_evals import cli, common, dataset, embedder, llm, results, sandbox
from bilbo_evals.arms import ARMS
from bilbo_evals.generate import prompts, queries


@pytest.fixture
def offline(monkeypatch):
    """Refuse any connect that is not loopback: the flow needs no network."""
    real = socket.socket.connect

    def guard(self, address, *a, **k):
        host = address[0] if isinstance(address, tuple) else None
        if self.family in (socket.AF_INET, socket.AF_INET6) and host not in ("127.0.0.1", "::1", "localhost"):
            raise AssertionError(f"network access to {address}")
        return real(self, address, *a, **k)

    monkeypatch.setattr(socket.socket, "connect", guard)


def test_the_whole_flow_runs_offline_on_the_fixture(fixture_copy, fake_embedder, bilbo_bin, tmp_path, monkeypatch, capsys, offline):
    ds = fixture_copy
    base = tmp_path / "evals"
    runs = base / "runs"
    monkeypatch.setattr(common, "EVALS_ROOT", base)
    monkeypatch.setattr(common, "RUNS_DIR", runs)
    monkeypatch.setattr(common, "TEST_RUNS_LOG", base / "test-runs.jsonl")
    monkeypatch.setenv("TMPDIR", str(tmp_path))
    # the dataset sits inside EVALS_ROOT, as a run requires
    (base / "datasets/fixture").mkdir(parents=True)
    ds = ds.rename(base / "datasets/fixture/v1")
    flags = ["--dataset", str(ds), "--bilbo", str(bilbo_bin), "--embedder-url", fake_embedder.url]

    tree = (ds / "FROZEN").read_text().strip()
    assert cli.main(["dataset", "verify", str(ds)]) == 0
    assert capsys.readouterr().out.strip() == tree

    assert cli.main(["l1", "run", "--split", "dev", "--arms", "all", "--run-id", "e2e", *flags]) == 0
    run = runs / "e2e"
    assert sorted(p.name for p in run.iterdir() if p.is_dir()) == sorted(ARMS)

    assert cli.main(["l1", "digest", "--split", "dev", "--sweep", "--run", str(run), *flags]) == 0
    assert (run / "digest").is_dir()

    capsys.readouterr()
    assert cli.main(["report", str(run)]) == 0
    assert capsys.readouterr().out.startswith(results.CLAIM)
    assert (run / "report.md").is_file()

    assert cli.main(["compare", str(run), str(run)]) == 0
    assert "L1 comparison" in capsys.readouterr().out


def test_parity_is_ok_on_the_fixture(fixture_copy, fake_embedder, bilbo_bin, tmp_path, monkeypatch, offline):
    monkeypatch.setenv("TMPDIR", str(tmp_path))
    ds = dataset.load(fixture_copy)
    sb = sandbox.create("e2e-parity")
    dataset.materialize(ds, sb.store)
    server = embedder.Server.attach(fake_embedder.url, version="fake")
    record = embedder.index_and_check(sb, bilbo_bin, server, ds)
    assert record["parity"] == "ok"


@pytest.fixture
def generation(tmp_path, fake_llm, monkeypatch):
    """A small two-project dataset with the fake codex; build_qrels is the real one."""
    from test_queries import make_world

    monkeypatch.setattr(llm, "preflight", lambda *a, **k: None)
    monkeypatch.delenv("CODEX_HOME", raising=False)
    (Path.home() / ".codex").mkdir()
    (Path.home() / ".codex/auth.json").write_text("{}", encoding="utf-8")
    return make_world(tmp_path / "gen"), fake_llm


def test_generated_query_and_prompt_rows_go_through_the_real_build_qrels(generation):
    from test_queries import intents

    ds, fake = generation
    _, (built, _) = intents(ds)
    fake.set_script([{"match": f"Item: {it.id}\n", "output": {"query": f"query for {it.id}", "lang": it.lang}} for it in built])
    assert queries.cmd(Namespace(dataset=str(ds), split="dev")) == 0

    batches, _ = prompts.plan_batches(queries.load_world(ds), "dev", {"positive": 12, "noise": 10, "off-topic": 10, "near-miss": 10}, 7)
    fake.set_script([{"match": f"Batch: {b.key}\n", "output": {"prompts": [f"{b.label} {b.key} {i}" for i in range(b.n)]}} for b in batches])
    assert prompts.cmd(Namespace(dataset=str(ds), split="dev")) == 0

    rows = common.read_jsonl(ds / "queries.jsonl")
    qrels = [line.split() for line in (ds / "qrels/dev.txt").read_text().splitlines()]
    library = [line.split() for line in (ds / "qrels/library-dev.txt").read_text().splitlines()]
    answered = {r["id"] for r in rows if r["split"] == "dev" and r["stratum"] not in ("no-answer", "library")}
    assert {qid for qid, _, _, rel in qrels if rel == "1"} == answered
    assert {qid for qid, *_ in library} == {r["id"] for r in rows if r["stratum"] == "library"}
    assert not any(qid.startswith("q-alpha-na-") for qid, *_ in qrels)
    assert all(r["canary"] == common.CANARY for r in common.read_jsonl(ds / "digest/prompts.jsonl"))
