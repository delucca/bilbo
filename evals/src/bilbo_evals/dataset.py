"""A dataset folder: load, check, qrels and corpus, freeze, verify and the rules that gate runs on it."""

from __future__ import annotations

import argparse
import json
import os
import shutil
from collections import Counter, defaultdict
from dataclasses import dataclass, field
from pathlib import Path

from bilbo_evals import schema, words
from bilbo_evals.common import (
    CANARY, Refused, out, read_jsonl, sha256_bytes, sha256_file, write_jsonl,
)

LEAK_STRATA = ("paraphrase", "pt-en", "alias")
LEAK_REJECT = 2
IGNORED_FILES = {"MANIFEST", "FROZEN", ".DS_Store"}


@dataclass
class Note:
    id: str
    file: str
    kind: str
    created: str
    title: str
    text: str
    project: str | None
    lang: str


@dataclass
class Source:
    ref: str
    file: str
    title: str
    text: str
    url: str | None
    licence: str | None


@dataclass
class Dataset:
    dir: Path
    name: str
    version: str
    frozen: bool
    tree_hash: str | None
    notes: dict[str, Note]
    sources: dict[str, Source]
    queries: list[dict]
    prompts: list[dict]
    aliases: list[dict] = field(default_factory=list)
    _df: dict[str, tuple[Counter, int]] = field(default_factory=dict, repr=False)

    def queries_for(self, split: str, library: bool | None = None) -> list[dict]:
        rows = [q for q in self.queries if split == "all" or q["split"] == split]
        if library is not None:
            rows = [q for q in rows if (q["stratum"] == "library") == library]
        return rows


def frontmatter(text: str) -> tuple[dict[str, str], str]:
    """Top-level `key: value` pairs of a leading `---` block, and the text after it."""
    lines = text.split("\n")
    if not lines or lines[0].strip() != "---":
        return {}, text
    for end in range(1, len(lines)):
        if lines[end].strip() == "---":
            meta = {}
            for line in lines[1:end]:
                if line and line[0] not in " \t-" and ":" in line:
                    key, value = line.split(":", 1)
                    meta[key.strip()] = value.strip().strip("\"'")
            return meta, "\n".join(lines[end + 1:])
    return {}, text


def _title(body: str) -> str:
    for line in body.split("\n"):
        if line.startswith("# "):
            return line[2:].strip()
    return ""


def refuse_if_frozen(ds_dir: Path) -> None:
    if (ds_dir / "FROZEN").exists():
        raise Refused(f"{ds_dir} is frozen: a change to its contents is a new version in a new folder")


def _jsonl(path: Path) -> list[dict]:
    return read_jsonl(path) if path.is_file() else []


def load(dir: Path) -> Dataset:
    dir = Path(dir)
    if not dir.is_dir():
        raise Refused(f"no dataset folder at {dir}")
    world = {r["id"]: r for r in _jsonl(dir / "world/notes.jsonl")}
    notes: dict[str, Note] = {}
    for path in sorted((dir / "store/notes").glob("*.md")):
        meta, body = frontmatter(path.read_text(encoding="utf-8"))
        nid = meta.get("id", "")
        w = world.get(nid, {})
        notes[nid] = Note(
            id=nid, file=path.name, kind=path.stem.split("-", 1)[0], created=meta.get("created", ""),
            title=_title(body), text=body, project=w.get("project"), lang=w.get("lang", "en"),
        )
    licences = {}
    lib = dir / "world/library.json"
    if lib.is_file():
        licences = {r["ref"]: r.get("licence") for r in json.loads(lib.read_text(encoding="utf-8"))}
    sources: dict[str, Source] = {}
    for path in sorted((dir / "store/library").glob("*/*.md")):
        if path.name == "guide.md":
            continue
        meta, body = frontmatter(path.read_text(encoding="utf-8"))
        ref = f"{path.parent.name}/{path.stem}"
        origin = meta.get("origin", "")
        url = origin.split(":", 1)[1].strip() if ":" in origin else None
        sources[ref] = Source(ref=ref, file=f"library/{ref}.md", title=_title(body), text=body, url=url,
                              licence=licences.get(ref))
    aliases_path = dir / "world/aliases.json"
    aliases = []
    if aliases_path.is_file():
        aliases = json.loads(aliases_path.read_text(encoding="utf-8"))
    frozen = (dir / "FROZEN").is_file()
    return Dataset(
        dir=dir, name=dir.resolve().parent.name, version=dir.resolve().name, frozen=frozen,
        tree_hash=(dir / "FROZEN").read_text(encoding="utf-8").strip() if frozen else None,
        notes=notes, sources=sources, queries=_jsonl(dir / "queries.jsonl"),
        prompts=_jsonl(dir / "digest/prompts.jsonl"), aliases=aliases,
    )


