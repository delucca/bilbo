"""Query intents per stratum, the blind boundary, resumption and rewrites."""

from __future__ import annotations

import json
from argparse import Namespace
from pathlib import Path
from types import SimpleNamespace

import pytest

from bilbo_evals import common
from bilbo_evals.common import Refused
from bilbo_evals.generate import queries as q

CONFIG = """seed = 7
max_calls = 200
concurrency = 2
[renderer]
cli = "claude"
model = "claude-sonnet-5-5"
[query_model]
cli = "codex"
model = "gpt-6.1-sol"
reasoning_effort = "low"
[world]
projects = 2
dev_projects = 1
notes = 20
filler_share = 0.3
max_test_note_queries = 50
[library]
corpus = "demo"
pages = 1
[strata.dev]
known-item = 2
paraphrase = 2
pt-en = 2
alias = 1
supersession = 1
multi-hop = 1
kind-filter = 1
library = 2
no-answer = 2
[prompts.dev]
positive = 12
noise = 10
off-topic = 10
near-miss = 10
"""

LIB = """---
id: demo/wal
---
# Write-ahead log

Intro line.

## Checkpointing

""" + "A checkpoint copies pages from the log back into the database file. " * 8 + """

## Readers

""" + "Readers never block the writer while the log grows between checkpoints. " * 8 + "\n"


def _write(p: Path, text: str) -> None:
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(text, encoding="utf-8")


def make_world(ds: Path) -> Path:
    """Two projects (alpha dev, beta test) with every structure the strata need; each note body holds BODYMARK-<id>."""
    facts, notes = [], []

    def fact(n, project, comp, kind, statement, lang="en", **extra):
        fid, nid = f"f-{project}-{n:03d}", f"N{project.upper()}{n:03d}"
        facts.append({
            "id": fid, "project": project, "component": comp, "family": f"fam-{project}-{n:03d}", "kind": kind,
            "statement": statement, "verbatim": [], "lang": lang, "valid_from": "2025-06-02T10:14-03:00",
            "supersedes": None, "superseded_by": None, "joins": [], "kind_pair": None, "bridge": None,
            "note_id": nid, "status": "planted", **extra,
        })
        notes.append({"id": nid, "file": f"{kind}-{project}-{n:03d}.md", "kind": kind, "topic": f"{project}-{n:03d}",
                      "project": project, "lang": lang, "created": "2025-06-02T10:14-03:00", "facts": [fid],
                      "filler": False, "noise": [], "near_duplicate_of": None, "render_attempts": 1, "status": "kept"})
        extra_body = " The old name is Lantern." if extra.get("bridge") else ""
        _write(ds / "store/notes" / notes[-1]["file"],
               f"---\nid: {nid}\n---\n# Title of {fid}\n\nBODYMARK-{nid}: {statement}{extra_body}\n")
        return fid

    for project in ("alpha", "beta"):
        for i in range(1, 7):
            fact(i, project, "edge-cache", "decision", f"Plain fact {i} of {project}.", "pt" if i in (2, 4) else "en")
        old = fact(10, project, "edge-cache", "decision", f"Retry limit of {project} is 3.")
        new = fact(11, project, "edge-cache", "decision", f"Retry limit of {project} is 5, replacing the old one.", supersedes=old)
        next(f for f in facts if f["id"] == old)["superseded_by"] = new
        a = fact(20, project, "ingest", "design", f"Ingest of {project} reads from queue A.")
        b = fact(21, project, "ingest", "design", f"Ingest of {project} writes to table B.")
        for x, y in ((a, b), (b, a)):
            next(f for f in facts if f["id"] == x)["joins"] = [y]
        g = fact(30, project, "billing", "gotcha", f"Billing of {project} double-charges on retry.")
        d = fact(31, project, "billing", "decision", f"Billing of {project} dedupes by invoice id.")
        for x, y in ((g, d), (d, g)):
            next(f for f in facts if f["id"] == x)["kind_pair"] = y
        fact(40, project, "edge-cache", "reference", f"Edge cache of {project} listens on port 5000.")
        fact(41, project, "edge-cache", "reference", f"Lantern, the old name of edge-cache, was renamed in {project}.", bridge={"alias": "Lantern", "canonical": "edge-cache"})
        fact(42, project, "edge-cache", "reference", f"Lantern still appears in the {project} dashboards.")

    projects = [{
        "slug": s, "name": s.capitalize(), "summary": f"The {s} service", "technologies": ["Go"],
        "components": [{"slug": c, "name": c, "aliases": []} for c in ("edge-cache", "ingest", "billing")],
        "candidate_facts": [],
    } for s in ("alpha", "beta")]
    aliases = [{"project": s, "component": "edge-cache", "canonical": "edge-cache", "alias": "Lantern",
                "type": "old-name", "bridge_notes": [f"N{s.upper()}041"]} for s in ("alpha", "beta")]
    _write(ds / "world/world.json", json.dumps({"seed": 7, "projects": projects}))
    _write(ds / "world/aliases.json", json.dumps(aliases))
    _write(ds / "world/splits.json", json.dumps({
        "seed": 7, "dev": ["alpha"], "test": ["beta"], "library": {"dev": ["demo/wal"], "test": []},
        "none_prompts": {"dev": [], "test": []}}))
    common.write_jsonl(ds / "world/facts.jsonl", facts)
    common.write_jsonl(ds / "world/notes.jsonl", notes)
    _write(ds / "store/library/demo/wal.md", LIB)
    _write(ds / "generation/config.toml", CONFIG)
    return ds


