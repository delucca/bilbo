"""`generate fidelity`: check every fact against its rendered note; re-render twice, then drop the fact and log it."""

from __future__ import annotations

import json
from pathlib import Path

from bilbo_evals import common, llm
from bilbo_evals.generate import notes as notes_mod
from bilbo_evals.generate import schema, template

STEP = "fidelity"
MAX_ATTEMPTS = 3


def norm(text: str) -> str:
    return " ".join(text.replace("`", "").split())


def body_text(ds: Path, note: dict) -> str:
    text = (Path(ds) / "store/notes" / note["file"]).read_text(encoding="utf-8")
    return text.split("\n---\n", 1)[1] if text.startswith("---\n") and "\n---\n" in text else text


def live(w: notes_mod.World, note: dict) -> list[dict]:
    return [w.facts[i] for i in note["facts"] if w.facts[i]["status"] == "planted"]


def missing_strings(text: str, fact: dict) -> list[str]:
    return [v for v in fact["verbatim"] if v not in text]


def check_prompt(text: str, facts: list[dict]) -> str:
    lines = "\n".join(f"- id: {f['id']}\n  fact: {f['statement']}" for f in facts)
    return template("fidelity.md").substitute(note=text.strip(), facts=lines)


def unreadable(text: str, facts: list[dict], answer: dict) -> dict[str, str]:
    """Fact id -> why the checking model's answer does not confirm it."""
    by_id = {r["id"]: r for r in answer["facts"]}
    plain = norm(text)
    bad = {}
    for f in facts:
        r = by_id.get(f["id"])
        if r is None:
            bad[f["id"]] = "the checker gave no answer for it"
        elif not r["readable"]:
            bad[f["id"]] = "the checker could not read it from the note"
        elif not r["evidence"].strip() or norm(r["evidence"]) not in plain:
            bad[f["id"]] = "the checker's evidence is not a quote of the note"
    return bad


def feedback(f: dict, missing: list[str], why: str | None) -> list[str]:
    out = [f"The note must contain `{v}` exactly as written; it was missing or altered." for v in missing]
    if why:
        out.append(f"State this fact explicitly, with its values: \"{f['statement']}\" ({why}).")
    return out


def drop(ds: Path, w: notes_mod.World, aliases: list[dict], note: dict, fact: dict, reason: str) -> None:
    fact["status"] = "dropped"
    note["facts"] = [i for i in note["facts"] if i != fact["id"]]
    if not note["facts"]:
        note["filler"] = True
    if fact["bridge"]:
        for a in aliases:
            if a["alias"] == fact["bridge"]["alias"] and a["project"] == fact["project"]:
                a["bridge_notes"] = [i for i in a["bridge_notes"] if i != note["id"]]
    common.append_jsonl(Path(ds) / "generation/drops.jsonl", {
        "item": fact["id"], "kind": "fact", "step": STEP, "note": note["id"], "reason": reason,
    })