# --- leakage and aliases -------------------------------------------------------------------------------------------

def _df(ds: Dataset, library: bool) -> tuple[Counter, int]:
    key = "library" if library else "notes"
    if key not in ds._df:
        docs = [(n.text, n.lang) for n in ds.notes.values()]
        if library:
            docs += [(s.text, "en") for s in ds.sources.values()]
        ds._df[key] = (words.doc_freq(docs), len(docs))
    return ds._df[key]


def leakage(ds: Dataset, q: dict) -> set[str]:
    """Distinctive stems the query shares with any of its gold notes (sources for a library query)."""
    library = q["stratum"] == "library"
    df, n_docs = _df(ds, library)
    found: set[str] = set()
    for gid in q["gold"]:
        if library:
            doc = ds.sources.get(gid)
            doc_text, doc_lang = (doc.text, "en") if doc else ("", "en")
        else:
            note = ds.notes.get(gid)
            doc_text, doc_lang = (note.text, note.lang) if note else ("", "en")
        found |= words.shared_distinctive(q["text"], q["lang"], doc_text, doc_lang, df, n_docs)
    return found


def _holds(text: str, phrase: str) -> bool:
    """True when the words of `phrase` occur in a row in the words of `text`."""
    target, hay = words.words(phrase), words.words(text)
    if not target:
        return False
    n = len(target)
    return any(hay[i:i + n] == target for i in range(len(hay) - n + 1))


def alias_problems(ds: Dataset, q: dict) -> list[str]:
    """Alias rules of an `alias` query: the gold notes never use the alias, and another note bridges it."""
    if q["stratum"] != "alias":
        return []
    named = [a for a in ds.aliases if a.get("project") == q.get("project") and _holds(q["text"], a["alias"])]
    if not named:
        return [f"{q['id']}: names no alias of world/aliases.json"]
    problems = []
    for a in named:
        for gid in q["gold"]:
            note = ds.notes.get(gid)
            if note and _holds(note.text, a["alias"]):
                problems.append(f"{q['id']}: alias {a['alias']!r} is used by its gold note {gid}")
        bridged = any(
            nid not in q["gold"] and _holds(n.text, a["alias"]) and _holds(n.text, a["canonical"])
            for nid, n in ds.notes.items()
        )
        if not bridged:
            problems.append(f"{q['id']}: alias {a['alias']!r} has no note that holds it with {a['canonical']!r}")
    return problems


# --- check ---------------------------------------------------------------------------------------------------------

def _home_problems(ds_dir: Path) -> list[str]:
    homes = {h for h in {str(Path.home()), os.environ.get("HOME", "")} if len(h) > 1}
    if not homes:
        return []
    found = []
    for path in sorted(ds_dir.rglob("*")):
        if not path.is_file():
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError):
            continue
        if any(h in text for h in homes):
            found.append(f"{path.relative_to(ds_dir).as_posix()}: holds the invoking user's home path")
    return found


def _ids_exist(ds: Dataset, q: dict) -> list[str]:
    library = q["stratum"] == "library"
    known = ds.sources if library else ds.notes
    what = "landed source" if library else "note"
    problems = []
    for field_name in ("gold", "decoys"):
        for gid in q[field_name]:
            if gid not in known:
                problems.append(f"{q['id']}: {field_name} id {gid} is not a {what}")
    for i, ev in enumerate(q["evidence_sets"]):
        for gid in ev:
            if gid not in known:
                problems.append(f"{q['id']}: evidence set {i} id {gid} is not a {what}")
    return problems


