"""Queries: an intent per stratum built from facts, worded by the query model; blind strata never see a note."""

from __future__ import annotations

import random
import re
import unicodedata
from dataclasses import dataclass, field
from pathlib import Path
from string import Template

from bilbo_evals import common, schema
from bilbo_evals.common import CANARY, Refused

HERE = Path(__file__).resolve().parent
STEP = "queries"
# Reasons `generate filter` rejects a query for; the query is worded again, up to MAX_ATTEMPTS.
REWRITE = ("leakage", "alias")
MAX_ATTEMPTS = 3
LANG_NAMES = {"en": "English", "pt": "Brazilian Portuguese"}
# Order the pools are drawn in: scarce structure first, so the open strata cannot starve it.
ORDER = ["alias", "supersession", "multi-hop", "kind-filter", "pt-en", "paraphrase", "known-item", "no-answer", "library"]
# The query model sees the fact and the alias table for these, never a note: Intent has no note field for them.
BLIND = frozenset({"paraphrase", "pt-en", "alias"})
LIBRARY_BASE = {"dev": 0, "test": 200}
ANGLES = [
    "deployment and rollout", "monitoring and alerting", "cost and capacity", "security and access",
    "onboarding a new teammate", "testing strategy", "performance tuning", "backups and recovery",
    "licensing and compliance", "upgrade and migration plans", "team ownership", "incident history",
]


@dataclass
class Intent:
    """What one query must be; `slots` is everything the query model is shown besides the common rules."""

    stratum: str
    split: str
    project: str | None
    family: str
    lang: str
    gold: list[str]
    slots: dict[str, str]
    evidence_sets: list[list[str]] = field(default_factory=list)
    decoys: list[str] = field(default_factory=list)
    kind: str | None = None
    fact_ids: list[str] = field(default_factory=list)
    gold_heading: str | None = None
    id: str = ""


@dataclass
class World:
    ds: Path
    projects: dict[str, dict]
    facts: dict[str, dict]
    notes: dict[str, dict]
    aliases: list[dict]
    splits: dict

    def split_projects(self, split: str) -> list[str]:
        return sorted(p for p in self.splits.get(split, []) if p in self.projects)

    def note_path(self, note_id: str) -> Path:
        return self.ds / "store/notes" / self.notes[note_id]["file"]

    def title(self, note_id: str) -> str:
        for line in self.note_path(note_id).read_text(encoding="utf-8").splitlines():
            if line.startswith("# "):
                return line[2:].strip()
        return self.notes[note_id]["topic"]

    def note_has_word(self, note_id: str, term: str) -> bool:
        return _has_word(self.note_path(note_id).read_text(encoding="utf-8"), term)

    def usable(self, fact: dict) -> bool:
        note = self.notes.get(fact.get("note_id") or "")
        return fact.get("status") == "planted" and note is not None and note.get("status") == "kept"

    def fact_lang(self, fact: dict) -> str:
        return self.notes[fact["note_id"]]["lang"]

    def alias_table(self, project: str) -> str:
        rows = [a for a in self.aliases if a["project"] == project]
        by_canon: dict[str, list[str]] = {}
        for a in rows:
            by_canon.setdefault(a["canonical"], []).append(f"{a['alias']} ({a['type']})")
        return "\n".join(f"- {c}: also called {', '.join(v)}" for c, v in sorted(by_canon.items())) or "- (none)"

    def component_name(self, project: str, slug: str) -> str:
        for c in self.projects[project].get("components", []):
            if c["slug"] == slug:
                return c["name"]
        return slug


def _fold(text: str) -> str:
    return "".join(c for c in unicodedata.normalize("NFD", text.lower()) if not unicodedata.combining(c))


def _has_word(text: str, term: str) -> bool:
    return re.search(rf"(?<![a-z0-9]){re.escape(_fold(term))}(?![a-z0-9])", _fold(text)) is not None