def settle(ds: Path, cfg: llm.GenConfig, w: notes_mod.World, aliases: list[dict]) -> tuple[Exception | None, dict[str, int]]:
    """Check, re-render and drop until every note with facts either passes or has lost its failing facts."""
    stats = {"checked": 0, "rendered": 0, "dropped": 0}
    pending = [n for n in w.notes if n["status"] == "kept" and live(w, n)]
    failed: dict[str, Exception] = {}
    for _ in range(MAX_ATTEMPTS + 1):
        if not pending:
            break
        attempts, texts, problems, checked, calls = {}, {}, {}, {}, []
        for n in pending:
            attempts[n["id"]], _ = notes_mod.latest(ds, n["id"])
            texts[n["id"]] = body_text(ds, n)
            found = {f["id"]: {"missing": m, "why": None} for f in live(w, n) if (m := missing_strings(texts[n["id"]], f))}
            problems[n["id"]] = found
            # a note that will be re-rendered anyway skips the checker, except on its last attempt
            checked[n["id"]] = live(w, n) if not found else (
                [f for f in live(w, n) if f["id"] not in found] if attempts[n["id"]] >= MAX_ATTEMPTS else []
            )
            if checked[n["id"]]:
                prompt = check_prompt(texts[n["id"]], checked[n["id"]])
                calls.append(llm.Call(STEP, n["id"], "codex", prompt, schema("fidelity.json"), attempts[n["id"]]))
        if calls:
            llm.preflight("codex", ds, cfg, STEP)
            results = llm.run_many(calls, ds, cfg)
            if llm.fatal(results):
                return llm.fatal(results), stats
            stats["checked"] += len(calls)
            for c in calls:
                answer = results[c.item]
                if isinstance(answer, Exception):
                    failed[c.item] = answer
                    continue
                for fid, why in unreadable(texts[c.item], checked[c.item], answer).items():
                    problems[c.item][fid] = {"missing": [], "why": why}
        renders, again = [], []
        for n in pending:
            if n["id"] in failed:
                continue
            found = problems[n["id"]]
            if not found:
                continue
            if attempts[n["id"]] >= MAX_ATTEMPTS:
                for fid, p in found.items():
                    reason = "fidelity: " + (f"missing {p['missing']}" if p["missing"] else p["why"])
                    drop(ds, w, aliases, n, w.facts[fid], reason)
                    stats["dropped"] += 1
                continue
            lines = [x for fid, p in found.items() for x in feedback(w.facts[fid], p["missing"], p["why"])]
            prompt = notes_mod.prompt_for(w, n, lines)
            renders.append(llm.Call(notes_mod.STEP, n["id"], "claude", prompt, schema("note.json"), attempts[n["id"]] + 1))
            again.append(n)
        if renders:
            llm.preflight("claude", ds, cfg, notes_mod.STEP)
            results = llm.run_many(renders, ds, cfg)
            stats["rendered"] += sum(1 for r in results.values() if isinstance(r, dict) and notes_mod.valid("", r) is None)
            notes_mod.write_all(ds, w)
            stop = llm.fatal(results)
            if stop:
                return stop, stats
        pending = again
    if failed:
        names = ", ".join(f"{i} ({e})" for i, e in list(failed.items())[:5])
        return common.Refused(f"fidelity: the check of {len(failed)} notes failed, run it again: {names}"), stats
    if pending:
        return common.Refused(f"fidelity: {len(pending)} notes could not be re-rendered: " + ", ".join(n["id"] for n in pending[:5])), stats
    return None, stats


def refresh_noise(ds: Path, w: notes_mod.World) -> None:
    """Keep the `stale` pairs whose superseding and superseded facts are both still planted."""
    path = Path(ds) / "world/noise.json"
    if not path.is_file():
        return
    noise = json.loads(path.read_text(encoding="utf-8"))
    kept = {
        (w.facts[f["supersedes"]]["note_id"], f["note_id"]) for f in w.facts.values()
        if f["supersedes"] and f["status"] == "planted" and w.facts[f["supersedes"]]["status"] == "planted"
    }
    noise["stale"] = [p for p in noise.get("stale", []) if tuple(p) in kept]
    common.write_json(path, noise)


def cmd(args) -> int:
    from bilbo_evals import dataset

    ds = Path(args.dataset)
    cfg = llm.load_config(ds)
    llm.require_cli("claude")
    llm.require_cli("codex")
    w = notes_mod.load(ds)
    missing = [n["file"] for n in w.notes if n["status"] == "kept" and not (ds / "store/notes" / n["file"]).is_file()]
    if missing:
        raise common.Refused(f"{len(missing)} notes are not rendered yet: run `generate notes` first")
    notes_mod.write_all(ds, w)
    aliases_path = ds / "world/aliases.json"
    aliases = json.loads(aliases_path.read_text(encoding="utf-8")) if aliases_path.is_file() else []
    stop, stats = settle(ds, cfg, w, aliases)
    notes_mod.save_notes(ds, w.notes)
    notes_mod.save_facts(ds, w)
    common.write_json(aliases_path, aliases)
    refresh_noise(ds, w)
    dataset.build_corpus(ds)
    common.out(f"fidelity: {stats['checked']} checks, {stats['rendered']} re-renders, {stats['dropped']} facts dropped")
    if stop is not None:
        raise stop
    return 0
