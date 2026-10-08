"""`pool`: every arm's top hits per query and prompt, judged by the checking model, and the reviewer's resolutions applied."""

from __future__ import annotations

import argparse
import json
import math
import random
from pathlib import Path

from bilbo_evals import common, dataset, llm, passages, runner, words
from bilbo_evals.arms import ARMS
from bilbo_evals.common import Refused, append_jsonl, err, out, read_jsonl, write_jsonl
from bilbo_evals.generate import finish, schema, solve, template

STEP = "pool"
TOP = 10
NOTE_CHARS = 3000
SOURCE_CHARS = 2000
EVIDENCE_CHARS = 1500
AUDIT_SHARE = 0.10
AUDIT_MIN = 50
PROMPT_LABELS = ("positive", "near-miss")
ACTIONS = ("add-gold", "add-evidence", "reject", "rewrite", "drop")
KIND_RULE = {
    "query": "The request is a question a developer puts to their notes.",
    "prompt": "The request is a message a developer sends to a coding agent; a note answers it when it holds "
              "information the developer needs to do the work asked.",
}
SET_RULE_PLAIN = "always null."
SET_RULE_MULTI = (
    "the 0-based number of an evidence set above that the candidate belongs in, because the set as listed would "
    "not answer the request without the information in the candidate; null when it belongs in none."
)


def _pool(ds_dir: Path, name: str) -> Path:
    return Path(ds_dir) / "generation/pool" / name


def _rows(ds_dir: Path, name: str) -> list[dict]:
    path = _pool(ds_dir, name)
    return read_jsonl(path) if path.is_file() else []


def fold(text: str) -> str:
    return " ".join(text.split())


# --- what is pooled -----------------------------------------------------------------------------------------------

def pooled_items(ds: dataset.Dataset, split: str) -> list[dict]:
    """Every query and the positive and near-miss prompts of the split(s), as `{id, kind, split, text, row}`."""
    items = [{"id": q["id"], "kind": "query", "split": q["split"], "text": q["text"], "row": q}
             for q in ds.queries_for(split)]
    items += [{"id": p["id"], "kind": "prompt", "split": p["split"], "text": p["prompt"], "row": p}
              for p in ds.prompts if (split == "all" or p["split"] == split) and p["label"] in PROMPT_LABELS]
    return sorted(items, key=lambda i: i["id"])


def top_candidates(ds: dataset.Dataset, item: dict, rankings: dict[str, dict[str, list[str]]]) -> list[dict]:
    """Union over the arms of each one's first TOP ids that are not gold; notes and sources only."""
    gold = set(item["row"]["gold"]) | {i for ev in item["row"].get("evidence_sets", []) for i in ev}
    found: dict[str, dict] = {}
    for arm in ARMS:
        ranking = [i for i in rankings.get(arm, {}).get(item["id"], []) if i not in gold and (i in ds.notes or i in ds.sources)]
        for rank, cid in enumerate(ranking[:TOP], start=1):
            c = found.setdefault(cid, {"id": cid, "arms": [], "best_rank": rank})
            c["arms"].append(arm)
            c["best_rank"] = min(c["best_rank"], rank)
    return sorted(found.values(), key=lambda c: (c["best_rank"], c["id"]))


def candidate_text(ds: dataset.Dataset, cid: str, query: str) -> str:
    """A note's body cut to NOTE_CHARS; a source's passage that shares the most words with the query, cut to SOURCE_CHARS."""
    if cid in ds.notes:
        return ds.notes[cid].text.strip()[:NOTE_CHARS]
    text = ds.sources[cid].text
    asked = set(words.words(query))
    best, best_score = None, -1
    for p in passages.passages(text):
        shown = " > ".join(p.heading_path) + "\n" + p.text if p.heading_path else p.text
        score = sum(w in asked for w in words.words(shown))
        if score > best_score:
            best, best_score = shown, score
    return (best if best is not None else text.strip())[:SOURCE_CHARS]


def build_prompt(ds: dataset.Dataset, item: dict, cands: list[dict], seed: int) -> str:
    multi = item["kind"] == "query" and item["row"]["stratum"] == "multi-hop"
    evidence = ""
    if multi:
        blocks = []
        for n, ev in enumerate(item["row"]["evidence_sets"]):
            body = "\n".join(f"  [{i}] {ds.notes[i].text.strip()[:EVIDENCE_CHARS]}" for i in ev if i in ds.notes)
            blocks.append(f"Evidence set {n}:\n{body}")
        evidence = "\nEvidence sets so far (each lists notes that together answer the request):\n" + "\n".join(blocks) + "\n"
    order = sorted(c["id"] for c in cands)
    random.Random(f"{seed}:{item['id']}").shuffle(order)
    listing = "\n".join(f"=== {cid} ===\n{candidate_text(ds, cid, item['text'])}\n" for cid in order)
    return template("pool.md").substitute(
        item=item["id"], kind_rule=KIND_RULE[item["kind"]], query=item["text"], evidence=evidence,
        set_rule=SET_RULE_MULTI if multi else SET_RULE_PLAIN, candidates=listing,
    )