def load_world(ds: Path) -> World:
    w = ds / "world"
    try:
        world = _json(w / "world.json")
        facts = common.read_jsonl(w / "facts.jsonl")
        notes = common.read_jsonl(w / "notes.jsonl")
        aliases = _json(w / "aliases.json")
        splits = _json(w / "splits.json")
    except FileNotFoundError as e:
        raise Refused(f"{e.filename} is missing: run the earlier generate steps first") from e
    return World(
        ds, {p["slug"]: p for p in world["projects"]}, {f["id"]: f for f in facts},
        {n["id"]: n for n in notes}, aliases, splits,
    )


def _json(p: Path):
    import json

    return json.loads(p.read_text(encoding="utf-8"))


def load_sections(name: str) -> dict[str, Template]:
    text = (HERE / "templates" / name).read_text(encoding="utf-8")
    parts = re.split(r"^## (\S+)\n", text, flags=re.M)
    return {parts[i]: Template(parts[i + 1].strip("\n")) for i in range(1, len(parts), 2)}


def split_counts(ds: Path, cfg, split: str) -> dict[str, int]:
    """Dev counts come from the config, test counts from preregistration.json."""
    if split == "dev":
        return {k: int(v) for k, v in cfg.strata["dev"].items()}
    pre = ds / "preregistration.json"
    if not pre.exists():
        raise Refused("preregistration.json is missing: size the test split with bilbo-evals power first")
    per = _json(pre).get("per_stratum", {})
    return {k: int(v) for k, v in per.items() if k in ORDER}


def prompt_counts(cfg, split: str) -> dict[str, int]:
    block = cfg.prompts.get(split) or cfg.prompts["dev"]
    return {k: int(v) for k, v in block.items()}


# ---- intents -------------------------------------------------------------------------------------------------


def build_intents(w: World, split: str, counts: dict[str, int], seed: int) -> tuple[list[Intent], dict[str, int]]:
    """Intents for one split with ids assigned, and the shortfall per stratum (asked minus built)."""
    projects = set(w.split_projects(split))
    facts = sorted((f for f in w.facts.values() if f["project"] in projects and w.usable(f)), key=lambda f: f["id"])
    out: list[Intent] = []
    short: dict[str, int] = {}
    for stratum in ORDER:
        want = counts.get(stratum, 0)
        if want <= 0:
            continue
        rng = random.Random(f"{seed}:{split}:{stratum}")
        used: set[str] = set()  # a fact serves one query per stratum, not one per split
        if stratum == "library":
            made = _library(w, split, want, rng)
        elif stratum == "no-answer":
            made = _no_answer(w, split, want, rng, projects, facts)
        else:
            pool = list(facts)
            rng.shuffle(pool)
            made = _from_facts(w, split, stratum, want, pool, used, rng)
        out += made
        if len(made) < want:
            short[stratum] = want - len(made)
    _assign_ids(out)
    return out, short


def _base(w: World, f: dict, split: str, stratum: str) -> dict:
    p = w.projects[f["project"]]
    return {
        "stratum": stratum, "split": split, "project": f["project"], "family": f["family"],
        "slots": {
            "project_name": p["name"], "project_summary": p["summary"],
            "component": w.component_name(f["project"], f["component"]), "statement": f["statement"],
            "aliases": w.alias_table(f["project"]),
        },
    }