@pytest.fixture
def ds(tmp_path) -> Path:
    return make_world(tmp_path / "ds")


COUNTS = {"known-item": 2, "paraphrase": 2, "pt-en": 2, "alias": 1, "supersession": 1, "multi-hop": 1,
          "kind-filter": 1, "library": 2, "no-answer": 2}


def intents(ds: Path, split="dev"):
    w = q.load_world(ds)
    return w, q.build_intents(w, split, COUNTS, 7)


def test_every_stratum_is_built_with_gold_from_one_split(ds):
    w, (built, short) = intents(ds)
    assert short == {}
    by = {}
    for it in built:
        by.setdefault(it.stratum, []).append(it)
    assert {s: len(v) for s, v in by.items()} == COUNTS
    assert all(it.project in (None, "alpha") for it in built)
    assert all(g.startswith("NALPHA") for it in built if it.stratum != "library" for g in it.gold + it.decoys)
    assert len({it.id for it in built}) == len(built)
    assert {it.id for it in by["library"]} == {"q-lib-001", "q-lib-002"}
    assert all(it.id.startswith("q-alpha-na-") and it.gold == [] for it in by["no-answer"])


def test_supersession_multihop_kind_and_pten_shapes(ds):
    w, (built, _) = intents(ds)
    by = {it.stratum: it for it in reversed(built)}
    sup = by["supersession"]
    assert sup.gold == ["NALPHA011"] and sup.decoys == ["NALPHA010"]
    hop = by["multi-hop"]
    assert hop.evidence_sets == [["NALPHA020", "NALPHA021"]] and set(hop.gold) == {"NALPHA020", "NALPHA021"}
    kf = by["kind-filter"]
    assert kf.kind in ("gotcha", "decision") and len(kf.decoys) == 1
    assert w.notes[kf.gold[0]]["kind"] == kf.kind and w.notes[kf.decoys[0]]["kind"] != kf.kind
    for it in (i for i in built if i.stratum == "pt-en"):
        assert it.lang != w.notes[it.gold[0]]["lang"]
    lib = [i for i in built if i.stratum == "library"]
    assert all(i.gold == ["demo/wal"] and i.family == "fam-lib-wal" and i.project is None for i in lib)
    assert {i.gold_heading for i in lib} == {"Write-ahead log > Checkpointing", "Write-ahead log > Readers"}


def test_a_fact_serves_one_query_per_stratum_not_one_per_split(ds):
    w = q.load_world(ds)
    built, short = q.build_intents(w, "dev", {"known-item": 12, "paraphrase": 12}, 7)
    assert short == {}
    by = {s: [f for i in built if i.stratum == s for f in i.fact_ids] for s in ("known-item", "paraphrase")}
    assert all(len(v) == len(set(v)) == 12 for v in by.values())
    assert set(by["known-item"]) & set(by["paraphrase"])


