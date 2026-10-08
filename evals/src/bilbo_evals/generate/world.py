"""`generate world`: the renderer designs the fictional projects, their components, aliases and candidate facts."""

from __future__ import annotations

import math
import re
import unicodedata
from pathlib import Path

from bilbo_evals import common, llm
from bilbo_evals.generate import finish, needs, schema, sections, solve
from bilbo_evals.schema import NOTE_KINDS

STEP = "world"
SLUG = re.compile(r"[a-z][a-z0-9-]{1,40}")
ALIAS = re.compile(r"[A-Za-z][A-Za-z0-9_-]{3,23}")
ALIAS_TYPES = ["old-name", "codename", "abbreviation"]
SOURCE_SHARE = 25


def slugify(text: str) -> str:
    folded = "".join(c for c in unicodedata.normalize("NFD", text.lower()) if not unicodedata.combining(c))
    return re.sub(r"[^a-z0-9]+", "-", folded).strip("-")[:40]


def targets(cfg: llm.GenConfig) -> dict[str, int]:
    """What one project call asks for, from the config: facts for the notes, and revision pairs for supersession."""
    w = cfg.world
    projects = int(w["projects"])
    test_projects = max(1, projects - int(w["dev_projects"]))
    gold = int(w["notes"]) * (1 - float(w["filler_share"]))
    return {
        "facts": math.ceil(gold / projects * 1.6),
        "pairs": math.ceil(1.15 * needs(cfg)["test"]["supersession"] / test_projects) + 2,
        "components": 8,
        "alias_components": 5,
    }


def project_list(out: dict, want: int) -> tuple[list[dict], str | None]:
    """The usable projects of a list answer, and a problem when there are fewer than `want`."""
    seen, projects = set(), []
    for p in out["projects"]:
        slug = p["slug"] if SLUG.fullmatch(p["slug"]) else slugify(p["name"])
        if not SLUG.fullmatch(slug) or slug in seen or not p["name"].strip() or len(p["technologies"]) < 2:
            continue
        seen.add(slug)
        projects.append({
            "slug": slug, "name": p["name"].strip(), "summary": p["summary"].strip(),
            "technologies": [t.strip() for t in p["technologies"] if t.strip()],
        })
    if len(projects) < want:
        return projects, f"{len(projects)} usable projects, {want} wanted"
    return projects[:want], None


def has_word(text: str, term: str) -> bool:
    return re.search(rf"(?<![A-Za-z0-9]){re.escape(term.lower())}(?![A-Za-z0-9])", text.lower()) is not None


def sanitize(raw: dict) -> dict:
    """Components, aliases and candidate facts of a project answer with every unusable entry dropped."""
    comps, slugs, names = [], set(), []
    for c in raw["components"]:
        slug = c["slug"] if SLUG.fullmatch(c["slug"]) else slugify(c["name"])
        name = c["name"].strip()
        if not SLUG.fullmatch(slug) or slug in slugs or len(name) < 6 or any(name.lower() == n.lower() for n in names):
            continue
        slugs.add(slug)
        names.append(name)
        comps.append({"slug": slug, "name": name, "aliases": c["aliases"]})
    aliases: list[str] = []
    for c in comps:
        kept = []
        for a in c["aliases"][:2]:
            alias = a["alias"].strip()
            low = alias.lower()
            clash = any(low in n.lower() or n.lower() in low for n in names) or any(low in x.lower() or x.lower() in low for x in aliases)
            if a["type"] in ALIAS_TYPES and ALIAS.fullmatch(alias) and not clash:
                kept.append({"alias": alias, "type": a["type"]})
                aliases.append(alias)
        c["aliases"] = kept
    facts, keys = [], set()
    for f in raw["candidate_facts"]:
        statement = f["statement"].strip()
        verbatim = [v for v in f["verbatim"] if v.strip()]
        ok = (
            f["key"] not in keys and f["component"] in slugs and f["kind"] in NOTE_KINDS and len(statement) >= 20
            and verbatim and all(v in statement for v in verbatim)
            and not any(has_word(statement, a) for a in aliases) and not any(has_word(v, a) for v in verbatim for a in aliases)
        )
        if not ok:
            continue
        keys.add(f["key"])
        source = f.get("source")
        facts.append({
            "key": f["key"], "component": f["component"], "kind": f["kind"], "statement": statement,
            "verbatim": verbatim, "replaces": f.get("replaces"),
            "source": source.strip() if isinstance(source, str) and re.match(r"(code|doc): \S", source.strip()) else None,
        })
    by_key = {f["key"]: f for f in facts}
    used: set[str] = set()
    chosen: dict[str, str] = {}
    for f in facts:
        old = by_key.get(f["replaces"] or "")
        if old and old is not f and old["component"] == f["component"] and old["kind"] == f["kind"] \
                and f["key"] not in used and old["key"] not in used:
            used.update((f["key"], old["key"]))
            chosen[f["key"]] = old["key"]
    for f in facts:
        f["replaces"] = chosen.get(f["key"])
    return {"components": comps, "candidate_facts": facts}


def project_problem(out: dict, want_facts: int) -> str | None:
    clean = sanitize(out)
    if len(clean["components"]) < 3:
        return f"{len(clean['components'])} usable components, 3 needed"
    if len(clean["candidate_facts"]) < math.ceil(0.6 * want_facts):
        return f"{len(clean['candidate_facts'])} usable facts, {math.ceil(0.6 * want_facts)} needed"
    return None


def _prompts(cfg: llm.GenConfig, t: dict[str, int], p: dict) -> str:
    return sections("world.md")["project"].substitute(
        name=p["name"], slug=p["slug"], summary=p["summary"], technologies=", ".join(p["technologies"]),
        components=t["components"], alias_components=t["alias_components"], facts=t["facts"], pairs=t["pairs"],
        source_share=SOURCE_SHARE,
    )


def cmd(args) -> int:
    ds = Path(args.dataset)
    cfg = llm.load_config(ds)
    llm.require_cli("claude")
    want = int(cfg.world["projects"])
    world_schema = schema("world.json")
    t = targets(cfg)

    prompt = sections("world.md")["list"].substitute(count=want)
    listed = solve(
        ds, cfg, STEP, "claude", {"list": (prompt, world_schema["project_list"])},
        lambda item, out: project_list(out, want)[1],
    )
    finish(STEP, listed, 1, "project lists")
    projects, _ = project_list(listed.good["list"], want)

    wants = {p["slug"]: (_prompts(cfg, t, p), world_schema["project"]) for p in projects}
    solved = solve(ds, cfg, STEP, "claude", wants, lambda item, out: project_problem(out, t["facts"]))
    finish(STEP, solved, len(projects), "projects")

    taken: set[str] = set()
    for p in projects:
        clean = sanitize(solved.good[p["slug"]])
        for c in clean["components"]:
            fresh = [a for a in c["aliases"] if a["alias"].lower() not in taken]
            taken.update(a["alias"].lower() for a in fresh)
            c["aliases"] = fresh
        p.update(clean)
    common.write_json(ds / "world/world.json", {"seed": cfg.seed, "projects": projects})
    facts = sum(len(p["candidate_facts"]) for p in projects)
    pairs = sum(1 for p in projects for f in p["candidate_facts"] if f["replaces"])
    common.out(f"world: {len(projects)} projects, {facts} candidate facts, {pairs} revision pairs")
    return 0
