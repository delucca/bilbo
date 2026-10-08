"""generate notes and generate fidelity with the fake claude and codex."""

from __future__ import annotations

import json
import random
import subprocess
from pathlib import Path

import pytest
from gen_e_helpers import login, make_ds, rows, use_cache

from bilbo_evals import cli, common, llm
from bilbo_evals.generate import facts as facts_mod
from bilbo_evals.generate import notes as notes_mod

PROBE = {"match": "preflight probe (claude)", "output": {"user_instructions_first_heading": "NONE"}}
CODEX_PROBE = {"match": "preflight probe (codex)", "output": {"user_instructions_first_heading": "NONE", "mcp_tools": []}}

S1 = "alpha-sync retries a failed push 3 times; the key is sync.max_retries = 3."
S2 = "alpha-sync now retries a failed push 5 times; the key is sync.max_retries = 5."
S3 = "edge-cache evicts above 900 MB and logs ERR_EVICT_STORM on port 7421."
S4 = "Lantern was the previous name of edge-cache; the component is called edge-cache now."
STYLE = {"chars": 1200, "headings": 2, "code_block": False, "wiki_link": None}


@pytest.fixture(autouse=True)
def _cache(tmp_path, monkeypatch):
    use_cache(tmp_path, monkeypatch)
    login(tmp_path)


def fact(fid, kind, comp, statement, verbatim, nid, **kw):
    base = {"id": fid, "project": "alpha", "component": comp, "family": "fam-alpha-001", "kind": kind,
            "statement": statement, "verbatim": verbatim, "lang": "en", "valid_from": "2026-03-02T10:14-03:00",
            "supersedes": None, "superseded_by": None, "joins": [], "kind_pair": None, "bridge": None,
            "note_id": nid, "status": "planted", "source": None}
    base.update(kw)
    return base


def note(nid, kind, topic, comp, facts, created, **kw):
    base = {"id": nid, "file": f"{kind}-{topic}.md", "kind": kind, "topic": topic, "project": "alpha", "component": comp,
            "lang": "en", "created": created, "facts": facts, "filler": not facts, "noise": [],
            "near_duplicate_of": None, "render_attempts": 1, "status": "kept", "omit": None, "activity": None,
            "style": dict(STYLE), "sources": []}
    base.update(kw)
    return base


@pytest.fixture
def mini(tmp_path):
    ds = make_ds(tmp_path)
    rng = random.Random(3)
    ids = [facts_mod.ulid(m, rng) for m in (30_000_000, 30_100_000, 30_200_000, 30_300_000, 30_400_000)]
    n1, n2, n3, n4, n5 = ids
    world = {"seed": 1, "projects": [{
        "slug": "alpha", "name": "Alpha", "summary": "Alpha syncs things.", "technologies": ["Go", "SQLite"],
        "components": [{"slug": "alpha-sync", "name": "alpha-sync", "aliases": []},
                       {"slug": "edge-cache", "name": "edge-cache", "aliases": [{"alias": "Lantern", "type": "old-name"}]}],
        "candidate_facts": []}]}
    facts = [
        fact("f-alpha-001", "decision", "alpha-sync", S1, ["sync.max_retries = 3"], n1, superseded_by="f-alpha-002"),
        fact("f-alpha-002", "decision", "alpha-sync", S2, ["sync.max_retries = 5"], n2, supersedes="f-alpha-001",
             source="code: src/sync.go"),
        fact("f-alpha-003", "gotcha", "edge-cache", S3, ["ERR_EVICT_STORM", "7421"], n3),
        fact("f-alpha-004", "gotcha", "edge-cache", S4, ["Lantern", "edge-cache"], n3, bridge={"alias": "Lantern", "canonical": "edge-cache"}),
    ]
    notes = [
        note(n1, "decision", "sync-retry-limit", "alpha-sync", ["f-alpha-001"], "2026-03-02T10:14-03:00"),
        note(n2, "decision", "sync-retry-limit-revised", "alpha-sync", ["f-alpha-002"], "2026-04-20T15:40-03:00",
             sources=["code: src/sync.go"], omit="the reason or rationale"),
        note(n3, "gotcha", "edge-cache-evict", "edge-cache", ["f-alpha-003", "f-alpha-004"], "2026-03-18T09:05-03:00", lang="pt"),
        note(n4, "report", "alpha-sync-weekly-recap", "alpha-sync", [], "2026-05-01T08:00-03:00", activity="write a weekly recap of work"),
        note(n5, "decision", "sync-retry-limit-recap", "alpha-sync", [], "2026-06-01T08:00-03:00", near_duplicate_of=n1,
             noise=["near-duplicate"]),
    ]
    aliases = [{"project": "alpha", "component": "edge-cache", "canonical": "edge-cache", "alias": "Lantern",
                "type": "old-name", "bridge_notes": [n3]}]
    w = ds / "world"
    w.mkdir()
    common.write_json(w / "world.json", world)
    common.write_jsonl(w / "facts.jsonl", facts)
    common.write_jsonl(w / "notes.jsonl", notes)
    common.write_json(w / "aliases.json", aliases)
    return ds, ids