def _splits_problems(ds: Dataset) -> list[str]:
    problems = []
    for key, label in (("project", "project"), ("family", "family")):
        seen: dict[str, dict[str, str]] = defaultdict(dict)
        for q in ds.queries:
            if q.get(key) and (key == "family" or q["stratum"] != "library"):
                seen[q[key]].setdefault(q["split"], q["id"])
        for p in ds.prompts:
            if key == "project" and p.get("project"):
                seen[p["project"]].setdefault(p["split"], p["id"])
        for name, splits in sorted(seen.items()):
            if len(splits) > 1:
                ids = ", ".join(f"{i} ({s})" for s, i in sorted(splits.items()))
                problems.append(f"{label} {name} is in both splits: {ids}")
    by_source: dict[str, dict[str, str]] = defaultdict(dict)
    for q in ds.queries:
        if q["stratum"] == "library":
            for gid in q["gold"]:
                by_source[gid].setdefault(q["split"], q["id"])
    for name, splits in sorted(by_source.items()):
        if len(splits) > 1:
            ids = ", ".join(f"{i} ({s})" for s, i in sorted(splits.items()))
            problems.append(f"source {name} is in both splits: {ids}")
    return problems


def check(ds: Dataset, split: str = "all") -> list[str]:
    problems: list[str] = []
    queries = ds.queries_for(split)
    prompts = [p for p in ds.prompts if split == "all" or p.get("split") == split]
    for q in queries:
        for p in schema.check("query", q):
            problems.append(f"{q.get('id', '?')}: {p}")
    for p in prompts:
        for msg in schema.check("prompt", p):
            problems.append(f"{p.get('id', '?')}: {msg}")
    for name, rows in (("queries", queries), ("prompts", prompts)):
        dup = [i for i, c in Counter(r.get("id") for r in rows).items() if c > 1]
        problems += [f"{name}: duplicate id {i}" for i in dup]
    if problems:
        return problems
    for q in queries:
        problems += _ids_exist(ds, q)
        if q["stratum"] == "no-answer":
            if q["gold"]:
                problems.append(f"{q['id']}: a no-answer query has gold ids")
        elif not q["gold"]:
            problems.append(f"{q['id']}: has no gold id")
        if q["stratum"] == "library" and not q["gold_heading"]:
            problems.append(f"{q['id']}: a library query needs gold_heading")
        if q["stratum"] == "kind-filter" and not q["kind"]:
            problems.append(f"{q['id']}: a kind-filter query needs a kind")
        problems += alias_problems(ds, q)
        if q["stratum"] in LEAK_STRATA:
            shared = leakage(ds, q)
            if len(shared) >= LEAK_REJECT:
                problems.append(f"{q['id']}: echoes its gold note, shared tokens: {', '.join(sorted(shared))}")
            if q["zero_overlap"] is None or q["zero_overlap"] != (not shared):
                problems.append(f"{q['id']}: zero_overlap is {q['zero_overlap']!r} but it shares {len(shared)} tokens")
    for p in prompts:
        for gid in p["gold"]:
            if gid not in ds.notes:
                problems.append(f"{p['id']}: gold id {gid} is not a note")
        if p["label"] == "positive" and not p["gold"]:
            problems.append(f"{p['id']}: a positive prompt has no gold id")
        if p["label"] != "positive" and p["gold"]:
            problems.append(f"{p['id']}: a {p['label']} prompt has gold ids")
    if split == "all":
        problems += _splits_problems(ds)
    if "" in ds.notes:
        problems.append("a note has no id")
    corpus = ds.dir / "corpus.jsonl"
    if corpus.is_file():
        have = {r["_id"] for r in read_jsonl(corpus)}
        want = set(ds.notes) | set(ds.sources)
        if have != want:
            problems.append(f"corpus.jsonl: out of step with store/ ({len(want - have)} missing, {len(have - want)} extra)")
    problems += _prereg_problems(ds, split)
    problems += _symlinks(ds.dir)
    problems += _home_problems(ds.dir)
    return problems


