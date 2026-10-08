"""generate world (with the fake claude) and generate facts (seeded, no model): shape, structure and determinism."""

from __future__ import annotations

import copy
import json
import re
from datetime import datetime

import pytest
from gen_e_helpers import KINDS, PROFILE, fake_project, fake_world, make_ds, rows, use_cache

from bilbo_evals import cli, common, llm
from bilbo_evals.generate import facts as facts_mod
from bilbo_evals.generate import world as world_mod

PROBE = {"match": "preflight probe (claude)", "output": {"user_instructions_first_heading": "NONE"}}
CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"


@pytest.fixture(autouse=True)
def _cache(tmp_path, monkeypatch):
    use_cache(tmp_path, monkeypatch)


def put_world(ds, projects=4, profile=None, **kw):
    w = ds / "world"
    w.mkdir(parents=True, exist_ok=True)
    common.write_json(w / "world.json", fake_world(projects, **kw))
    common.write_json(w / "profile.json", profile or PROFILE)


def run_facts(ds):
    return cli.main(["generate", "facts", "--dataset", str(ds)])


def load(ds):
    w = ds / "world"
    return {
        "facts": common.read_jsonl(w / "facts.jsonl"), "notes": common.read_jsonl(w / "notes.jsonl"),
        "aliases": json.loads((w / "aliases.json").read_text()), "noise": json.loads((w / "noise.json").read_text()),
        "splits": json.loads((w / "splits.json").read_text()),
    }


def ulid_minute(ulid: str) -> int:
    ms = 0
    for ch in ulid[:10]:
        ms = ms * 32 + CROCKFORD.index(ch)
    return ms // 60000


# --- world ------------------------------------------------------------------------------------------------------


def listing(n):
    return {"projects": [
        {"slug": p["slug"], "name": p["name"], "summary": p["summary"], "technologies": p["technologies"]}
        for p in (fake_project(i) for i in range(n))]}


def project_rules(n, **bad):
    rules = []
    for i in range(n):
        p = fake_project(i)
        out = {"components": p["components"], "candidate_facts": p["candidate_facts"]}
        rules.append({"match": f"(slug {p['slug']})", "output": bad.get(p["slug"], out)})
    return rules


def test_world_step_writes_projects_components_and_facts(tmp_path, fake_llm):
    ds = make_ds(tmp_path, projects=4, dev_projects=2)
    fake_llm.set_script([PROBE, {"match": "designing fictional software projects", "output": listing(4)}, *project_rules(4)])
    assert cli.main(["generate", "world", "--dataset", str(ds)]) == 0
    world = json.loads((ds / "world/world.json").read_text())
    assert world["seed"] == 20261007 and len(world["projects"]) == 4
    p = world["projects"][0]
    assert {"slug", "name", "summary", "technologies", "components", "candidate_facts"} <= set(p)
    assert sum(1 for f in p["candidate_facts"] if f["replaces"]) == 6
    assert p["components"][0]["aliases"][0]["alias"] == "Lantern0"
    assert all(c["call_id"] and c["status"] == "ok" for c in rows(ds))
    # probe + project list + 4 projects, every one by claude
    assert len(rows(ds)) == 6 and {c["cli"] for c in rows(ds)} == {"claude"}
    prompt = [c for c in fake_llm.calls() if "(slug parcelo)" in c["prompt"]][0]["prompt"]
    assert "Parcelo" in prompt and "PostgreSQL" in prompt and "replaces" in prompt


def test_world_step_resumes_without_new_calls(tmp_path, fake_llm):
    ds = make_ds(tmp_path, projects=4, dev_projects=2)
    fake_llm.set_script([PROBE, {"match": "designing fictional software projects", "output": listing(4)}, *project_rules(4)])
    assert cli.main(["generate", "world", "--dataset", str(ds)]) == 0
    first = (ds / "world/world.json").read_text()
    n = len(fake_llm.calls())
    assert cli.main(["generate", "world", "--dataset", str(ds)]) == 0
    assert len(fake_llm.calls()) == n and (ds / "world/world.json").read_text() == first