def test_alias_query_never_uses_an_alias_the_gold_note_holds(ds):
    w, (built, _) = intents(ds)
    alias = next(i for i in built if i.stratum == "alias")
    assert alias.slots["alias"] == "Lantern"
    assert not w.note_has_word(alias.gold[0], "Lantern")
    assert alias.gold[0] not in {"NALPHA041", "NALPHA042"}


def test_blind_prompts_never_hold_a_note(ds):
    """Paraphrase, pt-en and alias are worded from the fact and the alias table only."""
    w, (built, _) = intents(ds)
    templates = q.load_sections("queries.md")
    seen = set()
    for it in built:
        prompt = q.build_prompt(it, templates)
        seen.add(it.stratum)
        if it.stratum == "library":
            continue
        assert "BODYMARK" not in prompt, it.stratum
        if it.stratum in q.BLIND:
            assert f"Title of" not in prompt, it.stratum
            assert it.gold[0] not in prompt
        assert f"Item: {it.id}" in prompt
    assert q.BLIND <= seen
    known = next(i for i in built if i.stratum == "known-item")
    assert "Title of" in q.build_prompt(known, templates)


def test_the_avoid_block_names_the_rejected_tokens(ds):
    _, (built, _) = intents(ds)
    templates = q.load_sections("queries.md")
    text = q.build_prompt(built[0], templates, ["checkpoint", "fsync"])
    assert "checkpoint, fsync" in text and "checkpoint" not in q.build_prompt(built[0], templates)


def test_intents_are_deterministic_and_seeded(ds):
    w = q.load_world(ds)
    a, _ = q.build_intents(w, "dev", COUNTS, 7)
    b, _ = q.build_intents(w, "dev", COUNTS, 7)
    c, _ = q.build_intents(w, "dev", COUNTS, 8)
    assert [(i.id, i.gold, i.slots) for i in a] == [(i.id, i.gold, i.slots) for i in b]
    assert [i.gold for i in a] != [i.gold for i in c]


def test_a_stratum_the_facts_cannot_fill_reports_its_shortfall(ds):
    w = q.load_world(ds)
    _, short = q.build_intents(w, "dev", {**COUNTS, "supersession": 5}, 7)
    assert short == {"supersession": 4}


def test_test_counts_need_a_preregistration(ds):
    cfg = SimpleNamespace(strata={"dev": COUNTS})
    with pytest.raises(Refused, match="size the test split with bilbo-evals power first"):
        q.split_counts(ds, cfg, "test")
    common.write_json(ds / "preregistration.json", {"per_stratum": {"known-item": 3, "library": 20, "no-answer": 20}})
    assert q.split_counts(ds, cfg, "test") == {"known-item": 3, "library": 20, "no-answer": 20}
    assert q.split_counts(ds, cfg, "dev") == COUNTS


def test_next_step_walks_attempts_leakage_and_the_cap():
    it = q.Intent("paraphrase", "dev", "alpha", "fam", "en", ["N1"], {}, id="q-alpha-001")
    ok, bad = {"query": "how", "lang": "en"}, {"query": "ainda", "lang": "pt"}
    assert q.next_step(it, {}, {}) == ("call", 1)
    assert q.next_step(it, {1: ok}, {}) == ("row", 1)
    assert q.next_step(it, {1: ok}, {1: "leakage"}) == ("call", 2)
    assert q.next_step(it, {1: ok, 2: ok}, {1: "leakage"}) == ("row", 2)
    assert q.next_step(it, {1: bad}, {}) == ("call", 2)
    assert q.next_step(it, {1: ok, 2: ok, 3: ok}, {1: "leakage", 2: "leakage", 3: "alias"}) == ("drop", "alias-exhausted")
    assert q.next_step(it, {1: bad, 2: bad, 3: bad}, {}) == ("drop", "invalid-output")


# ---- the step, against the fake codex ----------------------------------------------------------------------