def _prereg_problems(ds: Dataset, split: str) -> list[str]:
    """Each stratum of the test split that holds fewer queries than `preregistration.json` asks."""
    path = ds.dir / "preregistration.json"
    if split not in ("test", "all") or not path.is_file():
        return []
    want = json.loads(path.read_text(encoding="utf-8")).get("per_stratum") or {}
    have = Counter(q["stratum"] for q in ds.queries_for("test"))
    return [f"test {s}: {have[s]} queries, preregistration asks {n}" for s, n in sorted(want.items()) if have[s] < n]


def cmd_check(args: argparse.Namespace) -> int:
    ds = load(args.dataset)
    problems = check(ds, args.split)
    for p in problems:
        out(p)
    if problems:
        return 1
    n_q = len(ds.queries_for(args.split))
    n_p = len([p for p in ds.prompts if args.split == "all" or p["split"] == args.split])
    out(f"ok: {n_q} queries, {n_p} prompts, {len(ds.notes)} notes")
    return 0


# --- corpus and qrels ----------------------------------------------------------------------------------------------

def build_corpus(dir: Path) -> None:
    refuse_if_frozen(dir)
    ds = load(dir)
    rows = []
    for n in ds.notes.values():
        rows.append({
            "_id": n.id, "title": n.title, "text": n.text,
            "metadata": {"kind": n.kind, "created": n.created, "lang": n.lang, "project": n.project,
                         "path": f"notes/{n.file}", "source": None},
            "canary": CANARY,
        })
    for s in ds.sources.values():
        meta, _ = frontmatter((dir / "store" / s.file).read_text(encoding="utf-8"))
        rows.append({
            "_id": s.ref, "title": s.title, "text": s.text,
            "metadata": {"kind": "source", "created": meta.get("fetched", ""), "lang": "en", "project": None,
                         "path": s.file, "source": {"url": s.url, "licence": s.licence}},
            "canary": CANARY,
        })
    write_jsonl(dir / "corpus.jsonl", sorted(rows, key=lambda r: r["_id"]))


def _yes(row: dict) -> bool:
    answered = row.get("answers") or row.get("completes_set") is not None
    return bool(answered and row.get("quote_found") and row.get("passage"))


def _pool(dir: Path, name: str) -> list[dict]:
    return _jsonl(dir / "generation/pool" / name)


def build_qrels(dir: Path) -> None:
    refuse_if_frozen(dir)
    ds = load(dir)
    judged: dict[str, set[str]] = defaultdict(set)
    for r in _pool(dir, "judgments.jsonl"):
        if not _yes(r):
            judged[r["item"]].add(r["candidate"])
    for r in _pool(dir, "resolutions.jsonl"):
        if r["action"] == "reject":
            judged[r["item"]].add(r["candidate"])
    for split in schema.SPLITS:
        for library in (False, True):
            lines = []
            for q in sorted(ds.queries_for(split, library), key=lambda q: q["id"]):
                if q["stratum"] == "no-answer":
                    continue
                rel1 = set(q["gold"]) | {i for ev in q["evidence_sets"] for i in ev}
                rel0 = (set(q["decoys"]) | judged[q["id"]]) - rel1
                rels = {**{d: 0 for d in rel0}, **{d: 1 for d in rel1}}
                lines += [f"{q['id']} 0 {d} {rels[d]}" for d in sorted(rels)]
            name = f"library-{split}.txt" if library else f"{split}.txt"
            path = dir / "qrels" / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("".join(line + "\n" for line in lines), encoding="utf-8")