def _from_facts(w: World, split: str, stratum: str, want: int, pool: list[dict], used: set[str], rng) -> list[Intent]:
    out: list[Intent] = []
    # A fact that is replaced, or only bridges an alias, would make the gold note ambiguous for the single-fact strata.
    single = [f for f in pool if not f.get("superseded_by") and not f.get("bridge")]
    if stratum == "alias":
        out = _alias(w, split, want, single, used, rng)
    elif stratum == "supersession":
        for f in pool:
            old = w.facts.get(f.get("supersedes") or "")
            if old is None or not w.usable(old) or f["id"] in used or old["id"] in used:
                continue
            if old["note_id"] == f["note_id"]:
                continue
            used |= {f["id"], old["id"]}
            b = _base(w, f, split, stratum)
            b["slots"]["older"] = old["statement"]
            out.append(Intent(**b, lang=w.fact_lang(f), gold=[f["note_id"]], decoys=[old["note_id"]], fact_ids=[f["id"], old["id"]]))
            if len(out) == want:
                break
    elif stratum == "multi-hop":
        # Two-note chains only: a third note answers no part of the question.
        seen: set[frozenset] = set()
        for f in pool:
            for j in sorted(f.get("joins", [])):
                ids = sorted({f["id"], j})
                other = w.facts.get(j)
                key = frozenset(ids)
                if len(ids) != 2 or key in seen or other is None or not w.usable(other) or f["id"] in used or j in used:
                    continue
                notes = list(dict.fromkeys(g["note_id"] for g in (w.facts[i] for i in ids)))
                if len(notes) != 2 or other["project"] != f["project"]:
                    continue
                seen.add(key)
                used |= key
                first = w.facts[ids[0]]
                b = _base(w, first, split, stratum)
                b["slots"]["facts"] = "\n".join(f"- {w.facts[i]['statement']}" for i in ids)
                out.append(Intent(**b, lang=w.fact_lang(first), gold=notes, evidence_sets=[notes], fact_ids=ids))
                break
            if len(out) == want:
                break
    elif stratum == "kind-filter":
        seen = set()
        for f in pool:
            mate = w.facts.get(f.get("kind_pair") or "")
            key = frozenset([f["id"], f.get("kind_pair") or ""])
            if mate is None or not w.usable(mate) or key in seen or f["id"] in used or mate["id"] in used:
                continue
            seen.add(key)
            gold, other = (f, mate) if rng.random() < 0.5 else (mate, f)
            kind = w.notes[gold["note_id"]]["kind"]
            if kind == w.notes[other["note_id"]]["kind"] or gold["note_id"] == other["note_id"]:
                continue
            used |= key
            b = _base(w, gold, split, stratum)
            b["slots"]["kind"] = kind
            out.append(Intent(**b, lang=w.fact_lang(gold), gold=[gold["note_id"]], decoys=[other["note_id"]], kind=kind, fact_ids=[gold["id"]]))
            if len(out) == want:
                break
    else:
        for f in single:
            if f["id"] in used:
                continue
            used.add(f["id"])
            b = _base(w, f, split, stratum)
            lang = w.fact_lang(f)
            if stratum == "pt-en":
                b["slots"]["fact_lang_name"] = LANG_NAMES[lang]
                lang = "pt" if lang == "en" else "en"
            elif stratum == "known-item":
                b["slots"]["title"] = w.title(f["note_id"])
            out.append(Intent(**b, lang=lang, gold=[f["note_id"]], fact_ids=[f["id"]]))
            if len(out) == want:
                break
    return out


def _alias(w: World, split: str, want: int, single: list[dict], used: set[str], rng) -> list[Intent]:
    """Pairs of (alias row, fact): the gold note never says the alias, another note says both names."""
    pairs = []
    for a in w.aliases:
        if not a.get("bridge_notes") or a["project"] not in w.split_projects(split):
            continue
        for f in single:
            if f["project"] == a["project"] and f["component"] == a["component"] and f["note_id"] not in a["bridge_notes"]:
                if not w.note_has_word(f["note_id"], a["alias"]):
                    pairs.append((a, f))
    rng.shuffle(pairs)
    out = []
    for a, f in pairs:
        if f["id"] in used:
            continue
        used.add(f["id"])
        b = _base(w, f, split, "alias")
        b["slots"].update(alias=a["alias"], alias_type=a["type"], component=a["canonical"])
        out.append(Intent(**b, lang=w.fact_lang(f), gold=[f["note_id"]], fact_ids=[f["id"]]))
        if len(out) == want:
            break
    return out


def _no_answer(w: World, split: str, want: int, rng, projects: set[str], facts: list[dict]) -> list[Intent]:
    slugs = sorted(projects)
    out = []
    if not slugs:
        return out
    order = list(slugs)
    rng.shuffle(order)
    for i in range(want):
        slug = order[i % len(order)]
        p = w.projects[slug]
        mine = [f["statement"] for f in facts if f["project"] == slug]
        rng.shuffle(mine)
        lang = rng.choice([w.fact_lang(f) for f in facts if f["project"] == slug] or ["en"])
        out.append(Intent(
            stratum="no-answer", split=split, project=slug, family=f"fam-{slug}-na", lang=lang, gold=[],
            slots={
                "project_name": p["name"], "project_summary": p["summary"],
                "technologies": ", ".join(p.get("technologies", [])),
                "components": ", ".join(c["name"] for c in p.get("components", [])),
                "angle": ANGLES[(i + rng.randrange(len(ANGLES))) % len(ANGLES)],
                "facts": "\n".join(f"- {s}" for s in mine[:40]) or "- (none)",
            },
        ))
    return out


