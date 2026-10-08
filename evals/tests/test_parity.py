"""`bilbo-evals parity`: bilbo and the port send the same inputs, and the command says which differ when they do not."""

from __future__ import annotations

import argparse

import pytest

from bilbo_evals import embedder, passages
from test_embedder import write_notes

LONG = "\n\n".join(f"paragraph {i}: " + "w" * 880 for i in range(10))
NOTES = {
    "decision-short.md": "# Short\n\n## A\n\nfew words\n",
    "research-long.md": f"# Long\n\n## Section\n\n{LONG}\n",
}


@pytest.fixture
def dataset(tmp_path):
    ds = tmp_path / "ds"
    write_notes(ds / "store", NOTES)
    return ds


def args(ds, bilbo_bin, fake_embedder):
    return argparse.Namespace(dataset=ds, bilbo=bilbo_bin, model=None, llama_server="llama-server", embedder_url=fake_embedder.url)


def test_parity_holds(dataset, bilbo_bin, fake_embedder, capsys, monkeypatch, tmp_path):
    monkeypatch.setenv("TMPDIR", str(tmp_path))
    assert embedder.cmd_parity(args(dataset, bilbo_bin, fake_embedder)) == 0
    n = len(set(passages.inputs(dataset / "store")))
    assert n > 2
    assert capsys.readouterr().out.strip() == f"parity ok: {n} inputs compared, no difference"


def test_a_port_that_cuts_elsewhere_fails_and_names_the_inputs(dataset, bilbo_bin, fake_embedder, capsys, monkeypatch, tmp_path):
    monkeypatch.setenv("TMPDIR", str(tmp_path))
    monkeypatch.setattr(passages, "PART_BYTES", 3000)
    assert embedder.cmd_parity(args(dataset, bilbo_bin, fake_embedder)) == 1
    lines = [l for l in capsys.readouterr().err.splitlines() if l.startswith("bilbo-evals: ")]
    sides = {l.split(": ")[1] for l in lines if "-only" in l}
    assert sides == {"harness-only", "bilbo-only"}
    shown = [l.split(": ", 2)[2] for l in lines if "-only" in l]
    assert shown and all(len(s) <= 80 for s in shown)
    assert any(s.startswith("Long > Section\\nparagraph") for s in shown)
    assert lines[-1].startswith("bilbo-evals: parity failed: ")


def test_parity_needs_a_store(tmp_path, bilbo_bin, fake_embedder):
    from bilbo_evals.common import Refused

    with pytest.raises(Refused, match="no store"):
        embedder.cmd_parity(args(tmp_path / "empty", bilbo_bin, fake_embedder))


def test_parity_through_the_cli(dataset, bilbo_bin, fake_embedder, capsys, monkeypatch, tmp_path):
    from bilbo_evals import cli

    monkeypatch.setenv("TMPDIR", str(tmp_path))
    code = cli.main(["parity", "--bilbo", str(bilbo_bin), "--dataset", str(dataset), "--embedder-url", fake_embedder.url])
    assert code == 0 and "parity ok" in capsys.readouterr().out