def pool_open_items(dir: Path) -> list[str]:
    """What stops a freeze in the pool files, over the queries and the positive and near-miss prompts the dataset holds now."""
    from bilbo_evals import pool

    ds = load(dir)
    items = pool.pooled_items(ds, "all")
    ids = {i["id"] for i in items}
    shas = {i["id"]: pool.text_sha(i["text"]) for i in items}
    cands = {r["item"]: r for r in _pool(dir, "candidates.jsonl") if r["item"] in ids}
    judgments = [r for r in _pool(dir, "judgments.jsonl") if r["item"] in ids]
    audit = [r for r in _pool(dir, "audit.jsonl") if r["item"] in ids]
    resolved = {(r["item"], r["candidate"]): r for r in _pool(dir, "resolutions.jsonl") if r["item"] in ids}
    applied = {(r["item"], r["candidate"]) for r in _pool(dir, "applied.jsonl")}
    open_items = []
    judged_now = {(r["item"], r["candidate"]) for r in judgments if pool.is_current(ds, r, shas)}
    for it in items:
        row = cands.get(it["id"])
        if row is None or row.get("text_sha256") != shas[it["id"]]:
            open_items.append(f"{it['id']}: not pooled for its current text; run pool")
            continue
        for c in row["candidates"]:
            if (it["id"], c["id"]) not in judged_now:
                open_items.append(f"{it['id']} {c['id']}: not judged; run pool")
    for r in judgments:
        key = (r["item"], r["candidate"])
        if key in resolved:
            continue
        if _yes(r):
            open_items.append(f"{r['item']} {r['candidate']}: pooled yes with no resolution")
        elif r.get("flag") == "yes_without_quote":
            open_items.append(f"{r['item']} {r['candidate']}: unquoted yes with no resolution")
    for key, r in sorted(resolved.items()):
        if r["action"] == "rewrite":
            open_items.append(f"{key[0]} {key[1]}: rewrite asked; edit the item, then run pool")
    for item in sorted({k[0] for k, r in resolved.items() if r["action"] == "drop"}):
        open_items.append(f"{item}: drop not applied; run pool --apply")
    for key, r in sorted(resolved.items()):
        if r["action"] in ("add-gold", "add-evidence") and key not in applied:
            open_items.append(f"{key[0]} {key[1]}: {r['action']} resolution not applied; run `pool --apply`")
    unreviewed = 0
    for r in audit:
        if r.get("verdict") is None:
            unreviewed += 1
        elif r["verdict"] == "disagree" and (r["item"], r["candidate"]) not in resolved:
            open_items.append(f"{r['item']} {r['candidate']}: disagreed audit with no resolution")
    if unreviewed:
        open_items.append(f"audit: {unreviewed} sampled noes have no reviewer verdict")
    for split in schema.SPLITS:
        noes = pool.current_noes(ds, [i for i in items if i["split"] == split], cands, judgments)
        if noes is None:
            continue
        ids_ = {(j["item"], j["candidate"]) for j in noes}
        have = len([a for a in audit if (a["item"], a["candidate"]) in ids_])
        need = pool.audit_need(len(noes))
        if have < need:
            open_items.append(f"audit {split}: {have} of {need} sampled noes; run pool")
    return open_items


# --- freeze, verify, gates -----------------------------------------------------------------------------------------

def _tree(dir: Path) -> dict[str, str]:
    files = {}
    for path in dir.rglob("*"):
        if path.is_file() and not path.is_symlink() and path.name not in IGNORED_FILES:
            files[path.relative_to(dir).as_posix()] = sha256_file(path)
    return dict(sorted(files.items()))


def _symlinks(dir: Path) -> list[str]:
    return [f"{path.relative_to(dir).as_posix()}: a symlink; a dataset holds regular files only"
            for path in sorted(dir.rglob("*")) if path.is_symlink()]


def _manifest_text(tree: dict[str, str]) -> str:
    return "".join(f"{h}  {p}\n" for p, h in tree.items())


def verify(dir: Path) -> tuple[str, list[str]]:
    dir = Path(dir)
    manifest, frozen = dir / "MANIFEST", dir / "FROZEN"
    if not manifest.is_file() or not frozen.is_file():
        missing = [n for n, p in (("MANIFEST", manifest), ("FROZEN", frozen)) if not p.is_file()]
        return "", [f"{n}: missing" for n in missing]
    problems = []
    manifest_bytes = manifest.read_bytes()
    tree_hash = sha256_bytes(manifest_bytes)
    if frozen.read_text(encoding="utf-8").strip() != tree_hash:
        problems.append("FROZEN: does not match the SHA-256 of MANIFEST")
    listed = {}
    for line in manifest_bytes.decode("utf-8").splitlines():
        digest, _, path = line.partition("  ")
        listed[path] = digest
    actual = _tree(dir)
    for path in sorted(listed.keys() | actual.keys()):
        if path not in actual:
            problems.append(f"{path}: listed in MANIFEST, missing")
        elif path not in listed:
            problems.append(f"{path}: not listed in MANIFEST, extra")
        elif actual[path] != listed[path]:
            problems.append(f"{path}: differs from MANIFEST")
    problems += _symlinks(dir)
    return tree_hash, problems