def sections(markdown: str) -> list[tuple[list[str], str]]:
    """(heading path, own text) per section of a source body; frontmatter and fenced code headings are skipped."""
    body = re.sub(r"\A---\n.*?\n---\n", "", markdown, flags=re.S)
    path: list[tuple[int, str]] = []
    out: list[tuple[list[str], list[str]]] = []
    fenced = False
    for line in body.splitlines():
        if line.startswith("```"):
            fenced = not fenced
        m = None if fenced else re.match(r"^(#{1,6})\s+(.*\S)\s*$", line)
        if m:
            level = len(m.group(1))
            path = [(l, t) for l, t in path if l < level] + [(level, m.group(2))]
            out.append(([t for _, t in path], []))
        elif out:
            out[-1][1].append(line)
    return [(p, "\n".join(t).strip()) for p, t in out]


def _library(w: World, split: str, want: int, rng) -> list[Intent]:
    refs = sorted(w.splits.get("library", {}).get(split, []))
    per_source: dict[str, list[tuple[list[str], str]]] = {}
    for ref in refs:
        path = w.ds / "store/library" / f"{ref}.md"
        if not path.exists():
            continue
        secs = [(p, t) for p, t in sections(path.read_text(encoding="utf-8")) if len(t) >= 300]
        rng.shuffle(secs)
        per_source[ref] = secs
    out = []
    for rnd in range(max((len(v) for v in per_source.values()), default=0)):
        for ref in sorted(per_source):
            if rnd < len(per_source[ref]) and len(out) < want:
                heading, text = per_source[ref][rnd]
                name = ref.split("/", 1)[1]
                out.append(Intent(
                    stratum="library", split=split, project=None, family=f"fam-lib-{name}", lang="en", gold=[ref],
                    gold_heading=" > ".join(heading),
                    slots={"heading": " > ".join(heading), "text": text[:2500]},
                ))
    return out


def _assign_ids(intents: list[Intent]) -> None:
    n: dict[str, int] = {}
    for it in intents:
        if it.stratum == "library":
            key = "lib"
            k = n[key] = n.get(key, LIBRARY_BASE[it.split]) + 1
            if k >= LIBRARY_BASE[it.split] + 200:
                raise Refused("more than 199 library queries in one split")
            it.id = f"q-lib-{k:03d}"
        elif it.stratum == "no-answer":
            key = f"na-{it.project}"
            n[key] = n.get(key, 0) + 1
            it.id = f"q-{it.project}-na-{n[key]:02d}"
        else:
            n[it.project] = n.get(it.project, 0) + 1
            it.id = f"q-{it.project}-{n[it.project]:03d}"


# ---- prompts to the query model ------------------------------------------------------------------------------


def build_prompt(it: Intent, templates: dict[str, Template], avoid: list[str] | None = None) -> str:
    fields = {"item": it.id, "lang": it.lang, "lang_name": LANG_NAMES[it.lang], **it.slots}
    text = templates["common"].substitute(fields) + "\n\n" + templates[it.stratum].substitute(fields)
    if avoid:
        text += "\n\n" + templates["avoid"].substitute(tokens=", ".join(sorted(avoid)))
    return text.strip() + "\n"


# ---- resumption ----------------------------------------------------------------------------------------------


def settled_items(ds: Path) -> set[str]:
    """Items another step already decided about: dropped (not for leakage) or applied by a review or a pool."""
    out: set[str] = set()
    for rel in ("generation/review/applied.jsonl", "generation/pool/applied.jsonl"):
        if (ds / rel).exists():
            out |= {r["item"] for r in common.read_jsonl(ds / rel) if "item" in r}
    return out | {r["item"] for r in _drops(ds) if r.get("reason") not in REWRITE}


