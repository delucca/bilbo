"""generate profile: only rounded numbers leave a store, and the store is not changed."""

from __future__ import annotations

import json
import re

import pytest
from gen_e_helpers import make_ds

from bilbo_evals import cli, common
from bilbo_evals.common import Refused
from bilbo_evals.generate import profile

SECRETS = ["Zyxwvut-Quuxgrault", "/Users/nobody/private-path", "01JZZZZZZZZZZZZZZZZZZZZZZZ", "2031-02-03T04:05-03:00", "amaranthine"]


def make_store(root):
    notes = root / "notes"
    notes.mkdir(parents=True)
    kinds = ["decision", "gotcha", "plan", "decision", "research", "gotcha", "decision", "report"]
    for i, kind in enumerate(kinds):
        pt = i % 2 == 0
        body = ("Isso não é para você, mas com uma decisão que foi tomada em os dias." if pt
                else "This is the note and that is the plan for it in the field.")
        extra = "\n```sh\nls\n```\n" if i % 3 == 0 else ""
        link = "\nVeja [[amaranthine-topic]].\n" if i % 4 == 0 else ""
        src = f'sources:\n  - "code: {SECRETS[1]}"\n' if i % 2 == 0 else ""
        (notes / f"{kind}-{SECRETS[4]}-{i}.md").write_text(
            f"---\nid: {SECRETS[2]}\ncreated: {SECRETS[3]}\n{src}---\n\n# {SECRETS[0]} {i}\n\n{body}\n## More\n{extra}{link}",
            encoding="utf-8")
    return root


def snapshot(root):
    return {str(p): (p.read_bytes(), p.stat().st_mtime_ns) for p in sorted(root.rglob("*")) if p.is_file()}


def test_only_rounded_numbers_leave_the_store(tmp_path):
    store = make_store(tmp_path / "store")
    ds = make_ds(tmp_path)
    before = snapshot(store)
    assert cli.main(["generate", "profile", "--store", str(store), "--dataset", str(ds)]) == 0
    assert snapshot(store) == before
    text = (ds / "world/profile.json").read_text()
    for secret in SECRETS:
        assert secret not in text
    assert not re.search(r"[A-Za-z]{12,}", text.replace("length_chars", "").replace("code_blocks", "").replace("wiki_links", ""))
    p = json.loads(text)
    assert p["notes"] == 0 or p["notes"] % 50 == 0
    assert all(abs(v * 20 - round(v * 20)) < 1e-9 for v in p["kinds"].values()), p["kinds"]
    assert all(v % 100 == 0 for v in p["length_chars"].values())
    assert all(isinstance(v, int) for v in p["headings"].values())
    assert set(p["shares"]) == {"code_blocks", "sources", "portuguese", "wiki_links"}
    assert p["shares"]["portuguese"] == 0.5 and p["shares"]["sources"] == 0.5
    assert p["kinds"]["decision"] == 0.4 and p["kinds"]["gotcha"] == 0.25


def test_a_folder_without_notes_is_refused(tmp_path, capsys):
    ds = make_ds(tmp_path)
    (tmp_path / "plain").mkdir()
    assert cli.main(["generate", "profile", "--store", str(tmp_path / "plain"), "--dataset", str(ds)]) == 1
    assert "not a bilbo store" in capsys.readouterr().err
    assert not (ds / "world/profile.json").exists()


def test_a_frozen_dataset_is_refused(tmp_path, capsys):
    ds = make_ds(tmp_path)
    (ds / "FROZEN").write_text("x\n")
    store = make_store(tmp_path / "store")
    assert cli.main(["generate", "profile", "--store", str(store), "--dataset", str(ds)]) == 1
    assert "frozen" in capsys.readouterr().err
    assert not (ds / "world").exists()


def test_an_empty_notes_folder_is_refused(tmp_path):
    (tmp_path / "s/notes").mkdir(parents=True)
    with pytest.raises(Refused, match="no notes"):
        profile.compute(tmp_path / "s")


def test_quantiles_interpolate():
    assert profile._quantile([1, 2, 3, 4, 5], 0.5) == 3
    assert profile._quantile([10, 20], 0.25) == 12.5
    assert common is not None
