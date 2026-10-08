"""`pool`: every arm's top hits per query and prompt, judged by the checking model, and the reviewer's resolutions applied."""

from __future__ import annotations

import argparse
import hashlib
import importlib.metadata
import json
import math
import random
import subprocess
from pathlib import Path

from bilbo_evals import common, dataset, llm, passages, runner, words
from bilbo_evals.arms import ARMS
from bilbo_evals.common import Refused, append_jsonl, err, out, read_jsonl, write_jsonl
from bilbo_evals.generate import HERE, finish, schema, solve, template

STEP = "pool"
TEMPLATE = "pool.md"
TOP = 10
SOURCE_CHARS = 2000
PASSAGE_CHARS = 1500
VIEW_CHARS = 4500
OUTLINE_CHARS = 500
OVERLAP_PASSAGES = 2
PROMPT_CHARS = 200_000  # the candidates of one call; 30 candidates at VIEW_CHARS come to 135,000, so an item is one call
AUDIT_SHARE = 0.10
AUDIT_MIN = 50
PROMPT_LABELS = ("positive", "near-miss")
ACTIONS = ("add-gold", "add-evidence", "reject", "rewrite", "drop")
KIND_RULE = {
    "query": "The request is a question a developer puts to their notes.",
    "prompt": "The request is a message a developer sends to a coding agent; a note answers it when it holds "
              "information the developer needs to do the work asked.",
}
_MARKS = str.maketrans("", "", "`*_")
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
    """Whitespace folded and the Markdown marks (backtick, asterisk, underscore) removed, the way models quote inline code."""
    return " ".join(text.translate(_MARKS).split())


def fold_quote(text: str) -> str:
    return fold(text).rstrip(".,;:")