def _drops(ds: Path) -> list[dict]:
    p = ds / "generation/drops.jsonl"
    return common.read_jsonl(p) if p.exists() else []


def attempt_outputs(ds: Path, step: str, item: str) -> dict[int, dict]:
    import json

    out = {}
    for p in sorted((ds / "generation/outputs" / step).glob(f"{item}.*.json")):
        m = re.fullmatch(re.escape(item) + r"\.(\d+)\.json", p.name)
        if m:
            out[int(m.group(1))] = json.loads(p.read_text(encoding="utf-8"))
    return out


def output_problem(it: Intent, out: dict) -> str | None:
    if not str(out.get("query", "")).strip():
        return "empty query"
    if out.get("lang") != it.lang:
        return f"language {out.get('lang')!r}, asked {it.lang!r}"
    if it.stratum == "multi-hop" and str(out.get("hop_link", "")).strip().upper() in ("", "NONE"):
        return "stitched multi-hop: no chain between the hops"
    return None


def rejections(ds: Path, item: str) -> list[dict]:
    return [d for d in _drops(ds) if d["item"] == item and d.get("reason") in REWRITE]


def next_step(it: Intent, attempts: dict[int, dict], rejected: dict[int, str]) -> tuple[str, int | str]:
    """What an item with no row needs: ("call", attempt), ("row", attempt) or ("drop", reason).

    `rejected` maps an attempt to the reason the filter rejected it.
    """
    if not attempts:
        return "call", 1
    last = max(attempts)
    if output_problem(it, attempts[last]):
        return ("call", last + 1) if last < MAX_ATTEMPTS else ("drop", "invalid-output")
    if last in rejected:
        return ("call", last + 1) if last < MAX_ATTEMPTS else ("drop", f"{rejected[last]}-exhausted")
    return "row", last


def make_row(it: Intent, out: dict, gen: dict) -> dict:
    row = {
        "id": it.id, "text": out["query"].strip(), "stratum": it.stratum, "split": it.split, "lang": it.lang,
        "project": it.project, "family": it.family, "gold": it.gold, "evidence_sets": it.evidence_sets,
        "decoys": it.decoys, "kind": it.kind, "fact_ids": it.fact_ids, "zero_overlap": None,
        "gold_heading": it.gold_heading, "gen": gen, "canary": CANARY,
    }
    problems = schema.check("query", row)
    if problems:
        raise Refused(f"query {it.id} is malformed: {problems[0]}")
    return row


def _gen(ds: Path, cfg, call_id: str, prompt: str) -> dict:
    p = ds / "generation/calls.jsonl"
    rec = next((r for r in reversed(common.read_jsonl(p)) if r.get("call_id") == call_id and r.get("status") == "ok"), None) if p.exists() else None
    return {
        "cli": (rec or {}).get("cli", "codex"), "model": (rec or {}).get("model", cfg.query_model),
        "prompt_sha256": (rec or {}).get("prompt_sha256") or common.sha256_bytes(prompt.encode("utf-8")),
        "call_id": call_id,
    }


# ---- the step ------------------------------------------------------------------------------------------------


TOPUP_MAX_ROUNDS = 3


def _taken_ids(ds: Path, base: list[Intent]) -> set[str]:
    """Every id ever used in the dataset: never reused, dropped ones included."""
    out = {it.id for it in base}
    for rel in ("queries.jsonl", "generation/drops.jsonl", "generation/topup.jsonl"):
        if (ds / rel).exists():
            out |= {r["id" if rel == "queries.jsonl" else "item"] for r in common.read_jsonl(ds / rel)}
    for p in (ds / "generation/outputs" / STEP).glob("*.json"):
        out.add(p.name.split(".")[0])
    return out