def test_world_step_drops_unusable_facts_and_rerolls_a_poor_project(tmp_path, fake_llm):
    ds = make_ds(tmp_path, projects=4, dev_projects=2)
    poor = copy.deepcopy(fake_project(1))
    for f in poor["candidate_facts"]:
        f["verbatim"] = ["not in the statement"]
    rules = [PROBE, {"match": "designing fictional software projects", "output": listing(4)}]
    rules += [r for r in project_rules(4) if "(slug parcelo)" not in r["match"]]
    rules += [
        {"match": "(slug parcelo)", "output": {"components": poor["components"], "candidate_facts": poor["candidate_facts"]}, "times": 1},
        {"match": "(slug parcelo)", "output": {"components": fake_project(1)["components"], "candidate_facts": fake_project(1)["candidate_facts"]}},
    ]
    fake_llm.set_script(rules)
    assert cli.main(["generate", "world", "--dataset", str(ds)]) == 0
    assert len(json.loads((ds / "world/world.json").read_text())["projects"][1]["candidate_facts"]) == 32
    assert (ds / "generation/outputs/world/parcelo.2.json").is_file()


def test_sanitize_rules():
    p = fake_project(0)
    raw = {"components": copy.deepcopy(p["components"]), "candidate_facts": copy.deepcopy(p["candidate_facts"])}
    raw["candidate_facts"][0]["statement"] = f"part0-ledgerly-svc, formerly {raw['components'][0]['aliases'][0]['alias']}, keeps LEDGERLY_00_VAL at 5."
    raw["candidate_facts"][1]["kind"] = "diary"
    raw["candidate_facts"][2]["component"] = "ghost"
    raw["candidate_facts"][3]["replaces"] = "c03"
    raw["candidate_facts"][13]["replaces"] = "c02"
    raw["components"][1]["aliases"].append({"alias": "part0-ledgerly", "type": "codename"})
    raw["components"][2]["aliases"] = [{"alias": "x", "type": "codename"}]
    raw["candidate_facts"][4]["source"] = "https://example.org/made-up"
    clean = world_mod.sanitize(raw)
    keys = {f["key"] for f in clean["candidate_facts"]}
    assert not keys & {"c00", "c01", "c02"} and "c03" in keys
    by = {f["key"]: f for f in clean["candidate_facts"]}
    assert by["c03"]["replaces"] is None and by["c13"]["replaces"] is None
    assert by["c04"]["source"] is None and by["c00" if "c00" in by else "c05"]["source"] in (None, by["c05"]["source"])
    assert clean["components"][1]["aliases"] == p["components"][1]["aliases"]
    assert clean["components"][2]["aliases"] == []


def test_a_short_project_list_is_rerolled_then_refused(tmp_path, fake_llm, capsys):
    ds = make_ds(tmp_path, projects=4, dev_projects=2)
    fake_llm.set_script([PROBE, {"match": "designing fictional software projects", "output": listing(2)}])
    assert cli.main(["generate", "world", "--dataset", str(ds)]) == 1
    assert "project lists" in capsys.readouterr().err
    assert len([c for c in rows(ds) if c["step"] == "world"]) == 3


def test_world_step_stops_at_the_budget_and_keeps_its_work(tmp_path, fake_llm, capsys):
    ds = make_ds(tmp_path, projects=4, dev_projects=2, max_calls=4, concurrency=1)
    fake_llm.set_script([PROBE, {"match": "designing fictional software projects", "output": listing(4)}, *project_rules(4)])
    assert cli.main(["generate", "world", "--dataset", str(ds)]) == 1
    err = capsys.readouterr().err
    assert "budget of 4" in err and "projects left" in err
    assert len(list((ds / "generation/outputs/world").glob("*.json"))) == 3
    assert not (ds / "world/world.json").exists()


def test_world_targets_follow_the_config(tmp_path):
    ds = make_ds(tmp_path, projects=14, dev_projects=6, notes=600, max_test=450, known=20, alias=20, sup=20)
    t = world_mod.targets(llm.load_config(ds))
    assert t["facts"] == 48 and t["pairs"] >= 10