def text_sha(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def note_sha(ds: dataset.Dataset, cid: str) -> str | None:
    """SHA-256 of a candidate's text (note or source); None for an id the dataset does not hold."""
    if cid in ds.notes:
        return text_sha(ds.notes[cid].text)
    return text_sha(ds.sources[cid].text) if cid in ds.sources else None


def _superseded(ds_dir: Path) -> Path:
    return _pool(ds_dir, "superseded.jsonl")


# --- what is pooled -----------------------------------------------------------------------------------------------

def pooled_items(ds: dataset.Dataset, split: str) -> list[dict]:
    """Every query and the positive and near-miss prompts of the split(s), as `{id, kind, split, text, row}`."""
    items = [{"id": q["id"], "kind": "query", "split": q["split"], "text": q["text"], "row": q}
             for q in ds.queries_for(split)]
    items += [{"id": p["id"], "kind": "prompt", "split": p["split"], "text": p["prompt"], "row": p}
              for p in ds.prompts if (split == "all" or p["split"] == split) and p["label"] in PROMPT_LABELS]
    return sorted(items, key=lambda i: i["id"])


def top_candidates(ds: dataset.Dataset, item: dict, rankings: dict[str, dict[str, list[str]]],
                   hits: dict[str, dict[str, dict[str, int]]] | None = None) -> list[dict]:
    """Union over the arms of each one's first TOP ids that are not gold; notes and sources only.

    `hits[arm][item][note]` is the file line of the passage that ranked the note; a candidate keeps it as `lines[arm]`.
    """
    gold = set(item["row"]["gold"]) | {i for ev in item["row"].get("evidence_sets", []) for i in ev}
    found: dict[str, dict] = {}
    for arm in ARMS:
        ranking = [i for i in rankings.get(arm, {}).get(item["id"], []) if i not in gold and (i in ds.notes or i in ds.sources)]
        for rank, cid in enumerate(ranking[:TOP], start=1):
            c = found.setdefault(cid, {"id": cid, "arms": [], "best_rank": rank})
            c["arms"].append(arm)
            c["best_rank"] = min(c["best_rank"], rank)
            line = (hits or {}).get(arm, {}).get(item["id"], {}).get(cid)
            if line is not None:
                c.setdefault("lines", {})[arm] = line
    return sorted(found.values(), key=lambda c: (c["best_rank"], c["id"]))


def _shown(p: passages.Passage) -> str:
    return " > ".join(p.heading_path) + "\n" + p.text if p.heading_path else p.text


def note_view(ds: dataset.Dataset, cid: str, query: str, lines: list[int] | None = None) -> str:
    """A note's title and heading outline, the passages at `lines` (the arms' hits) and its top passages by shared words.

    Each passage is cut at PASSAGE_CHARS and the whole view at VIEW_CHARS; hits come before overlap passages.
    """
    note = ds.notes[cid]
    path = Path(ds.dir) / "store/notes" / note.file
    ps = passages.passages(path.read_text(encoding="utf-8"), path.stem)
    below = lambda p: p.heading_path[1:] if p.heading_path[:1] == [note.title] else p.heading_path  # noqa: E731
    outline = list(dict.fromkeys(" > ".join(below(p)) for p in ps if below(p)))
    head = f"Title: {note.title}\nOutline: {' | '.join(outline)}"[:OUTLINE_CHARS]
    chosen: list[int] = []
    for line in lines or []:
        at = max((n for n, p in enumerate(ps) if p.line <= line), default=None)
        if at is not None and at not in chosen:
            chosen.append(at)
    asked = set(words.words(query))
    scores = [sum(w in asked for w in words.words(_shown(p))) for p in ps]
    chosen += [n for n in sorted(range(len(ps)), key=lambda n: (-scores[n], n))[:OVERLAP_PASSAGES] if n not in chosen]
    room, kept = VIEW_CHARS - len(head), []
    for n in chosen:
        if room <= 1:
            break
        cut = _shown(ps[n])[: min(PASSAGE_CHARS, room - 1)]
        kept.append((n, cut))
        room -= len(cut) + 1
    return "\n".join([head, *(c for _, c in sorted(kept))])[:VIEW_CHARS]


def candidate_text(ds: dataset.Dataset, cid: str, query: str, lines: list[int] | None = None) -> str:
    """A note's view (see `note_view`); a source's passage that shares the most words with the query, cut to SOURCE_CHARS."""
    if cid in ds.notes:
        return note_view(ds, cid, query, lines)
    text = ds.sources[cid].text
    asked = set(words.words(query))
    best, best_score = None, -1
    for p in passages.passages(text):
        shown = _shown(p)
        score = sum(w in asked for w in words.words(shown))
        if score > best_score:
            best, best_score = shown, score
    return (best if best is not None else text.strip())[:SOURCE_CHARS]


def _view(ds: dataset.Dataset, cand: dict, query: str) -> str:
    return candidate_text(ds, cand["id"], query, list(cand.get("lines", {}).values()))


def _order(item: dict, cands: list[dict], seed: int) -> list[dict]:
    order = sorted(cands, key=lambda c: c["id"])
    random.Random(f"{seed}:{item['id']}").shuffle(order)
    return order


def chunks(ds: dataset.Dataset, item: dict, cands: list[dict], seed: int) -> list[list[dict]]:
    """The candidates in the seeded order, split into calls whose candidate text stays within PROMPT_CHARS."""
    out: list[list[dict]] = [[]]
    size = 0
    for c in _order(item, cands, seed):
        n = len(_view(ds, c, item["text"]))
        if out[-1] and size + n > PROMPT_CHARS:
            out.append([])
            size = 0
        out[-1].append(c)
        size += n
    return out


def call_ids(item: dict, n: int) -> list[str]:
    """The item's call id, or `<item>--b<k>` per chunk when it needs more than one."""
    return [item["id"]] if n == 1 else [f"{item['id']}--b{k}" for k in range(1, n + 1)]


def build_prompt(ds: dataset.Dataset, item: dict, cands: list[dict], seed: int, call_id: str | None = None) -> str:
    multi = item["kind"] == "query" and item["row"]["stratum"] == "multi-hop"
    evidence = ""
    if multi:
        blocks = []
        for n, ev in enumerate(item["row"]["evidence_sets"]):
            body = "\n".join(f"  [{i}]\n{note_view(ds, i, item['text'])}" for i in ev if i in ds.notes)
            blocks.append(f"Evidence set {n}:\n{body}")
        evidence = "\nEvidence sets so far (each lists notes that together answer the request):\n" + "\n".join(blocks) + "\n"
    listing = "\n".join(f"=== {c['id']} ===\n{_view(ds, c, item['text'])}\n" for c in _order(item, cands, seed))
    return template(TEMPLATE).substitute(
        item=call_id or item["id"], kind_rule=KIND_RULE[item["kind"]], query=item["text"], evidence=evidence,
        set_rule=SET_RULE_MULTI if multi else SET_RULE_PLAIN, candidates=listing,
    )


def evidence_shas(ds: dataset.Dataset, row: dict) -> dict[str, str]:
    """Text hash of every note in the item's evidence sets, which the prompt of a multi-hop item shows."""
    ids = sorted({i for ev in row.get("evidence_sets", []) for i in ev if i in ds.notes})
    return {i: text_sha(ds.notes[i].text) for i in ids}


def _template_sha(name: str) -> str:
    return text_sha((HERE / "templates" / name).read_text(encoding="utf-8"))


def harness() -> dict:
    """The version of this package and the git commit of the checkout it runs from (None outside a checkout)."""
    try:
        package = importlib.metadata.version("bilbo-evals")
    except importlib.metadata.PackageNotFoundError:
        package = "unknown"
    try:
        done = subprocess.run(["git", "-C", str(Path(__file__).resolve().parent), "rev-parse", "HEAD"],
                              capture_output=True, text=True, timeout=10, check=True)
        commit = done.stdout.strip() or None
    except (OSError, subprocess.SubprocessError):
        commit = None
    return {"package": package, "commit": commit}


def call_record(ds: dataset.Dataset, item: dict, cands: list[dict], seed: int, call_id: str, prompt: str) -> dict:
    """What `build_prompt` reads for one call, as judged: kept in `pool/records.jsonl` so the prompt can be rebuilt later."""
    row = item["row"]
    return {
        "call_id": call_id, "item": item["id"], "kind": item["kind"], "text": item["text"], "text_sha256": text_sha(item["text"]),
        "stratum": row.get("stratum"), "evidence_sets": [list(ev) for ev in row.get("evidence_sets", [])], "evidence_sha256": evidence_shas(ds, row),
        "candidates": [{"id": c["id"], "lines": c.get("lines", {})} for c in cands], "seed": seed,
        "harness": harness(), "template": TEMPLATE, "template_sha256": _template_sha(TEMPLATE), "prompt_sha256": text_sha(prompt),
    }


def rebuild_prompt(ds: dataset.Dataset, record: dict) -> str:
    """The prompt of a recorded call, from its record, the notes and the template; refuses once the template changed."""
    if record["template_sha256"] != _template_sha(record["template"]):
        raise Refused(f"{record['call_id']}: the template {record['template']} changed since the call; the prompt cannot be rebuilt")
    if record.get("harness") != harness():
        err(f"{record['call_id']}: the prompt was built by another harness version or commit; the rebuild is exact only under the recorded one")
    item = {"id": record["item"], "kind": record["kind"], "text": record["text"],
            "row": {"stratum": record["stratum"], "evidence_sets": record["evidence_sets"], "gold": []}}
    return build_prompt(ds, item, record["candidates"], record["seed"], record["call_id"])


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


def judgment_rows(ds: dataset.Dataset, item: dict, cands: list[dict], value: dict, attempt: int,
                  call_id: str | None = None) -> list[dict]:  # fmt: skip
    """One row per candidate; a yes without a passage found verbatim in the text shown keeps its claim and is flagged."""
    multi = item["kind"] == "query" and item["row"]["stratum"] == "multi-hop"
    texts = {c["id"]: fold(_view(ds, c, item["text"])) for c in cands}
    sha = text_sha(item["text"])
    rows = []
    for j in sorted(value["judgments"], key=lambda j: j["id"]):
        cs = j["completes_set"] if multi else None
        passage = (j["passage"] or "").strip() or None
        found = bool(passage) and bool(fold_quote(passage)) and fold_quote(passage) in texts[j["id"]]
        claimed = bool(j["answers"] or cs is not None)
        rows.append({
            "item": item["id"], "candidate": j["id"], "answers": bool(j["answers"]), "completes_set": cs,
            "passage": passage, "quote_found": found, "flag": "yes_without_quote" if claimed and not found else None,
            "call_id": call_id or f"{STEP}/{item['id']}/{attempt}", "text_sha256": sha,
            "note_sha256": note_sha(ds, j["id"]), "evidence_sha256": evidence_shas(ds, item["row"]) if multi else {},
        })
    return rows


# --- the pooling step ---------------------------------------------------------------------------------------------

def audit_need(n_noes: int) -> int:
    return min(n_noes, max(AUDIT_MIN, math.ceil(AUDIT_SHARE * n_noes)))


def audit_topup(judgments: list[dict], have: list[dict], seed: int, split: str) -> list[dict]:
    """Audit rows to add so the split's sample reaches `audit_need` over its current noes; `have` rows that still match a no count."""
    noes = sorted((r["item"], r["candidate"]) for r in judgments if not _yes(r))
    kept = {(a["item"], a["candidate"]) for a in have} & set(noes)
    missing = audit_need(len(noes)) - len(kept)
    if missing <= 0:
        return []
    rest = [n for n in noes if n not in kept]
    picked = sorted(random.Random(f"{seed}:audit:{split}:{len(kept)}").sample(rest, missing))
    return [{"item": i, "candidate": c, "split": split, "verdict": None, "reason": None, "reviewer": None}
            for i, c in picked]


def is_current(ds: dataset.Dataset, row: dict, shas: dict[str, str]) -> bool:
    """A judgment still describes the item's text, the candidate's text and the evidence notes it was shown, as they are now."""
    return (row.get("text_sha256") == shas.get(row["item"]) and row.get("note_sha256") == note_sha(ds, row["candidate"])
            and all(note_sha(ds, i) == h for i, h in row.get("evidence_sha256", {}).items()))


def current_noes(ds: dataset.Dataset, items: list[dict], cands: dict[str, dict], judged: list[dict]) -> list[dict] | None:
    """Judgments (as no) of the items' listed candidates for the current texts; None while one of them is not judged."""
    shas = {i["id"]: text_sha(i["text"]) for i in items}
    by = {(r["item"], r["candidate"]): r for r in judged if is_current(ds, r, shas)}
    noes = []
    for it in items:
        row = cands.get(it["id"])
        if not row or row.get("text_sha256") != shas[it["id"]]:
            continue
        for c in row["candidates"]:
            j = by.get((it["id"], c["id"]))
            if j is None:
                return None
            if not _yes(j):
                noes.append(j)
    return noes


def _yes(row: dict) -> bool:
    return dataset._yes(row)


def _attempt_of(ds_dir: Path, item: str, value: dict) -> int:
    for a in (1, 2, 3):
        path = llm.output_path(ds_dir, STEP, item, a)
        if path.is_file() and json.loads(path.read_text(encoding="utf-8")) == value:
            return a
    return 1


def set_aside(ds_dir: Path, items: list[dict]) -> None:
    """Move the pool rows and outputs of every item whose text changed since it was ranked to `superseded.jsonl`."""
    cands = {r["item"]: r for r in _rows(ds_dir, "candidates.jsonl")}
    stale = sorted(it["id"] for it in items if it["id"] in cands and cands[it["id"]].get("text_sha256") != text_sha(it["text"]))
    if not stale:
        return
    gone = set(stale)
    for name in ("candidates", "judgments", "audit", "resolutions", "applied", "records"):
        rows = _rows(ds_dir, f"{name}.jsonl")
        if not any(r["item"] in gone for r in rows):
            continue
        for r in rows:
            if r["item"] in gone:
                append_jsonl(_superseded(ds_dir), {"file": f"{name}.jsonl", "row": r, "time": common.now()})
        write_jsonl(_pool(ds_dir, f"{name}.jsonl"), [r for r in rows if r["item"] not in gone])
    for item in stale:
        _park_outputs(ds_dir, item, cands[item].get("text_sha256", "unhashed"))
        err(f"{item}: its text changed since it was pooled; the old rows are in generation/pool/superseded.jsonl")


def set_aside_judgments(ds_dir: Path, ds: dataset.Dataset, items: list[dict]) -> None:
    """Move the judgments, audit rows, records and outputs of every item with a judgment whose candidate text changed since.

    The resolutions of the pairs that are no longer current go too: a reviewer's verdict on the old text must not cover the new judgment.
    """
    shas = {it["id"]: text_sha(it["text"]) for it in items}
    judged = _rows(ds_dir, "judgments.jsonl")
    old = {(r["item"], r["candidate"]) for r in judged if r["item"] in shas and r.get("text_sha256") == shas[r["item"]]
           and not is_current(ds, r, shas)}
    stale = sorted({i for i, _ in old})
    if not stale:
        return
    gone = set(stale)
    prompts = {r["call_id"]: r["prompt_sha256"] for r in _rows(ds_dir, "records.jsonl") if r["item"] in gone}
    for name in ("judgments", "audit", "records", "resolutions"):
        rows = _rows(ds_dir, f"{name}.jsonl")
        hit = (lambda r: (r["item"], r["candidate"]) in old) if name == "resolutions" else (lambda r: r["item"] in gone)  # noqa: E731
        if not any(hit(r) for r in rows):
            continue
        for r in rows:
            if hit(r):
                append_jsonl(_superseded(ds_dir), {"file": f"{name}.jsonl", "row": r, "time": common.now()})
        write_jsonl(_pool(ds_dir, f"{name}.jsonl"), [r for r in rows if not hit(r)])
    for r in _rows(ds_dir, "applied.jsonl"):
        if (r["item"], r["candidate"]) in old:
            err(f"{r['item']} {r['candidate']}: its {r['action']} was applied to the item before the note changed; check the gold")
    for item in stale:
        _park_outputs(ds_dir, item, shas[item], prompts)
        err(f"{item}: a candidate's or evidence note's text changed since it was judged; the old judgments are in generation/pool/superseded.jsonl")


def _park(path: Path, tag: str) -> None:
    """Move an output into `superseded/` under a name that holds `tag` and never replaces an earlier one."""
    target = path.parent / "superseded" / f"{path.stem}.{tag[:12]}.json"
    n = 1
    while target.exists():
        n += 1
        target = target.with_name(f"{path.stem}.{tag[:12]}.{n}.json")
    target.parent.mkdir(parents=True, exist_ok=True)
    path.replace(target)


def _park_outputs(ds_dir: Path, item: str, tag: str, prompts: dict[str, str] | None = None) -> None:
    """Park every output of the item's calls; one that has a prompt hash in `prompts` is named by it."""
    out = llm.output_path(ds_dir, STEP, item, 1).parent
    for path in sorted([*out.glob(f"{item}.[123].json"), *out.glob(f"{item}--b*.[123].json")]):
        _park(path, (prompts or {}).get(path.stem.rsplit(".", 1)[0], tag))


def set_aside_stale_outputs(ds_dir: Path, records: dict[str, dict]) -> None:
    """Park the outputs of a call whose prompt is no longer the one recorded for it, so `solve` makes a real call."""
    old = {r["call_id"]: r for r in _rows(ds_dir, "records.jsonl")}
    for cid, rec in records.items():
        if cid in old and old[cid]["prompt_sha256"] != rec["prompt_sha256"]:
            out = llm.output_path(ds_dir, STEP, cid, 1).parent
            for path in sorted(out.glob(f"{cid}.[123].json")):
                _park(path, old[cid]["prompt_sha256"])


def _top_up_audits(ds_dir: Path, ds: dataset.Dataset, seed: int) -> None:
    """Per split whose pooled items are all judged, add audit rows until the sample reaches its size."""
    cands = {r["item"]: r for r in _rows(ds_dir, "candidates.jsonl")}
    judged = _rows(ds_dir, "judgments.jsonl")
    audit = _rows(ds_dir, "audit.jsonl")
    all_items = pooled_items(ds, "all")
    added: list[dict] = []
    for split in dataset.schema.SPLITS:
        items = [i for i in all_items if i["split"] == split]
        noes = current_noes(ds, items, cands, judged)
        if noes is None:
            continue
        ids = {(j["item"], j["candidate"]) for j in noes}
        have = [a for a in audit if (a["item"], a["candidate"]) in ids]
        added += audit_topup(noes, have, seed, split)
    if added:
        write_jsonl(_pool(ds_dir, "audit.jsonl"), sorted(audit + added, key=lambda a: (a["item"], a["candidate"])))


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
    set_aside(ds_dir, items)
    set_aside_judgments(ds_dir, ds, items)
    shas = {it["id"]: text_sha(it["text"]) for it in items}

    cand_rows = {r["item"]: r for r in _rows(ds_dir, "candidates.jsonl")}
    missing = [i for i in items if i["id"] not in cand_rows]
    if missing:
        hits: dict = {}
        rankings = runner.rank_all(ds_dir, args.split, list(ARMS), Path(args.bilbo), args.model, args.llama_server,
                                   getattr(args, "embedder_url", None), hits=hits)
        for it in missing:
            cand_rows[it["id"]] = {"item": it["id"], "kind": it["kind"], "split": it["split"],
                                   "text_sha256": shas[it["id"]], "candidates": top_candidates(ds, it, rankings, hits)}
        write_jsonl(_pool(ds_dir, "candidates.jsonl"), [cand_rows[k] for k in sorted(cand_rows)])

    judged = _rows(ds_dir, "judgments.jsonl")
    done = {r["item"] for r in judged if r.get("text_sha256") == shas.get(r["item"])}
    todo = [it for it in items if it["id"] not in done and cand_rows[it["id"]]["candidates"]]
    wants, checks, parts, records = {}, {}, {}, {}
    pool_schema = schema("pool.json")
    for it in todo:
        n_sets = len(it["row"].get("evidence_sets", [])) if it["kind"] == "query" and it["row"]["stratum"] == "multi-hop" else 0
        groups = chunks(ds, it, cand_rows[it["id"]]["candidates"], cfg.seed)
        parts[it["id"]] = call_ids(it, len(groups))
        for cid, group in zip(parts[it["id"]], groups):
            prompt = build_prompt(ds, it, group, cfg.seed, cid)
            wants[cid] = (prompt, pool_schema)
            records[cid] = call_record(ds, it, group, cfg.seed, cid, prompt)
            checks[cid] = valid_output({c["id"] for c in group}, n_sets)
    set_aside_stale_outputs(ds_dir, records)
    if records:
        kept = [r for r in _rows(ds_dir, "records.jsonl") if r["call_id"] not in records]
        write_jsonl(_pool(ds_dir, "records.jsonl"), sorted([*kept, *records.values()], key=lambda r: r["call_id"]))
    solved = solve(ds_dir, cfg, STEP, "codex", wants, lambda item, value: checks[item](item, value))
    by_id = {it["id"]: it for it in items}
    for item, ids in sorted(parts.items()):
        if not all(i in solved.good for i in ids):
            continue
        merged = {"judgments": [j for i in ids for j in solved.good[i]["judgments"]]}
        attempts = [_attempt_of(ds_dir, i, solved.good[i]) for i in ids]
        call_id = ";".join(f"{STEP}/{i}/{a}" for i, a in zip(ids, attempts))
        judged += judgment_rows(ds, by_id[item], cand_rows[item]["candidates"], merged, attempts[0], call_id)
    write_jsonl(_pool(ds_dir, "judgments.jsonl"), sorted(judged, key=lambda r: (r["item"], r["candidate"])))

    _top_up_audits(ds_dir, ds, cfg.seed)
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