def require_ready(dir: Path, draft: bool, split: str) -> str | None:
    dir = Path(dir)
    if draft:
        if split == "test":
            raise Refused("a draft run on the test split is refused: the test split runs only on a frozen dataset")
        if not (dir / "FROZEN").is_file():
            return None
    elif not (dir / "FROZEN").is_file():
        raise Refused(f"{dir} is not frozen; freeze it, or pass --draft to run on the dev split of a draft")
    tree_hash, problems = verify(dir)
    if problems:
        raise Refused(f"{dir} does not match its MANIFEST:\n" + "\n".join(problems))
    return tree_hash


def materialize(ds: Dataset, store: Path) -> dict[str, str]:
    """Copy `store/` into `store` and map the absolute path of every note and source file to its id."""
    store = Path(store)
    shutil.copytree(ds.dir / "store", store, symlinks=False, dirs_exist_ok=True)
    real = store.resolve()
    mapping: dict[str, str] = {}
    for root in {store, real}:
        for n in ds.notes.values():
            mapping[str(root / "notes" / n.file)] = n.id
        for s in ds.sources.values():
            mapping[str(root / "library" / f"{s.ref}.md")] = s.ref
    return mapping


def _readme_problems(dir: Path) -> None:
    readme = dir / "README.md"
    if not readme.is_file():
        raise Refused("README.md is missing: write it before the freeze")
    text = readme.read_text(encoding="utf-8")
    problems = [] if CANARY in text else ["README.md: lacks the canary"]
    library = dir / "world/library.json"
    licences = sorted({r["licence"] for r in json.loads(library.read_text(encoding="utf-8")) if r.get("licence")}) if library.is_file() else []
    problems += [f"README.md: lacks the licence {x} of a library document" for x in licences if x not in text]
    if problems:
        raise Refused("\n".join(problems))


def stamp_canary(dir: Path) -> None:
    """Add `"canary"` to every row of a `*.jsonl` file that lacks it; a file whose rows all carry it is left alone."""
    for path in sorted(dir.rglob("*.jsonl")):
        if path.is_symlink() or not path.is_file():
            continue
        rows = read_jsonl(path)
        if all("canary" in r for r in rows if isinstance(r, dict)):
            continue
        write_jsonl(path, [{**r, "canary": CANARY} if isinstance(r, dict) and "canary" not in r else r for r in rows])


def freeze(dir: Path) -> str:
    from bilbo_evals import review

    dir = Path(dir)
    if (dir / "FROZEN").exists():
        raise Refused(f"{dir} is already frozen")
    if not (dir / "preregistration.json").is_file():
        raise Refused("preregistration.json is missing: run `bilbo-evals power` on a dev run first")
    _readme_problems(dir)
    stamp_canary(dir)
    build_corpus(dir)
    build_qrels(dir)
    problems = check(load(dir))
    if problems:
        raise Refused("dataset check fails:\n" + "\n".join(problems))
    summary, review_problems = review.evaluate(dir, "all")
    if review_problems:
        raise Refused(f"the validity review has not passed ({summary}):\n" + "\n".join(review_problems))
    open_items = pool_open_items(dir)
    if open_items:
        raise Refused("unresolved pooled items:\n" + "\n".join(open_items))
    text = _manifest_text(_tree(dir))
    (dir / "MANIFEST").write_text(text, encoding="utf-8")
    tree_hash = sha256_bytes(text.encode("utf-8"))
    (dir / "FROZEN").write_text(tree_hash + "\n", encoding="utf-8")
    return tree_hash


def cmd_freeze(args: argparse.Namespace) -> int:
    out(freeze(args.dir))
    return 0


def cmd_verify(args: argparse.Namespace) -> int:
    tree_hash, problems = verify(args.dir)
    if problems:
        for p in problems:
            out(p)
        return 1
    out(tree_hash)
    return 0