@pytest.fixture
def step(ds, fake_llm, monkeypatch):
    from bilbo_evals import dataset, llm

    monkeypatch.setattr(llm, "preflight", lambda *a, **k: None)
    monkeypatch.delenv("CODEX_HOME", raising=False)
    (Path.home() / ".codex").mkdir()
    (Path.home() / ".codex/auth.json").write_text("{}", encoding="utf-8")
    monkeypatch.setattr(dataset, "build_qrels", lambda d: None)
    w, (built, _) = intents(ds)
    rules = [{"match": f"Item: {it.id}\n", "output": {"query": f"query for {it.id}", "lang": it.lang}} for it in built]
    fake_llm.set_script(rules)
    return Namespace(ds=ds, fake=fake_llm, built=built, run=lambda: q.cmd(Namespace(dataset=str(ds), split="dev")))


def test_cmd_writes_rows_and_the_query_model_never_sees_a_blind_note(step):
    assert step.run() == 0
    rows = common.read_jsonl(step.ds / "queries.jsonl")
    assert len(rows) == len(step.built)
    for r in rows:
        assert r["canary"] == common.CANARY and r["gen"]["call_id"] == f"queries/{r['id']}/1"
        assert r["gen"]["model"] == "gpt-6.1-sol" and len(r["gen"]["prompt_sha256"]) == 64
    log = {c["prompt"].split("Item: ")[1].split("\n")[0]: c["prompt"] for c in step.fake.calls() if "Item: q-" in c["prompt"]}
    blind = {r["id"] for r in rows if r["stratum"] in q.BLIND}
    assert blind and blind <= set(log)
    assert all("BODYMARK" not in p for p in log.values())
    assert next(r for r in rows if r["stratum"] == "supersession")["decoys"] == ["NALPHA010"]


def test_cmd_is_resumable_and_makes_no_second_call(step):
    step.run()
    calls = len(step.fake.calls())
    assert step.run() == 0
    assert len(step.fake.calls()) == calls


def test_cmd_regenerates_a_leaked_query_with_the_tokens_to_avoid(step):
    step.run()
    rows = common.read_jsonl(step.ds / "queries.jsonl")
    target = next(r for r in rows if r["stratum"] == "paraphrase")
    common.write_jsonl(step.ds / "queries.jsonl", [r for r in rows if r["id"] != target["id"]])
    common.append_jsonl(step.ds / "generation/drops.jsonl", {"item": target["id"], "reason": "leakage", "attempt": 1, "tokens": ["checkpoint", "fsync"]})
    before = len(step.fake.calls())
    step.fake.set_script([{"match": "checkpoint, fsync", "output": {"query": "second try", "lang": target["lang"]}}])
    assert step.run() == 0
    again = next(r for r in common.read_jsonl(step.ds / "queries.jsonl") if r["id"] == target["id"])
    assert again["text"] == "second try" and again["gen"]["call_id"] == f"queries/{target['id']}/2"
    new = step.fake.calls()[before:]
    assert len(new) == 1 and "BODYMARK" not in new[0]["prompt"]


def test_cmd_gives_up_after_three_attempts(step):
    step.run()
    rows = common.read_jsonl(step.ds / "queries.jsonl")
    target = next(r for r in rows if r["stratum"] == "alias")
    common.write_jsonl(step.ds / "queries.jsonl", [r for r in rows if r["id"] != target["id"]])
    for a in (1, 2, 3):
        common.append_jsonl(step.ds / "generation/drops.jsonl", {"item": target["id"], "reason": "leakage", "attempt": a, "tokens": ["x"]})
    for a in (2, 3):
        common.write_json(step.ds / f"generation/outputs/queries/{target['id']}.{a}.json", {"query": "q", "lang": target["lang"]})
    assert step.run() == 0
    drops = common.read_jsonl(step.ds / "generation/drops.jsonl")
    assert drops[-1] == {"item": target["id"], "reason": "leakage-exhausted", "stratum": "alias"}
    assert target["id"] not in {r["id"] for r in common.read_jsonl(step.ds / "queries.jsonl")}


def test_cmd_refuses_a_missing_codex(step, monkeypatch):
    monkeypatch.setenv("PATH", "/nonexistent")
    with pytest.raises(Refused, match="codex"):
        step.run()
    assert not (step.ds / "queries.jsonl").exists()