def _next_id(taken: set[str], it: Intent) -> str:
    if it.stratum == "library":
        lo = LIBRARY_BASE[it.split]
        top = max([int(m.group(1)) for t in taken if (m := re.fullmatch(r"q-lib-(\d{3})", t)) and lo <= int(m.group(1)) < lo + 200] + [lo])
        return f"q-lib-{top + 1:03d}"
    if it.stratum == "no-answer":
        top = max([int(m.group(1)) for t in taken if (m := re.fullmatch(rf"q-{it.project}-na-(\d+)", t))] + [0])
        return f"q-{it.project}-na-{top + 1:02d}"
    top = max([int(m.group(1)) for t in taken if (m := re.fullmatch(rf"q-{it.project}-(\d{{3}})", t))] + [0])
    return f"q-{it.project}-{top + 1:03d}"


def extras_for(ds: Path, w: World, split: str, counts: dict[str, int], seed: int, base: list[Intent], need: dict[str, int] | None = None) -> tuple[list[Intent], dict[str, int]]:
    """Top-up intents already registered in generation/topup.jsonl, plus `need` new ones per stratum.

    The pool of a stratum is drawn with the base's own seed and order, so intent `want + k` uses facts the
    first `want` never used; each extra gets a fresh id, recorded so a resumed run names it the same way.
    """
    path = ds / "generation/topup.jsonl"
    reg = [r for r in (common.read_jsonl(path) if path.exists() else []) if r["split"] == split]
    need = {s: n for s, n in (need or {}).items() if n > 0}
    if not reg and not need:
        return [], {}
    wide = {s: counts.get(s, 0) + sum(1 for r in reg if r["stratum"] == s) + need.get(s, 0) for s in set(counts) | set(need)}
    built, _ = build_intents(w, split, wide, seed)
    taken = _taken_ids(ds, base)
    out: list[Intent] = []
    short: dict[str, int] = {}
    for s in ORDER:
        pool = [i for i in built if i.stratum == s][counts.get(s, 0):]
        mine = sorted((r for r in reg if r["stratum"] == s), key=lambda r: r["index"])
        for r in mine:
            if r["index"] < len(pool):
                pool[r["index"]].id = r["item"]
                out.append(pool[r["index"]])
        fresh = pool[len(mine):][: need.get(s, 0)]
        if len(fresh) < need.get(s, 0):
            short[s] = need[s] - len(fresh)
        for k, it in enumerate(fresh, start=len(mine)):
            it.id = _next_id(taken, it)
            taken.add(it.id)
            common.append_jsonl(path, {"item": it.id, "split": split, "stratum": s, "index": k})
            out.append(it)
    return out, short


def _pass(ds: Path, cfg, intents: list[Intent], rows: dict[str, dict], templates, query_schema, flags: dict) -> tuple[Exception | None, int]:
    """Generate, read back and row every intent that can be; returns (stop, items left)."""
    from bilbo_evals import llm

    rej = {it.id: rejections(ds, it.id) for it in intents}
    left = 0
    stop: Exception | None = None
    for _ in range(MAX_ATTEMPTS + 1):
        settled = settled_items(ds)
        calls = []
        for it in intents:
            if it.id in rows or it.id in settled:
                continue
            what, arg = next_step(it, attempt_outputs(ds, STEP, it.id), {d.get("attempt", 1): d["reason"] for d in rej[it.id]})
            if what == "call":
                avoid = sorted({t for d in rej[it.id] for t in d.get("tokens", [])})
                calls.append(llm.Call(STEP, it.id, "codex", build_prompt(it, templates, avoid), query_schema, arg))
            elif what == "drop":
                common.append_jsonl(ds / "generation/drops.jsonl", {"item": it.id, "reason": arg, "stratum": it.stratum})
                common.err(f"{it.id} dropped: {arg}")
            else:
                call_id = f"{STEP}/{it.id}/{arg}"
                out = attempt_outputs(ds, STEP, it.id)[arg]
                rows[it.id] = make_row(it, out, _gen(ds, cfg, call_id, build_prompt(it, templates)))
        if not calls:
            break
        if not flags.get("preflighted"):
            llm.preflight("codex", ds, cfg, STEP)
            flags["preflighted"] = True
        results = llm.run_many(calls, ds, cfg)
        stop = llm.fatal(results)
        if stop or llm.failed(results):
            break

    # Rows for outputs that landed in the last round (the loop above ends before it reads them).
    settled = settled_items(ds)
    for it in intents:
        if it.id in rows or it.id in settled:
            continue
        outs = attempt_outputs(ds, STEP, it.id)
        what, arg = next_step(it, outs, {d.get("attempt", 1): d["reason"] for d in rej[it.id]})
        if what == "row":
            rows[it.id] = make_row(it, outs[arg], _gen(ds, cfg, f"{STEP}/{it.id}/{arg}", build_prompt(it, templates)))
        elif what == "drop":
            common.append_jsonl(ds / "generation/drops.jsonl", {"item": it.id, "reason": arg, "stratum": it.stratum})
        else:
            left += 1
    return stop, left