def body(*sentences, extra=""):
    return {"title": "A title", "body": "First paragraph of the note.\n\n## Details\n\n" + " ".join(sentences) + "\n" + extra}


GOOD = {
    "n1": body("The push is retried and", S1.split(";")[1].strip(), "We keep it as `sync.max_retries = 3`."),
    "n2": body("Replaces the earlier limit.", "The key is `sync.max_retries = 5`."),
    "n3": body("O cache registra `ERR_EVICT_STORM` na porta 7421.", "Lantern era o nome antigo de edge-cache."),
}


def claude_rules(extra=()):
    return [
        *extra, PROBE,
        {"match": S1, "output": GOOD["n1"]},
        {"match": S2, "output": GOOD["n2"]},
        {"match": S3, "output": GOOD["n3"]},
        {"match": "write a weekly recap of work", "output": body("General progress on the component this week was fine.")},
        {"match": "existing note about \"sync retry limit\"", "output": body("A loose second note about the retry limit of the sync worker.")},
    ]


def check_rule(fids_and_quotes, match):
    return {"match": match, "output": {"facts": [{"id": i, "readable": q is not None, "evidence": q or ""} for i, q in fids_and_quotes]}}


def codex_rules(extra=()):
    return [
        CODEX_PROBE, *extra,
        check_rule([("f-alpha-001", "We keep it as `sync.max_retries = 3`.")], f"fact: {S1}"),
        check_rule([("f-alpha-002", "The key is `sync.max_retries = 5`.")], f"fact: {S2}"),
        check_rule([("f-alpha-003", "O cache registra `ERR_EVICT_STORM` na porta 7421."),
                    ("f-alpha-004", "Lantern era o nome antigo de edge-cache.")], f"fact: {S3}"),
        check_rule([("f-alpha-004", "Lantern era o nome antigo de edge-cache.")], f"fact: {S4}"),
    ]


def script(claude=(), codex=()):
    """Codex rules first: a check prompt quotes the facts, which the claude rules match too."""
    return [*codex_rules(codex), *claude_rules(claude)]


def run(cmd, ds):
    return cli.main(["generate", cmd, "--dataset", str(ds)])


def read_notes(ds):
    return {n["id"]: n for n in common.read_jsonl(ds / "world/notes.jsonl")}


# --- notes ----------------------------------------------------------------------------------------------------------


def test_notes_step_writes_bilbo_notes(mini, fake_llm, bilbo_bin, tmp_path):
    ds, ids = mini
    fake_llm.set_script(script())
    assert run("notes", ds) == 0
    n1, n2, n3, n4, n5 = ids
    text = (ds / "store/notes/decision-sync-retry-limit-revised.md").read_text()
    assert text.startswith(f"---\nid: {n2}\ncreated: 2026-04-20T15:40-03:00\nsources:\n  - \"code: src/sync.go\"\n---\n\n# A title\n\n")
    assert "## Details" in text and text.endswith("\n") and "\n\n\n" not in text
    assert len(list((ds / "store/notes").glob("*.md"))) == 5
    corpus = common.read_jsonl(ds / "corpus.jsonl")
    assert {r["_id"] for r in corpus} == set(ids) and all(r["canary"] == common.CANARY for r in corpus)
    env = {"BILBO_HOME": str(ds / "store"), "HOME": str(tmp_path / "home"), "PATH": "/usr/bin:/bin",
           "XDG_CONFIG_HOME": str(tmp_path / "cfg"), "XDG_STATE_HOME": str(tmp_path / "state"), "XDG_CACHE_HOME": str(tmp_path / "cache")}
    check = subprocess.run([str(bilbo_bin), "check"], env=env, capture_output=True, text=True)
    assert (check.returncode, check.stdout.strip()) == (0, ""), check.stdout + check.stderr