def valid_output(ids: set[str], n_sets: int):
    def check(_item: str, value: dict) -> str | None:
        got = [j.get("id") for j in value.get("judgments", [])]
        if sorted(got) != sorted(ids):
            return f"judgments name {len(got)} ids, expected each of the {len(ids)} candidates once"
        for j in value["judgments"]:
            cs = j["completes_set"]
            if cs is not None and not 0 <= cs < max(n_sets, 1):
                return f"{j['id']}: completes_set {cs} names no evidence set"
        return None
    return check


def judgment_rows(ds: dataset.Dataset, item: dict, cands: list[dict], value: dict, attempt: int) -> list[dict]:
    """One row per candidate; a yes without a passage found verbatim in the text shown keeps its claim and is flagged."""
    multi = item["kind"] == "query" and item["row"]["stratum"] == "multi-hop"
    texts = {c["id"]: fold(candidate_text(ds, c["id"], item["text"])) for c in cands}
    rows = []
    for j in sorted(value["judgments"], key=lambda j: j["id"]):
        cs = j["completes_set"] if multi else None
        passage = (j["passage"] or "").strip() or None
        found = bool(passage) and fold(passage) in texts[j["id"]]
        claimed = bool(j["answers"] or cs is not None)
        rows.append({
            "item": item["id"], "candidate": j["id"], "answers": bool(j["answers"]), "completes_set": cs,
            "passage": passage, "quote_found": found, "flag": "yes_without_quote" if claimed and not found else None,
            "call_id": f"{STEP}/{item['id']}/{attempt}",
        })
    return rows


# --- the pooling step ---------------------------------------------------------------------------------------------

def audit_sample(judgments: list[dict], seed: int) -> list[dict]:
    noes = sorted((r["item"], r["candidate"]) for r in judgments if not _yes(r))
    size = min(len(noes), max(AUDIT_MIN, math.ceil(AUDIT_SHARE * len(noes))))
    picked = sorted(random.Random(seed).sample(noes, size))
    return [{"item": i, "candidate": c, "verdict": None, "reason": None, "reviewer": None} for i, c in picked]


def _yes(row: dict) -> bool:
    return dataset._yes(row)


def _attempt_of(ds_dir: Path, item: str, value: dict) -> int:
    for a in (1, 2, 3):
        path = llm.output_path(ds_dir, STEP, item, a)
        if path.is_file() and json.loads(path.read_text(encoding="utf-8")) == value:
            return a
    return 1


def cmd(args: argparse.Namespace) -> int:
    if args.apply:
        return apply(Path(args.dataset))
    ds_dir = Path(args.dataset)
    dataset.refuse_if_frozen(ds_dir)
    cfg = llm.load_config(ds_dir)
    llm.require_cli("codex")
    ds = dataset.load(ds_dir)
    items = pooled_items(ds, args.split)
    if not items:
        raise Refused(f"the {args.split} split has no queries or prompts to pool")

    cand_rows = {r["item"]: r for r in _rows(ds_dir, "candidates.jsonl")}
    missing = [i for i in items if i["id"] not in cand_rows]
    if missing:
        rankings = runner.rank_all(ds_dir, args.split, list(ARMS), Path(args.bilbo), args.model, args.llama_server,
                                   getattr(args, "embedder_url", None))
        for it in missing:
            cand_rows[it["id"]] = {"item": it["id"], "kind": it["kind"], "split": it["split"],
                                   "candidates": top_candidates(ds, it, rankings)}
        write_jsonl(_pool(ds_dir, "candidates.jsonl"), [cand_rows[k] for k in sorted(cand_rows)])

    judged = _rows(ds_dir, "judgments.jsonl")
    done = {r["item"] for r in judged}
    todo = [it for it in items if it["id"] not in done and cand_rows[it["id"]]["candidates"]]
    wants, checks = {}, {}
    pool_schema = schema("pool.json")
    for it in todo:
        cands = cand_rows[it["id"]]["candidates"]
        n_sets = len(it["row"].get("evidence_sets", [])) if it["kind"] == "query" and it["row"]["stratum"] == "multi-hop" else 0
        wants[it["id"]] = (build_prompt(ds, it, cands, cfg.seed), pool_schema)
        checks[it["id"]] = valid_output({c["id"] for c in cands}, n_sets)
    solved = solve(ds_dir, cfg, STEP, "codex", wants, lambda item, value: checks[item](item, value))
    by_id = {it["id"]: it for it in items}
    for item, value in sorted(solved.good.items()):
        judged += judgment_rows(ds, by_id[item], cand_rows[item]["candidates"], value, _attempt_of(ds_dir, item, value))
    write_jsonl(_pool(ds_dir, "judgments.jsonl"), sorted(judged, key=lambda r: (r["item"], r["candidate"])))

    finished = len({r["item"] for r in judged}) >= len([i for i in items if cand_rows[i["id"]]["candidates"]])
    if finished and not _rows(ds_dir, "audit.jsonl"):
        write_jsonl(_pool(ds_dir, "audit.jsonl"), audit_sample(judged, cfg.seed))
    yes = [r for r in judged if _yes(r)]
    flagged = [r for r in judged if r["flag"]]
    out(f"pooled {len(items)} items, {len(judged)} candidates judged: {len(yes)} yes, {len(flagged)} unquoted yes counted as no")
    finish(STEP, solved, len(wants), "items")
    return 0