# --- facts ----------------------------------------------------------------------------------------------------------


@pytest.fixture
def planned(tmp_path):
    ds = make_ds(tmp_path)
    put_world(ds)
    assert run_facts(ds) == 0
    return ds, load(ds)


def test_facts_files_have_the_documented_shape(planned):
    ds, d = planned
    f = d["facts"][0]
    assert {"id", "project", "component", "family", "kind", "statement", "verbatim", "lang", "valid_from", "supersedes",
            "superseded_by", "joins", "kind_pair", "bridge", "note_id", "status"} <= set(f)
    n = d["notes"][0]
    assert {"id", "file", "kind", "topic", "project", "lang", "created", "facts", "filler", "noise", "near_duplicate_of",
            "render_attempts", "status"} <= set(n)
    assert re.fullmatch(r"f-[a-z]+-\d{3}", f["id"]) and re.fullmatch(r"fam-[a-z]+-\d{3}", f["family"])
    assert all(x["status"] == "planted" for x in d["facts"]) and all(x["status"] == "kept" for x in d["notes"])
    assert len(d["notes"]) == 100 and sum(1 for x in d["notes"] if x["filler"]) == 30


def test_splits_are_by_project(planned):
    _, d = planned
    s = d["splits"]
    assert len(s["dev"]) == 2 and len(s["test"]) == 2 and not set(s["dev"]) & set(s["test"])
    assert s["none_prompts"] == {"dev": ["p-none-001", "p-none-002", "p-none-003", "p-none-004"],
                                 "test": ["p-none-005", "p-none-006", "p-none-007", "p-none-008"]}
    assert {f["project"] for f in d["facts"]} == set(s["dev"]) | set(s["test"])