def _short(ds: Path, intents: list[Intent], rows: dict[str, dict], counts: dict[str, int]) -> dict[str, int]:
    """Per stratum, how far the intents that are not dropped for good (no row, settled) fall below the target."""
    settled = settled_items(ds)
    alive: dict[str, int] = {}
    for it in intents:
        if it.id in rows or it.id not in settled:
            alive[it.stratum] = alive.get(it.stratum, 0) + 1
    return {s: n - alive.get(s, 0) for s, n in counts.items() if n > alive.get(s, 0)}


def wide_multihop(ds: Path) -> dict[str, list[str]]:
    """Per split, the multi-hop queries whose evidence sets hold more than two notes."""
    path = ds / "queries.jsonl"
    out: dict[str, list[str]] = {}
    for r in common.read_jsonl(path) if path.exists() else []:
        if r["stratum"] == "multi-hop" and any(len(s) > 2 for s in r["evidence_sets"]):
            out.setdefault(r["split"], []).append(r["id"])
    return out


def cmd(args) -> int:
    from bilbo_evals import dataset, llm

    ds = Path(args.dataset)
    if getattr(args, "list_wide_multihop", False):
        for split, ids in sorted(wide_multihop(ds).items()):
            common.out(f"{split}: " + " ".join(sorted(ids)))
        return 0
    if not getattr(args, "split", None):
        raise common.UsageError("--split is required")
    cfg = llm.load_config(ds)
    split = args.split
    llm.require_cli("codex")
    world = load_world(ds)
    counts = split_counts(ds, cfg, split)
    base, short = build_intents(world, split, counts, cfg.seed)
    for stratum, n in sorted(short.items()):
        common.err(f"{split} {stratum}: the facts allow {n} fewer queries than asked")

    templates = load_sections("queries.md")
    query_schema = _json(HERE / "schemas/query.json")
    qpath = ds / "queries.jsonl"
    rows = {r["id"]: r for r in (common.read_jsonl(qpath) if qpath.exists() else [])}
    flags: dict = {}
    extras, _ = extras_for(ds, world, split, counts, cfg.seed, base)
    topup_short: dict[str, int] = {}
    for round_ in range(TOPUP_MAX_ROUNDS + 1):
        intents = base + extras
        stop, left = _pass(ds, cfg, intents, rows, templates, query_schema, flags)
        if stop is not None or left or round_ == TOPUP_MAX_ROUNDS:
            break
        # A stratum below its target after drops gets fresh intents, from facts it has not used.
        need = _short(ds, intents, rows, counts)
        if not need:
            break
        more, topup_short = extras_for(ds, world, split, counts, cfg.seed, base, need)
        if not more:
            break
        extras += more

    common.write_jsonl(qpath, [rows[i] for i in sorted(rows)])
    dataset.build_qrels(ds)
    made = {s: sum(1 for r in rows.values() if r["split"] == split and r["stratum"] == s) for s in ORDER}
    common.out(f"{split}: " + ", ".join(f"{s} {n}" for s, n in made.items() if n))
    for s, n in sorted(topup_short.items()):
        common.err(f"{split} {s}: no unused facts left for {n} top-up queries")
    if stop is not None:
        common.err(f"{left} queries left")
        raise stop
    if left:
        common.err(f"{left} queries left")
        return 1
    missing = sum(max(0, counts.get(s, 0) - n) for s, n in made.items() if counts.get(s, 0))
    if missing:
        common.err(f"{missing} queries short of the asked counts")
        return 1
    return 0