def test_the_note_prompt_carries_the_manifest(mini, fake_llm):
    ds, ids = mini
    fake_llm.set_script(script())
    assert run("notes", ds) == 0
    prompts = {c["prompt"] for c in fake_llm.calls()}
    by = lambda needle: next(p for p in prompts if needle in p)
    p2 = by(S2)
    assert common.sha256_bytes(notes_mod.SKILL.read_bytes()) in p2 and "<style-guide>" in p2
    assert "`sync.max_retries = 5`" in p2 and "Note kind: decision" in p2 and "Language of the note: English" in p2
    assert "replaces an earlier note about \"sync retry limit\"" in p2
    assert "Leave out the reason or rationale" in p2 and "Use no fenced code blocks" in p2
    p3 = by(S3)
    assert "Brazilian Portuguese" in p3 and "`ERR_EVICT_STORM`, `7421`" in p3 and "`Lantern`, `edge-cache`" in p3
    p4 = by("weekly recap")
    assert "Do not state any exact number" in p4 and "Must appear exactly" not in p4
    p5 = by("existing note about")
    assert "sync retry limit" in p5 and S1 not in p5 and "3 times" not in p5
    assert "Your previous attempt was rejected" not in p2


def test_notes_step_is_resumable(mini, fake_llm):
    ds, ids = mini
    fake_llm.set_script(script())
    assert run("notes", ds) == 0
    n = len(fake_llm.calls())
    assert run("notes", ds) == 0
    assert len(fake_llm.calls()) == n
    llm.output_path(ds, "notes", ids[3], 1).unlink()
    assert run("notes", ds) == 0
    assert len(fake_llm.calls()) == n + 1


def test_notes_step_stops_at_the_budget(tmp_path, mini, fake_llm, capsys):
    ds, ids = mini
    cfg = (ds / "generation/config.toml").read_text().replace("max_calls = 200", "max_calls = 3").replace("concurrency = 2", "concurrency = 1")
    (ds / "generation/config.toml").write_text(cfg)
    fake_llm.set_script(script())
    assert run("notes", ds) == 1
    err = capsys.readouterr().err
    assert "budget of 3" in err and "notes left" in err
    assert len(list((ds / "store/notes").glob("*.md"))) == 2
    assert (ds / "corpus.jsonl").is_file()


def test_clean_body_fixes_headings_and_front_matter():
    text = notes_mod.clean_body("My note", "# My note\n\nIntro\n\n# Second\n\n```sh\n# comment\n```\n")
    assert text == "Intro\n\n## Second\n\n```sh\n# comment\n```"
    assert notes_mod.clean_body("T", "---\nid: x\n---\n\nBody") == "Body"


def test_notes_need_the_earlier_steps(tmp_path, fake_llm, capsys):
    ds = make_ds(tmp_path)
    assert run("notes", ds) == 1
    assert "run `generate world` first" in capsys.readouterr().err


def test_notes_refuse_a_frozen_dataset(mini, fake_llm, capsys):
    ds, _ = mini
    (ds / "FROZEN").write_text("x\n")
    assert run("notes", ds) == 1
    assert "frozen" in capsys.readouterr().err and not (ds / "store").exists()


# --- fidelity -------------------------------------------------------------------------------------------------------


def start(ds, fake_llm, extra_claude=()):
    fake_llm.set_script(script(extra_claude))
    assert run("notes", ds) == 0


