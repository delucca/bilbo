"""generate library: public-domain pages staged from files (never the network), kept lines from the fake claude, landed by bilbo."""

from __future__ import annotations

import json
import subprocess

import pytest
from gen_e_helpers import make_ds, rows, use_cache

from bilbo_evals import cli, common
from bilbo_evals.generate import library

PROBE = {"match": "preflight probe (claude)", "output": {"user_instructions_first_heading": "NONE"}}
EVIDENCE = "https://www.sqlite.org/copyright.html"


def page(title: str, body: str) -> str:
    nav = "[Home](/index.html) | [About](/about.html) | [Docs](/docs.html)\n\n"
    return f"{nav}# {title}\n\nIntro of {title}.\n\n## Overview\n\n{body}\n\n## Details\n\nMore about {title} and its limits.\n\n---\n\nThis page was generated on 2026-01-01.\n"


@pytest.fixture
def pages(tmp_path, monkeypatch):
    use_cache(tmp_path, monkeypatch)
    src = tmp_path / "pages"
    src.mkdir()
    spec = {"wal": "Write-Ahead Logging", "busy": "Busy Timeout", "locks": "File Locking", "fts": "Full Text Search"}
    urls = {}
    for key, title in spec.items():
        f = src / f"{key}.md"
        f.write_text(page(title, f"{title} keeps readers and writers apart."), encoding="utf-8")
        urls[key] = f"https://www.sqlite.org/{key}.html"
    monkeypatch.setattr(library, "STAGE_FROM_FILES", {urls[k]: str(src / f"{k}.md") for k in spec})
    listing = tmp_path / "library_pages.txt"
    lines = ["# test pages", *(f"{urls[k]} public-domain {EVIDENCE}" for k in ("wal", "busy", "locks")),
             "https://www.example.org/unlicensed.html",
             "https://www.example.org/cc.html cc-by https://example.org/licence", f"{urls['fts']} public-domain {EVIDENCE}"]
    listing.write_text("\n".join(lines) + "\n", encoding="utf-8")
    monkeypatch.setattr(library, "PAGES_FILE", listing)
    return urls


def keep(url_key, name, **kw):
    out = {"keep": "3-11", "name": name, "guide_entry": f"Explains {name} in the SQLite documentation. Read it when a question is about {name}."}
    out.update(kw)
    return {"match": f"https://www.sqlite.org/{url_key}.html", "output": out}


def rules(extra=()):
    return [PROBE, *extra, keep("wal", "wal"), keep("busy", "busy-timeout"), keep("locks", "locking"), keep("fts", "fts")]


def run(ds, bilbo):
    return cli.main(["generate", "library", "--dataset", str(ds), "--bilbo", str(bilbo)])


def test_pages_are_staged_landed_and_copied_in(tmp_path, pages, fake_llm, bilbo_bin, capsys):
    ds = make_ds(tmp_path)
    cfg = (ds / "generation/config.toml").read_text().replace("pages = 2", "pages = 3")
    (ds / "generation/config.toml").write_text(cfg)
    fake_llm.set_script(rules())
    assert run(ds, bilbo_bin) == 0
    err = capsys.readouterr().err
    assert "unlicensed.html: no recorded public-domain licence" in err and "cc.html: no recorded public-domain licence" in err
    lib = ds / "store/library/sqlite"
    assert len(list(lib.glob("*.md"))) == 4 and (lib / "guide.md").is_file()
    assert not (ds / "store/library/.lock").exists()
    name = next(p.stem for p in lib.glob("*.md") if p.stem != "guide")
    source = (lib / f"{name}.md").read_text()
    assert source.startswith("---\nid: ") and 'origin: "url: https://www.sqlite.org/' in source and "digest: sha256:" in source
    assert "Home](/index.html)" not in source and "generated on" not in source
    guide = (lib / "guide.md").read_text()
    assert "TODO" not in guide and f"Explains {name} in the SQLite documentation" in guide and "public domain" in guide
    assert any((ds / "store/.bilbo/captures").rglob("capture.md"))
    env = {"BILBO_HOME": str(ds / "store"), "HOME": str(tmp_path / "home"), "PATH": "/usr/bin:/bin",
           "XDG_CONFIG_HOME": str(tmp_path / "cfg"), "XDG_STATE_HOME": str(tmp_path / "state"), "XDG_CACHE_HOME": str(tmp_path / "cache")}
    check = subprocess.run([str(bilbo_bin), "check"], env=env, capture_output=True, text=True)
    assert (check.returncode, check.stdout.strip()) == (0, ""), check.stdout


def test_library_records_origin_licence_split_and_corpus(tmp_path, pages, fake_llm, bilbo_bin):
    ds = make_ds(tmp_path)
    fake_llm.set_script(rules())
    assert run(ds, bilbo_bin) == 0
    lib = json.loads((ds / "world/library.json").read_text())
    assert len(lib) == 2 and [r["ref"] for r in lib] == sorted(r["ref"] for r in lib)
    for r in lib:
        assert r["url"].startswith("https://www.sqlite.org/") and r["licence"] == "public-domain"
        assert r["licence_evidence"] == EVIDENCE and r["keep"] == "3-11" and len(r["stage_sha256"]) == 64 and r["guide_entry"]
    splits = json.loads((ds / "world/splits.json").read_text())["library"]
    assert sorted(splits["dev"] + splits["test"]) == [r["ref"] for r in lib] and splits["dev"] and splits["test"]
    corpus = {r["_id"]: r for r in common.read_jsonl(ds / "corpus.jsonl")}
    assert set(corpus) == {r["ref"] for r in lib}
    row = corpus[lib[0]["ref"]]
    assert row["metadata"]["kind"] == "source" and row["metadata"]["source"] == {"url": lib[0]["url"], "licence": "public-domain"}


