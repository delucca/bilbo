"""Digest prompts: positives from a note's facts, noise, off-topic and near-miss negatives, in batches of 10."""

from __future__ import annotations

import random
from dataclasses import dataclass
from pathlib import Path
from string import Template

from bilbo_evals import common, schema
from bilbo_evals.common import CANARY, Refused
from bilbo_evals.generate import queries as q

STEP = "prompts"
BATCH = 10
MAX_ATTEMPTS = 3
LABELS = ["positive", "noise", "off-topic", "near-miss"]
# ids are positional inside a fixed block, so a resumed or repeated run always names a prompt the same way
PROJECT_BASE = {"positive": 0, "near-miss": 100}
NONE_BASE = {("noise", "dev"): 0, ("noise", "test"): 200, ("off-topic", "dev"): 400, ("off-topic", "test"): 600}
BLOCK = 100
NONE_BLOCK = 200
NOISE_HINTS = ["terse and lowercase", "polite and wordy", "impatient", "approving", "uncertain", "with a typo", "a single word"]
OFF_TOPIC_HINTS = ["cooking and food", "travel planning", "personal finance", "writing and email", "data science scripts", "shell one-liners", "other programming languages", "health and fitness"]


@dataclass
class Batch:
    key: str
    split: str
    label: str
    project: str | None
    start: int
    n: int
    lang: str
    slots: dict[str, str]
    gold: list[list[str]]


def allocate(total: int, caps: dict[str, int | None], rng) -> dict[str, int]:
    """Share `total` round-robin over keys in a seeded order, never beyond a key's cap (None: unlimited)."""
    order = sorted(caps)
    rng.shuffle(order)
    got = {k: 0 for k in order}
    while total > 0:
        moved = False
        for k in order:
            if total > 0 and (caps[k] is None or got[k] < caps[k]):
                got[k] += 1
                total -= 1
                moved = True
        if not moved:
            break
    return got


def _pt_share(w: q.World, projects: list[str]) -> float:
    notes = [n for n in w.notes.values() if n["project"] in projects and n.get("status") == "kept"]
    return sum(n["lang"] == "pt" for n in notes) / len(notes) if notes else 0.0


def _project_slots(w: q.World, slug: str, facts: list[dict], rng) -> dict[str, str]:
    p = w.projects[slug]
    mine = [f["statement"] for f in facts if f["project"] == slug]
    rng.shuffle(mine)
    return {
        "project_name": p["name"], "project_summary": p["summary"],
        "technologies": ", ".join(p.get("technologies", [])),
        "components": ", ".join(c["name"] for c in p.get("components", [])),
        "facts": "\n".join(f"- {s}" for s in mine[:30]) or "- (none)",
    }


def plan_batches(w: q.World, split: str, counts: dict[str, int], seed: int) -> tuple[list[Batch], dict[str, int]]:
    """Batches of at most 10 prompts per (split, label, project) and the shortfall per label."""
    projects = w.split_projects(split)
    facts = sorted((f for f in w.facts.values() if f["project"] in projects and w.usable(f)), key=lambda f: f["id"])
    pt = _pt_share(w, projects)
    out: list[Batch] = []
    short: dict[str, int] = {}
    for label in LABELS:
        want = counts.get(label, 0)
        rng = random.Random(f"{seed}:{split}:prompts:{label}")
        if label in ("noise", "off-topic"):
            base = NONE_BASE[(label, split)]
            if want > NONE_BLOCK:
                raise Refused(f"{label}: at most {NONE_BLOCK} prompts per split")
            hints = NOISE_HINTS if label == "noise" else OFF_TOPIC_HINTS
            for k, start in enumerate(range(0, want, BATCH)):
                n = min(BATCH, want - start)
                lang = "pt" if rng.random() < pt else "en"
                out.append(Batch(f"{split}-{label}-none-{k + 1:02d}", split, label, None, base + start, n, lang, {"hint": hints[k % len(hints)]}, [[] for _ in range(n)]))
            continue
        if label == "positive":
            notes = {p: _positive_notes(w, p, facts, rng) for p in projects}
            share = allocate(want, {p: len(v) for p, v in notes.items()}, rng)
        else:
            share = allocate(want, {p: None for p in projects}, rng)
        if sum(share.values()) < want:
            short[label] = want - sum(share.values())
        for p in projects:
            if share.get(p, 0) > BLOCK - 1:
                raise Refused(f"{label}: at most {BLOCK - 1} prompts per project")
            for k, start in enumerate(range(0, share.get(p, 0), BATCH)):
                n = min(BATCH, share[p] - start)
                slots = _project_slots(w, p, facts, rng)
                if label == "positive":
                    chosen = notes[p][start:start + n]
                    slots["notes"] = "\n".join(f"{i + 1}. [{w.notes[nid]['kind']}] " + "; ".join(st) for i, (nid, st) in enumerate(chosen))
                    gold = [[nid] for nid, _ in chosen]
                    lang = w.notes[chosen[0][0]]["lang"]
                else:
                    slots["hint"] = q.ANGLES[(k + rng.randrange(len(q.ANGLES))) % len(q.ANGLES)]
                    gold = [[] for _ in range(n)]
                    lang = "pt" if rng.random() < pt else "en"
                out.append(Batch(f"{split}-{label}-{p}-{k + 1:02d}", split, label, p, PROJECT_BASE[label] + start, n, lang, slots, gold))
    return out, short