def test_ulids_match_created_and_the_timeline_is_eighteen_months(planned):
    _, d = planned
    low, high = datetime.fromisoformat("2025-04-01T00:00-03:00"), datetime.fromisoformat("2026-09-30T23:59-03:00")
    ids = set()
    for n in d["notes"]:
        assert re.fullmatch(r"[0-9A-HJKMNP-TV-Z]{26}", n["id"])
        created = datetime.fromisoformat(n["created"])
        assert low <= created <= high and n["created"].endswith("-03:00")
        assert ulid_minute(n["id"]) == int(created.timestamp() // 60)
        ids.add(n["id"])
    assert len(ids) == len(d["notes"])
    assert [n["id"] for n in d["notes"]] == sorted(n["id"] for n in d["notes"])


def test_topics_are_valid_and_unique_across_kinds(planned):
    _, d = planned
    topics = [n["topic"] for n in d["notes"]]
    assert len(set(topics)) == len(topics)
    for n in d["notes"]:
        assert re.fullmatch(r"[a-z0-9]+(-[a-z0-9]+)*", n["topic"]) and n["file"] == f"{n['kind']}-{n['topic']}.md"
        assert n["kind"] in KINDS


def test_every_fact_lives_in_a_note_of_its_kind_and_time(planned):
    _, d = planned
    notes = {n["id"]: n for n in d["notes"]}
    for f in d["facts"]:
        n = notes[f["note_id"]]
        assert f["id"] in n["facts"] and f["project"] == n["project"] and f["lang"] == n["lang"]
        assert f["valid_from"] == n["created"] and f["kind"] == n["kind"]
        assert all(v in f["statement"] for v in f["verbatim"])
    for n in d["notes"]:
        assert bool(n["facts"]) != n["filler"]


def test_supersession_pairs_are_sibling_notes_in_time_order(planned):
    _, d = planned
    facts = {f["id"]: f for f in d["facts"]}
    notes = {n["id"]: n for n in d["notes"]}
    pairs = [f for f in d["facts"] if f["supersedes"]]
    assert len(pairs) >= 5
    for new in pairs:
        old = facts[new["supersedes"]]
        assert old["superseded_by"] == new["id"] and old["project"] == new["project"]
        assert old["note_id"] != new["note_id"] and old["kind"] == new["kind"] and old["component"] == new["component"]
        gap = datetime.fromisoformat(new["valid_from"]) - datetime.fromisoformat(old["valid_from"])
        assert gap.days >= 7
        assert notes[new["note_id"]]["topic"] == notes[old["note_id"]]["topic"] + "-revised"
        assert old["lang"] == new["lang"]
    assert sorted(d["noise"]["stale"]) == sorted([facts[f["supersedes"]]["note_id"], f["note_id"]] for f in pairs)


def test_joins_and_kind_pairs_are_symmetric_and_families_stay_small(planned):
    _, d = planned
    facts = {f["id"]: f for f in d["facts"]}
    joins = [f for f in d["facts"] if f["joins"]]
    kinds = [f for f in d["facts"] if f["kind_pair"]]
    assert len(joins) >= 8 and len(kinds) >= 8
    for f in joins:
        group = {f["id"], *f["joins"]}
        assert 2 <= len(group) <= 3 and len({facts[i]["note_id"] for i in group}) == len(group)
        assert len({facts[i]["project"] for i in group}) == 1
        assert all(set(facts[i]["joins"]) | {i} == group for i in group)
        assert not any(facts[i]["superseded_by"] for i in group)
    for f in kinds:
        mate = facts[f["kind_pair"]]
        assert mate["kind_pair"] == f["id"] and mate["kind"] != f["kind"] and mate["component"] == f["component"]
        assert mate["note_id"] != f["note_id"] and not f["superseded_by"]
    sizes = {}
    for f in d["facts"]:
        sizes[f["family"]] = sizes.get(f["family"], 0) + 1
    assert max(sizes.values()) <= 6
    for f in d["facts"]:
        for other in [f["supersedes"], f["superseded_by"], f["kind_pair"], *f["joins"]]:
            if other:
                assert facts[other]["family"] == f["family"]


def test_alias_bridges(planned):
    _, d = planned
    facts = {f["id"]: f for f in d["facts"]}
    notes = {n["id"]: n for n in d["notes"]}
    world = json.loads((planned[0] / "world/world.json").read_text())
    aliases = {a["alias"]: a for a in d["aliases"]}
    assert len(aliases) == 16
    for a in d["aliases"]:
        assert a["bridge_notes"] and {"project", "component", "canonical", "alias", "type"} <= set(a)
        host = notes[a["bridge_notes"][0]]
        bridge = [facts[i] for i in host["facts"] if facts[i]["bridge"]]
        assert bridge and bridge[0]["bridge"] == {"alias": a["alias"], "canonical": a["canonical"]}
        assert a["alias"] in bridge[0]["statement"] and a["canonical"] in bridge[0]["statement"]
        assert host["project"] == a["project"] and not host["filler"]
    for f in d["facts"]:
        if not f["bridge"]:
            assert not any(re.search(rf"\b{a}\b", f["statement"], re.I) for a in aliases)
    assert {c["slug"] for p in world["projects"] for c in p["components"]} >= {a["component"] for a in d["aliases"]}


def test_noise_is_recorded(planned):
    _, d = planned
    notes = {n["id"]: n for n in d["notes"]}
    dups = [n for n in d["notes"] if n["near_duplicate_of"]]
    assert len(dups) == 6 and all(n["filler"] and n["facts"] == [] for n in dups)
    assert sorted(d["noise"]["near-duplicate"]) == sorted([n["near_duplicate_of"], n["id"]] for n in dups)
    for orig, dup in d["noise"]["near-duplicate"]:
        assert notes[orig]["kind"] == notes[dup]["kind"] and notes[orig]["component"] == notes[dup]["component"]
        assert notes[dup]["created"] > notes[orig]["created"]
    assert set(d["noise"]["omission"]) == {n["id"] for n in d["notes"] if n["omit"]}
    assert all("omission" in notes[i]["noise"] for i in d["noise"]["omission"])


def test_languages_follow_the_profile_and_cover_both(planned):
    _, d = planned
    share = sum(1 for n in d["notes"] if n["lang"] == "pt") / len(d["notes"])
    assert 0.1 < share < 0.6


def test_style_targets_come_from_the_profile(planned):
    _, d = planned
    for n in d["notes"]:
        s = n["style"]
        assert s["chars"] >= 400 and s["headings"] >= 1 and isinstance(s["code_block"], bool)
        assert s["wiki_link"] is None or s["wiki_link"] in {x["topic"] for x in d["notes"]}
        assert isinstance(n["sources"], list)


def test_facts_are_deterministic_for_a_seed(tmp_path):
    outs = []
    for name, seed in (("a", 20261007), ("b", 20261007), ("c", 5)):
        ds = make_ds(tmp_path, name)
        put_world(ds)
        text = (ds / "generation/config.toml").read_text().replace("seed = 20261007", f"seed = {seed}")
        (ds / "generation/config.toml").write_text(text)
        assert run_facts(ds) == 0
        outs.append({p.name: p.read_bytes() for p in sorted((ds / "world").glob("*")) if p.name in
                     ("facts.jsonl", "notes.jsonl", "aliases.json", "noise.json", "splits.json")})
    assert outs[0] == outs[1] and outs[0] != outs[2]


def test_rerun_keeps_the_library_split(tmp_path):
    ds = make_ds(tmp_path)
    put_world(ds)
    assert run_facts(ds) == 0
    splits = json.loads((ds / "world/splits.json").read_text())
    splits["library"] = {"dev": ["sqlite/wal"], "test": ["sqlite/busy"]}
    common.write_json(ds / "world/splits.json", splits)
    assert run_facts(ds) == 0
    assert json.loads((ds / "world/splits.json").read_text())["library"] == splits["library"]


def test_capacity_is_printed_and_a_shortfall_refuses(tmp_path, capsys):
    ds = make_ds(tmp_path)
    put_world(ds)
    assert run_facts(ds) == 0
    out = capsys.readouterr().out
    assert "stratum" in out and "supersession" in out and "multi-hop" in out
    short = make_ds(tmp_path, "short", sup=30, known=30)
    put_world(short)
    assert run_facts(short) == 1
    assert "short of the strata" in capsys.readouterr().err
    assert not (short / "world/facts.jsonl").exists()


def test_facts_need_the_world_and_the_profile(tmp_path, capsys):
    ds = make_ds(tmp_path)
    assert run_facts(ds) == 1
    assert "run `generate world` first" in capsys.readouterr().err
    (ds / "world").mkdir()
    common.write_json(ds / "world/world.json", fake_world(4))
    assert run_facts(ds) == 1
    assert "generate profile" in capsys.readouterr().err


def test_facts_refuse_a_frozen_dataset(tmp_path, capsys):
    ds = make_ds(tmp_path)
    put_world(ds)
    (ds / "FROZEN").write_text("x\n")
    assert run_facts(ds) == 1
    assert "frozen" in capsys.readouterr().err
    assert not (ds / "world/facts.jsonl").exists()


def test_unit_helpers():
    import random

    rng = random.Random(1)
    assert facts_mod.largest_remainder(10, {"a": 1, "b": 1, "c": 1}) == {"a": 4, "b": 3, "c": 3} or sum(
        facts_mod.largest_remainder(10, {"a": 1, "b": 1, "c": 1}).values()) == 10
    assert sum(facts_mod.spread(7, {"a": 2, "b": 9}, rng).values()) == 7
    assert facts_mod.spread(10, {"a": 2, "b": 3}, rng) == {"a": 2, "b": 3}
    d = facts_mod.DSU()
    assert d.can_join(["a", "b"]) and (d.join(["a", "b"]) or True)
    assert not d.can_join(["a", "b"])
    q = {"p10": 100, "p25": 200, "p50": 300, "p75": 400, "p90": 500}
    assert all(100 <= facts_mod.sample_quantiles(q, rng) <= 500 for _ in range(50))
    assert facts_mod.topic_for("edge-cache-layer", "The retry count is raised to five", set()) == "edge-cache-retry-count"