def test_nothing_in_the_dataset_names_the_users_home_or_the_sandbox(tmp_path, pages, fake_llm, bilbo_bin):
    import os

    ds = make_ds(tmp_path)
    fake_llm.set_script(rules())
    assert run(ds, bilbo_bin) == 0
    home = os.environ["HOME"]
    for p in ds.rglob("*"):
        if p.is_file():
            text = p.read_text(errors="replace")
            assert home not in text and "bilbo-evals-library" not in text, p


def test_the_renderer_sees_the_line_listing_and_not_a_note(tmp_path, pages, fake_llm, bilbo_bin):
    ds = make_ds(tmp_path)
    fake_llm.set_script(rules())
    assert run(ds, bilbo_bin) == 0
    prompt = next(c["prompt"] for c in fake_llm.calls() if "wal.html" in c["prompt"])
    assert "Write-Ahead Logging" in prompt and "## Overview" in prompt and "1\t" in prompt
    assert "converter suggests keeping lines" in prompt


def test_an_out_of_range_keep_is_rerolled(tmp_path, pages, fake_llm, bilbo_bin):
    ds = make_ds(tmp_path)
    fake_llm.set_script(rules([{**keep("wal", "wal", keep="3-999"), "times": 1}]))
    assert run(ds, bilbo_bin) == 0
    assert (ds / "generation/outputs/library/wal.2.json").is_file()
    assert (ds / "store/library/sqlite/wal.md").is_file()


def first_page(tmp_path):
    import random

    cfg = library.llm.load_config(tmp_path / "datasets/notes-synth/ds")
    order = library.pages()
    random.Random(f"{cfg.seed}:library-pages").shuffle(order)
    return order[0][0]


def test_a_page_that_never_gets_a_usable_range_is_skipped_for_the_next_one(tmp_path, pages, fake_llm, bilbo_bin, capsys):
    ds = make_ds(tmp_path)
    bad = first_page(tmp_path)
    key = bad.rsplit("/", 1)[1].removesuffix(".html")
    fake_llm.set_script(rules([keep(key, key, keep="x")]))
    assert run(ds, bilbo_bin) == 0
    assert "no usable keep range" in capsys.readouterr().err
    refs = [r["url"] for r in json.loads((ds / "world/library.json").read_text())]
    assert len(refs) == 2 and bad not in refs


def test_a_page_that_cannot_be_staged_is_skipped(tmp_path, pages, fake_llm, bilbo_bin, monkeypatch, capsys):
    ds = make_ds(tmp_path)
    monkeypatch.setitem(library.STAGE_FROM_FILES, pages["wal"], str(tmp_path / "missing.md"))
    fake_llm.set_script(rules())
    assert run(ds, bilbo_bin) == 0
    assert f"skipped {pages['wal']}" in capsys.readouterr().err
    assert not (ds / "store/library/sqlite/wal.md").exists()


def test_fewer_sources_than_asked_is_refused_but_kept(tmp_path, pages, fake_llm, bilbo_bin, capsys):
    ds = make_ds(tmp_path)
    cfg = (ds / "generation/config.toml").read_text().replace("pages = 2", "pages = 9")
    (ds / "generation/config.toml").write_text(cfg)
    fake_llm.set_script(rules())
    assert run(ds, bilbo_bin) == 1
    assert "only 4 of 9 sources landed" in capsys.readouterr().err
    assert (ds / "world/library.json").is_file()


def test_the_budget_stops_before_landing(tmp_path, pages, fake_llm, bilbo_bin, capsys):
    ds = make_ds(tmp_path, max_calls=2, concurrency=1)
    fake_llm.set_script(rules())
    assert run(ds, bilbo_bin) == 1
    err = capsys.readouterr().err
    assert "budget of 2" in err and "sources left" in err
    assert len(json.loads((ds / "world/library.json").read_text())) == 1
    assert len(rows(ds)) == 2


def test_resuming_makes_no_new_calls(tmp_path, pages, fake_llm, bilbo_bin):
    ds = make_ds(tmp_path)
    fake_llm.set_script(rules())
    assert run(ds, bilbo_bin) == 0
    n = len(fake_llm.calls())
    first = (ds / "world/library.json").read_text()
    assert run(ds, bilbo_bin) == 0
    assert len(fake_llm.calls()) == n
    assert [r["ref"] for r in json.loads((ds / "world/library.json").read_text())] == [r["ref"] for r in json.loads(first)]


def test_a_missing_binary_or_cli_is_refused(tmp_path, pages, fake_llm, capsys):
    ds = make_ds(tmp_path)
    assert cli.main(["generate", "library", "--dataset", str(ds)]) == 1
    assert "needs --bilbo" in capsys.readouterr().err


def test_the_pages_file_has_only_public_domain_sqlite_pages():
    text = (library.HERE / "library_pages.txt").read_text()
    lines = [x.split() for x in text.splitlines() if x.strip() and not x.startswith("#")]
    assert len(lines) >= 70 and len({x[0] for x in lines}) == len(lines)
    assert all(x[0].startswith("https://www.sqlite.org/") and x[1:] == ["public-domain", EVIDENCE] for x in lines)


def test_page_slugs_and_stage_output_parse():
    assert library.slug("https://www.sqlite.org/c3ref/busy_timeout.html") == "c3ref-busy_timeout"
    info = library.parse_stage("stage: 01X\ncapture: /a b/capture.md\nlines: 11\ntokens: 3\ntitle: T: x\nkeep: 2-11\n\n1\t# T\n5\t## Overview\n")
    assert info["stage"] == "01X" and info["capture"] == "/a b/capture.md" and info["title"] == "T: x"
    assert info["headings"] == [(1, "# T"), (5, "## Overview")]