def test_a_fact_that_survives_is_kept(mini, fake_llm):
    ds, ids = mini
    start(ds, fake_llm)
    before = len(rows(ds))
    assert run("fidelity", ds) == 0
    facts = {f["id"]: f for f in common.read_jsonl(ds / "world/facts.jsonl")}
    assert all(f["status"] == "planted" for f in facts.values())
    new = [r for r in rows(ds)[before:]]
    assert {r["cli"] for r in new} == {"codex"} and len([r for r in new if r["step"] == "fidelity"]) == 3
    assert not (ds / "generation/drops.jsonl").exists()
    assert all(n["render_attempts"] == 1 for n in read_notes(ds).values())


def test_a_missing_string_is_rerendered_and_then_kept(mini, fake_llm):
    ds, ids = mini
    lost = body("The cache logs ERR_EVICT_STORM and evicts a lot.")
    start(ds, fake_llm, [{"match": S3, "output": lost, "times": 1}])
    assert "7421" not in (ds / "store/notes/gotcha-edge-cache-evict.md").read_text()
    assert run("fidelity", ds) == 0
    assert "7421" in (ds / "store/notes/gotcha-edge-cache-evict.md").read_text()
    assert read_notes(ds)[ids[2]]["render_attempts"] == 2
    rerender = [c for c in fake_llm.calls() if "Your previous attempt was rejected" in c["prompt"]]
    assert len(rerender) == 1 and "`7421` exactly as written" in rerender[0]["prompt"] and "ERR_EVICT_STORM" not in rerender[0]["prompt"].split("rejected")[1].split("Answer with")[0]
    assert not (ds / "generation/drops.jsonl").exists()


def test_a_fact_lost_three_times_is_dropped_and_logged(mini, fake_llm):
    ds, ids = mini
    lost = body("The cache logs ERR_EVICT_STORM and evicts a lot. Lantern era o nome antigo de edge-cache.")
    start(ds, fake_llm, [{"match": S3, "output": lost}])
    assert run("fidelity", ds) == 0
    renders = [r for r in rows(ds) if r["step"] == "notes" and r["item"] == ids[2]]
    assert [r["attempt"] for r in renders] == [1, 2, 3]
    facts = {f["id"]: f for f in common.read_jsonl(ds / "world/facts.jsonl")}
    assert facts["f-alpha-003"]["status"] == "dropped" and facts["f-alpha-004"]["status"] == "planted"
    drop = common.read_jsonl(ds / "generation/drops.jsonl")
    assert len(drop) == 1 and drop[0]["item"] == "f-alpha-003" and drop[0]["note"] == ids[2] and drop[0]["kind"] == "fact"
    assert "7421" in drop[0]["reason"]
    n3 = read_notes(ds)[ids[2]]
    assert n3["facts"] == ["f-alpha-004"] and n3["filler"] is False and n3["render_attempts"] == 3
    assert run("fidelity", ds) == 0
    assert len(common.read_jsonl(ds / "generation/drops.jsonl")) == 1


def test_a_note_left_with_no_fact_becomes_a_filler_and_a_lost_bridge_is_unlinked(mini, fake_llm):
    ds, ids = mini
    lost = body("The cache evicts a lot and nothing more is said here.")
    start(ds, fake_llm, [{"match": S3, "output": lost}])
    assert run("fidelity", ds) == 0
    n3 = read_notes(ds)[ids[2]]
    assert n3["facts"] == [] and n3["filler"] is True
    alias = json.loads((ds / "world/aliases.json").read_text())[0]
    assert alias["bridge_notes"] == []
    assert len(common.read_jsonl(ds / "generation/drops.jsonl")) == 2
    corpus = {r["_id"] for r in common.read_jsonl(ds / "corpus.jsonl")}
    assert ids[2] in corpus


def test_the_checker_can_fail_a_fact_the_strings_pass(mini, fake_llm):
    ds, ids = mini
    start(ds, fake_llm)
    bad = check_rule([("f-alpha-001", None)], f"fact: {S1}")
    fake_llm.set_script(script(codex=[{**bad, "times": 1}]))
    assert run("fidelity", ds) == 0
    assert read_notes(ds)[ids[0]]["render_attempts"] == 2
    assert any("could not read it" in c["prompt"] or "State this fact explicitly" in c["prompt"] for c in fake_llm.calls())