def _positive_notes(w: q.World, project: str, facts: list[dict], rng) -> list[tuple[str, list[str]]]:
    by_note: dict[str, list[str]] = {}
    for f in facts:
        if f["project"] == project and not f.get("bridge"):
            by_note.setdefault(f["note_id"], []).append(f["statement"])
    items = sorted(by_note.items())
    rng.shuffle(items)
    return items


def build_prompt(b: Batch, templates: dict[str, Template]) -> str:
    fields = {"item": b.key, "n": str(b.n), "lang_name": q.LANG_NAMES[b.lang], **b.slots}
    return (templates["common"].substitute(fields) + "\n\n" + templates[b.label].substitute(fields)).strip() + "\n"


def prompt_id(b: Batch, i: int) -> str:
    return f"p-{b.project or 'none'}-{b.start + i + 1:03d}"


def output_problem(b: Batch, out: dict) -> str | None:
    items = [s for s in out.get("prompts", []) if isinstance(s, str) and s.strip()]
    if b.label == "positive" and len(out.get("prompts", [])) != b.n:
        return f"{len(out.get('prompts', []))} prompts for {b.n} notes"
    return None if len(items) >= b.n else f"{len(items)} prompts, {b.n} asked"


def make_rows(b: Batch, out: dict) -> list[dict]:
    texts = [s.strip() for s in out["prompts"]][: b.n] if b.label == "positive" else [s.strip() for s in out["prompts"] if s.strip()][: b.n]
    rows, seen = [], set()
    for i, text in enumerate(texts):
        if not text or text.lower() in seen:
            continue
        seen.add(text.lower())
        row = {
            "id": prompt_id(b, i), "prompt": text, "split": b.split, "label": b.label,
            "gold": b.gold[i], "project": b.project, "canary": CANARY,
        }
        problems = schema.check("prompt", row)
        if problems:
            raise Refused(f"prompt {row['id']} is malformed: {problems[0]}")
        rows.append(row)
    return rows


def cmd(args) -> int:
    from bilbo_evals import dataset, llm

    ds = Path(args.dataset)
    cfg = llm.load_config(ds)
    split = args.split
    llm.require_cli("codex")
    w = q.load_world(ds)
    batches, short = plan_batches(w, split, q.prompt_counts(cfg, split), cfg.seed)
    for label, n in sorted(short.items()):
        common.err(f"{split} {label}: the notes allow {n} fewer prompts than asked")

    templates = q.load_sections("prompts.md")
    prompts_schema = q._json(q.HERE / "schemas/prompts.json")
    path = ds / "digest/prompts.jsonl"
    existing = {r["id"]: r for r in (common.read_jsonl(path) if path.exists() else [])}
    settled = q.settled_items(ds)
    preflighted = False
    stop: Exception | None = None

    for _ in range(MAX_ATTEMPTS):
        calls = []
        for b in batches:
            outs = q.attempt_outputs(ds, STEP, b.key)
            if outs and not output_problem(b, outs[max(outs)]):
                continue
            nxt = max(outs, default=0) + 1
            if nxt <= MAX_ATTEMPTS and b.key not in settled:
                calls.append(llm.Call(STEP, b.key, "codex", build_prompt(b, templates), prompts_schema, nxt))
        if not calls:
            break
        if not preflighted:
            llm.preflight("codex", ds, cfg, STEP)
            preflighted = True
        results = llm.run_many(calls, ds, cfg)
        stop = llm.fatal(results)
        if stop or llm.failed(results):
            break

    left = 0
    for b in batches:
        if b.key in settled:
            continue
        outs = q.attempt_outputs(ds, STEP, b.key)
        good = next((outs[a] for a in sorted(outs) if not output_problem(b, outs[a])), None)
        if good is None:
            left += 1
            if len(outs) >= MAX_ATTEMPTS:
                common.append_jsonl(ds / "generation/drops.jsonl", {"item": b.key, "reason": "invalid-output", "stratum": b.label})
            continue
        for row in make_rows(b, good):
            if row["id"] not in existing and row["id"] not in settled:
                existing[row["id"]] = row
    common.write_jsonl(path, [existing[i] for i in sorted(existing)])
    dataset.build_qrels(ds)
    made = {lab: sum(1 for r in existing.values() if r["split"] == split and r["label"] == lab) for lab in LABELS}
    common.out(f"{split}: " + ", ".join(f"{lab} {n}" for lab, n in made.items() if n))
    if stop is not None:
        common.err(f"{left} batches left")
        raise stop
    if left:
        common.err(f"{left} batches left")
        return 1
    return 0
