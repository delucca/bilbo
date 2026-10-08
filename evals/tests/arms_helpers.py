"""Contexts for the arm tests: the fixture dataset, a real sandbox with its store, the fake embedder."""

from __future__ import annotations

from bilbo_evals import dataset, embedder, sandbox
from bilbo_evals.arms import Context


def query(ds, qid):
    return next(q for q in ds.queries if q["id"] == qid)


def note_id(ds, file):
    return next(n.id for n in ds.notes.values() if n.file == file)


def make_context(fixture_dir, monkeypatch, tmp_path, bilbo=None, fake=None, split="dev") -> Context:
    monkeypatch.setenv("TMPDIR", str(tmp_path))
    ds = dataset.load(fixture_dir)
    sb = sandbox.create("arms-test")
    mapping = dataset.materialize(ds, sb.store)
    server = embedder.Server.attach(fake.url, version="fake") if fake is not None else None
    return Context(ds=ds, sb=sb, bilbo=bilbo, server=server, path_to_id=mapping, split=split)