def test_evidence_that_is_not_a_quote_fails(mini, fake_llm):
    ds, ids = mini
    start(ds, fake_llm)
    fake_llm.set_script(script(codex=[{**check_rule([("f-alpha-002", "Something the note never says.")], f"fact: {S2}"), "times": 1}]))
    assert run("fidelity", ds) == 0
    assert read_notes(ds)[ids[1]]["render_attempts"] == 2


def test_fidelity_needs_rendered_notes(mini, fake_llm, capsys):
    ds, _ = mini
    fake_llm.set_script(script())
    assert run("fidelity", ds) == 1
    assert "run `generate notes` first" in capsys.readouterr().err


def test_fidelity_without_codex_is_refused(mini, fake_llm, monkeypatch, tmp_path, capsys):
    ds, _ = mini
    start(ds, fake_llm)
    only_claude = tmp_path / "only-claude"
    only_claude.mkdir()
    (only_claude / "claude").symlink_to(Path(__file__).resolve().parent / "fakes/claude")
    monkeypatch.setenv("PATH", f"{only_claude}:/usr/bin:/bin")
    assert run("fidelity", ds) == 1
    assert "`codex` is missing" in capsys.readouterr().err


def test_the_last_attempt_checks_the_facts_whose_strings_are_present(mini, fake_llm):
    ds, ids = mini
    lost = body("The cache logs ERR_EVICT_STORM and evicts a lot. Lantern era o nome antigo de edge-cache.")
    start(ds, fake_llm, [{"match": S3, "output": lost}])
    unreadable = check_rule([("f-alpha-004", None)], f"fact: {S4}")
    fake_llm.set_script(script(codex=[unreadable], claude=[{"match": S3, "output": lost}]))
    before = len(fake_llm.calls())
    assert run("fidelity", ds) == 0
    facts = {f["id"]: f for f in common.read_jsonl(ds / "world/facts.jsonl")}
    assert facts["f-alpha-003"]["status"] == "dropped" and facts["f-alpha-004"]["status"] == "dropped"
    reasons = {d["item"]: d["reason"] for d in common.read_jsonl(ds / "generation/drops.jsonl")}
    assert "7421" in reasons["f-alpha-003"] and "could not read" in reasons["f-alpha-004"]
    checks = [c for c in fake_llm.calls()[before:] if "Facts to check" in c["prompt"] and "Lantern" in c["prompt"]]
    assert len(checks) == 1 and f"fact: {S4}" in checks[0]["prompt"] and f"fact: {S3}" not in checks[0]["prompt"]


def test_one_failed_check_does_not_block_the_other_notes(mini, fake_llm, capsys):
    ds, ids = mini
    start(ds, fake_llm)
    broken = {"match": f"fact: {S1}", "output": {}, "exit": 1}
    bad = check_rule([("f-alpha-002", None)], f"fact: {S2}")
    fake_llm.set_script(script(codex=[broken, {**bad, "times": 1}]))
    assert run("fidelity", ds) == 1
    assert ids[0] in capsys.readouterr().err
    notes = read_notes(ds)
    assert notes[ids[1]]["render_attempts"] == 2 and notes[ids[0]]["render_attempts"] == 1
    facts = {f["id"]: f for f in common.read_jsonl(ds / "world/facts.jsonl")}
    assert facts["f-alpha-001"]["status"] == "planted" and facts["f-alpha-002"]["status"] == "planted"
    assert not (ds / "generation/drops.jsonl").exists()


def test_noise_stale_follows_the_dropped_facts(mini, fake_llm):
    ds, ids = mini
    common.write_json(ds / "world/noise.json", {"near-duplicate": [ids[4]], "omission": [ids[1]], "stale": [[ids[0], ids[1]]]})
    lost = body("The push is retried and the key is something.")
    start(ds, fake_llm, [{"match": S2, "output": lost}])
    assert run("fidelity", ds) == 0
    facts = {f["id"]: f for f in common.read_jsonl(ds / "world/facts.jsonl")}
    assert facts["f-alpha-002"]["status"] == "dropped"
    noise = json.loads((ds / "world/noise.json").read_text())
    assert noise["stale"] == [] and noise["omission"] == [ids[1]] and noise["near-duplicate"] == [ids[4]]
