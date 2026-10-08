"""Helpers of the generation tests: a dataset folder with a config, a codex login and the cache folder."""

from __future__ import annotations

import json
import os
from pathlib import Path

from bilbo_evals import common, llm

CONFIG = """seed = 20261007
max_calls = {max_calls}
concurrency = {concurrency}
[renderer]
cli = "claude"
model = "claude-sonnet-5-5"
[query_model]
cli = "codex"
model = "gpt-6.1-sol"
reasoning_effort = "low"
[world]
projects = {projects}
dev_projects = {dev_projects}
notes = {notes}
filler_share = 0.30
max_test_note_queries = {max_test}
[library]
corpus = "sqlite"
pages = 2
[strata.dev]
known-item = {known}
paraphrase = {known}
pt-en = {known}
alias = {alias}
supersession = {sup}
multi-hop = {sup}
kind-filter = {sup}
library = 2
no-answer = 1
[prompts.dev]
positive = 4
noise = 2
off-topic = 2
near-miss = 2
"""

DEFAULTS = dict(max_calls=200, concurrency=2, projects=4, dev_projects=2, notes=100, max_test=30, known=3, alias=2, sup=2)


def make_ds(tmp_path: Path, name: str = "ds", **overrides) -> Path:
    ds = tmp_path / "datasets" / "notes-synth" / name
    (ds / "generation").mkdir(parents=True)
    (ds / "generation/config.toml").write_text(CONFIG.format(**{**DEFAULTS, **overrides}), encoding="utf-8")
    return ds


def use_cache(tmp_path: Path, monkeypatch) -> None:
    monkeypatch.setattr(common, "CACHE_DIR", tmp_path / "cache")
    monkeypatch.delenv("CODEX_HOME", raising=False)


def login(tmp_path: Path) -> Path:
    """The invoking user's codex login, under the test's HOME."""
    auth = Path(os.environ["HOME"]) / ".codex/auth.json"
    auth.parent.mkdir(parents=True, exist_ok=True)
    auth.write_text(json.dumps({"tokens": {"access_token": "old"}}), encoding="utf-8")
    return auth


def rows(ds: Path, name: str = "calls.jsonl") -> list[dict]:
    p = ds / "generation" / name
    return common.read_jsonl(p) if p.is_file() else []


def cfg(ds: Path) -> llm.GenConfig:
    return llm.load_config(ds)


CLEAN_PROBE = {"user_instructions_first_heading": "NONE", "mcp_tools": []}


KINDS = ["plan", "spec", "design", "decision", "gotcha", "research", "review", "report", "reference"]
ALIAS_WORDS = ["Lantern", "Quokka", "Zephyrine", "Marlowe", "Tundra", "Obelisk", "Pinnacle", "Wombat", "Saffron", "Cobalt",
               "Harbinger", "Nimbus", "Orchid", "Basilisk", "Mirage", "Falconer", "Juniper", "Kestrel", "Lodestar", "Magpie"]
PROJECT_NAMES = ["Ledgerly", "Parcelo", "Vitalis", "Framebay", "Tidewire", "Kilnworks", "Quillbase", "Sunmeter"]


def fake_project(index: int, facts: int = 32, pairs: int = 6, comps: int = 4) -> dict:
    """A raw `world` project answer: components with aliases, facts with exact values, revision pairs."""
    name = PROJECT_NAMES[index]
    slug = name.lower()
    components = []
    for c in range(comps):
        alias = ALIAS_WORDS[(index * comps + c) % len(ALIAS_WORDS)] + str(index) if c < 4 else None
        aliases = [{"alias": alias, "type": ["old-name", "codename", "abbreviation"][c % 3]}] if alias else []
        components.append({"slug": f"part{c}-{slug}", "name": f"part{c}-{slug}-svc", "aliases": aliases})
    cand = []
    for n in range(facts):
        comp = components[n % comps]
        value = f"{slug.upper()}_{n:02d}_VAL"
        cand.append({
            "key": f"c{n:02d}", "component": comp["slug"], "kind": KINDS[(n % comps * 2 + (n // comps) % 2) % len(KINDS)],
            "statement": f"{comp['name']} keeps {value} at {n * 100 + 5} for the tunable number {n} of {name}.",
            "verbatim": [value, str(n * 100 + 5)], "replaces": None,
            "source": f"code: src/{comp['slug']}/mod{n}.go" if n % 4 == 0 else None,
        })
    # facts `2 * comps` apart share a component and a kind: the later one replaces the earlier
    made = 0
    for n in range(facts - 2 * comps):
        if made < pairs:
            cand[n + 2 * comps]["replaces"] = cand[n]["key"]
            cand[n + 2 * comps]["statement"] += " It replaces the earlier setting."
            made += 1
    return {
        "slug": slug, "name": name, "summary": f"{name} does things for teams.", "technologies": ["PostgreSQL", "Go"],
        "components": components, "candidate_facts": cand,
    }


def fake_world(n: int, **kw) -> dict:
    return {"seed": 20261007, "projects": [fake_project(i, **kw) for i in range(n)]}


PROFILE = {
    "schema_version": 1, "notes": 600,
    "kinds": {"plan": 0.1, "spec": 0.05, "design": 0.1, "decision": 0.25, "gotcha": 0.2, "research": 0.1,
              "review": 0.05, "report": 0.1, "reference": 0.05},
    "length_chars": {"p10": 600, "p25": 1000, "p50": 1700, "p75": 2600, "p90": 3900},
    "headings": {"p10": 1, "p25": 2, "p50": 3, "p75": 5, "p90": 7},
    "shares": {"code_blocks": 0.4, "sources": 0.25, "portuguese": 0.3, "wiki_links": 0.1},
}