# --- applying the resolutions -------------------------------------------------------------------------------------

def apply(ds_dir: Path) -> int:
    dataset.refuse_if_frozen(ds_dir)
    ds = dataset.load(ds_dir)
    applied = {(r["item"], r["candidate"]) for r in _rows(ds_dir, "applied.jsonl")}
    resolutions = _rows(ds_dir, "resolutions.jsonl")
    for r in resolutions:
        if r["action"] not in ACTIONS:
            raise Refused(f"{r['item']} {r['candidate']}: unknown action {r['action']!r}; the actions are {', '.join(ACTIONS)}")
    queries = {q["id"]: q for q in ds.queries}
    prompts = {p["id"]: p for p in ds.prompts}
    dropped = {r["item"] for r in resolutions if r["action"] == "drop"}
    new = []
    for r in resolutions:
        if (r["item"], r["candidate"]) in applied:
            continue
        if r["action"] == "rewrite":
            err(f"{r['item']}: the reviewer asked for a rewrite; edit the item by hand")
        elif r["action"] != "reject":
            new.append(r)
    for r in new:
        if r["item"] in dropped:
            continue
        if r["item"] not in queries and r["item"] not in prompts:
            raise Refused(f"{r['item']} {r['candidate']}: the item is not in the dataset")
        _apply_one(r, queries, prompts)
    for item in dropped:
        queries.pop(item, None)
        prompts.pop(item, None)
    if new:
        write_jsonl(ds_dir / "queries.jsonl", [queries[q["id"]] for q in ds.queries if q["id"] in queries])
        if ds.prompts:
            write_jsonl(ds_dir / "digest/prompts.jsonl", [prompts[p["id"]] for p in ds.prompts if p["id"] in prompts])
        for r in new:
            append_jsonl(_pool(ds_dir, "applied.jsonl"), {"item": r["item"], "candidate": r["candidate"],
                                                           "action": r["action"], "time": common.now()})
    dataset.build_qrels(ds_dir)
    out(f"applied {len(new)} resolutions")
    open_items = dataset.pool_open_items(ds_dir)
    for line in open_items:
        out(line)
    return 1 if open_items else 0


def _apply_one(r: dict, queries: dict[str, dict], prompts: dict[str, dict]) -> None:
    item, cid, action = r["item"], r["candidate"], r["action"]
    if action == "drop":
        return
    if item in prompts:
        p = prompts[item]
        if action == "add-evidence":
            raise Refused(f"{item} {cid}: a digest prompt has no evidence sets; use add-gold")
        if cid not in p["gold"]:
            p["gold"].append(cid)
        if p["label"] == "near-miss":
            p["label"] = "positive"
        return
    q = queries[item]
    if q["stratum"] == "no-answer":
        raise Refused(f"{item} {cid}: a no-answer query cannot take gold; drop or rewrite it")
    if action == "add-gold":
        if cid not in q["gold"]:
            q["gold"].append(cid)
        if q["stratum"] == "multi-hop":
            q["evidence_sets"].append([cid])
        return
    n = r.get("evidence_set")
    if q["stratum"] != "multi-hop" or not isinstance(n, int) or not 0 <= n < len(q["evidence_sets"]):
        raise Refused(f"{item} {cid}: add-evidence needs a multi-hop query and an evidence_set that exists")
    if cid not in q["evidence_sets"][n]:
        q["evidence_sets"][n].append(cid)
    if cid not in q["gold"]:
        q["gold"].append(cid)
