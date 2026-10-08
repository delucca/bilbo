"""Build the invented fixture dataset into an empty folder, freeze it and print its tree hash.

    uv run python tests/fixtures/build_fixture.py [--out DIR] [--bilbo PATH] [--fetched YYYY-MM-DD] [--force]
    uv run python tests/fixtures/build_fixture.py --check     # rebuild in a temp folder and compare with the committed one

Run by hand; the output is committed. Every id and date is fixed, so a rebuild is byte-identical.
"""

from __future__ import annotations

import argparse
import json
import os
import random
import shutil
import subprocess
import sys
import tempfile
from datetime import datetime
from pathlib import Path

from bilbo_evals import dataset, review
from bilbo_evals.common import CANARY, REPO_ROOT, Refused, read_jsonl, sha256_bytes, write_json, write_jsonl
from bilbo_evals.generate import filter as leak_filter

HERE = Path(__file__).resolve().parent
DEFAULT_OUT = HERE / "notes-fixture"
SEED = 20261007
CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"
GEN = {"cli": "fixture", "model": "fixture", "prompt_sha256": "0" * 64}

# (project, file, kind, created, lang, fact statement, verbatim, text)
NOTES = [
    ("alpha", "decision-retry-limit.md", "decision", "2026-03-02T10:14-03:00", "en",
     "The sync worker retries a failed push 3 times.", ["sync.max_retries = 3"],
     "# Retry limit for the sync worker\n\nThe sync worker retries a failed push 3 times before it gives up.\n\n"
     "## Setting\n\n`sync.max_retries = 3` in `/etc/alpha/sync.toml`.\n"),
    ("alpha", "decision-retry-limit-raised.md", "decision", "2026-04-20T15:40-03:00", "en",
     "The sync worker retries a failed push 5 times.", ["sync.max_retries"],
     "# Retry limit raised\n\nThis replaces decision-retry-limit. Pushes were failing during short outages, "
     "so `sync.max_retries` is now 5.\n"),
    ("alpha", "gotcha-edge-cache-eviction.md", "gotcha", "2026-03-18T09:05-03:00", "en",
     "edge-cache evicts every entry above 900 MB and logs ERR_EVICT_STORM.", ["ERR_EVICT_STORM", "cache.high_water = 700MB"],
     "# edge-cache drops every entry under memory pressure\n\nWhen the resident size passes 900 MB the cache evicts "
     "everything at once and logs `ERR_EVICT_STORM`.\n\n## Fix\n\nSet `cache.high_water = 700MB` so eviction starts "
     "early and stays gradual.\n"),
    ("alpha", "reference-edge-cache-ports.md", "reference", "2026-03-20T14:22-03:00", "en",
     "edge-cache listens on 7421 (admin) and 7422 (data).", ["7421", "7422"],
     "# edge-cache ports\n\nThe admin socket listens on 7421. The data socket listens on 7422.\n"),
    ("alpha", "research-lantern-rename.md", "research", "2026-01-27T16:30-03:00", "en",
     "Lantern is the old name of edge-cache.", ["Lantern"],
     "# Lantern is now edge-cache\n\nLantern, the old name of edge-cache, was dropped in January. Older dashboards "
     "and runbooks still say Lantern.\n"),
    ("alpha", "plan-migracao-cache.md", "plan", "2026-05-12T08:50-03:00", "pt",
     "O cache migra na sexta; a porta 7422 reabre no fim.", ["7422"],
     "# Plano de migração do cache\n\nNa sexta o cache vai para o novo cluster.\n\n## Passos\n\n"
     "1. Congelar as escritas no edge-cache.\n2. Copiar o snapshot para o novo cluster.\n"
     "3. Reabrir a porta 7422 e liberar as escritas.\n"),
    ("beta", "decision-batch-size.md", "decision", "2026-02-11T09:00-03:00", "en",
     "The importer loads rows in batches of 200.", ["import.batch_rows = 200"],
     "# Batch size for the importer\n\nThe importer loads rows in batches of 200.\n\n## Setting\n\n"
     "`import.batch_rows = 200`.\n"),
    ("beta", "decision-batch-size-lowered.md", "decision", "2026-05-06T11:30-03:00", "en",
     "The importer loads rows in batches of 50.", ["import.batch_rows"],
     "# Batch size lowered\n\nThis replaces decision-batch-size. Large batches collided, so `import.batch_rows` "
     "is now 50.\n"),
    ("beta", "gotcha-importer-row-lock.md", "gotcha", "2026-05-02T17:10-03:00", "en",
     "Overlapping imports fail with ERR_ROW_LOCK; take pg_advisory_lock(42).", ["ERR_ROW_LOCK", "pg_advisory_lock(42)"],
     "# importer fails when two imports overlap\n\nOverlapping imports stop with `ERR_ROW_LOCK` on the orders table."
     "\n\n## Fix\n\nTake the advisory lock first: `SELECT pg_advisory_lock(42)`.\n"),
    ("beta", "reference-importer-paths.md", "reference", "2026-02-14T10:00-03:00", "en",
     "The importer reads /srv/beta/inbox and moves finished files to /srv/beta/done.", ["/srv/beta/inbox", "/srv/beta/done"],
     "# importer folders\n\nThe importer reads files from `/srv/beta/inbox` and moves finished ones to "
     "`/srv/beta/done`.\n"),
    ("beta", "research-conveyor-rename.md", "research", "2026-01-30T13:15-03:00", "en",
     "Conveyor is the old name of the importer.", ["Conveyor"],
     "# Conveyor is now importer\n\nConveyor, the old name of the importer, was retired in January. Old cron "
     "entries still say Conveyor.\n"),
    ("beta", "plan-janela-importacao.md", "plan", "2026-05-20T07:45-03:00", "pt",
     "A importação roda às 03:00 e a pasta /srv/beta/inbox reabre depois.", ["/srv/beta/inbox", "03:00"],
     "# Janela de importação\n\nA importação roda de madrugada, às 03:00.\n\n## Passos\n\n"
     "1. Pausar o cron do importer.\n2. Esperar os lotes terminarem.\n3. Reabrir `/srv/beta/inbox` para novos arquivos.\n"),
]

LICENCE = "public-domain"
SOURCES = [
    ("demo/wal", "wal", "Demo WAL", "https://fixture.invalid/demo/wal",
     "How the write-ahead log and its checkpoints work.",
     "# Demo WAL\n\nIntro nav line.\n\n## Write-ahead log\n\nThe log records every change before the pages change.\n\n"
     "### Checkpoints\n\nA checkpoint copies pages back into the database file. It runs when the log passes "
     "1000 pages.\n"),
    ("demo/busy", "busy", "Demo busy timeout", "https://fixture.invalid/demo/busy",
     "How the busy timeout makes a writer wait.",
     "# Demo busy timeout\n\nIntro nav line.\n\n## Busy timeout\n\nA writer that meets a lock waits up to the timeout "
     "before it reports `SQLITE_BUSY`.\n\n### Setting the timeout\n\nThe timeout is zero by default; set it in "
     "milliseconds right after opening the connection.\n"),
]
GUIDE_CREATED = "2026-01-15T00:00-03:00"


def ulid(created: str, key: str) -> str:
    ms = int(datetime.fromisoformat(created).timestamp() * 1000)
    rnd = random.Random(key).getrandbits(80)
    value = (ms << 80) | rnd
    return "".join(CROCKFORD[(value >> (5 * i)) & 31] for i in reversed(range(26)))


def prompt_row(pid, text, split, label, gold, project):
    return {"id": pid, "prompt": text, "split": split, "label": label, "gold": gold, "project": project, "canary": CANARY}


def sandbox_env(root: Path, store: Path) -> dict[str, str]:
    home = root / "home"
    (root / "config").mkdir(parents=True)
    (root / "config/config").write_text("", encoding="utf-8")
    return {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": str(home), "BILBO_HOME": str(store),
        "BILBO_CONFIG": str(root / "config/config"), "XDG_CONFIG_HOME": str(root / "xdg-config"),
        "XDG_DATA_HOME": str(root / "xdg-data"), "XDG_CACHE_HOME": str(root / "xdg-cache"),
        "XDG_STATE_HOME": str(root / "xdg-state"), "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8", "TMPDIR": str(root),
    }


def bilbo(exe: Path, env: dict[str, str], *args: str) -> str:
    p = subprocess.run([str(exe), *args], env=env, capture_output=True, text=True, encoding="utf-8")
    if p.returncode != 0:
        raise Refused(f"bilbo {' '.join(args)} exited {p.returncode}: {(p.stderr + p.stdout).strip()}")
    return p.stdout


def write_store(out: Path, exe: Path, fetched: str) -> dict[str, str]:
    """Notes by hand with fixed ids; sources through `bilbo library stage` and `land`. Returns note ids by file."""
    store = out / "store"
    (store / "notes").mkdir(parents=True)
    ids = {}
    for project, file, kind, created, lang, *_rest, text in NOTES:
        nid = ulid(created, file)
        ids[file] = nid
        (store / "notes" / file).write_text(f"---\nid: {nid}\ncreated: {created}\n---\n\n{text}", encoding="utf-8")
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        env = sandbox_env(root, store)
        for ref, name, title, url, _guide, text in SOURCES:
            sid = ulid(GUIDE_CREATED, ref)
            page = root / f"{name}.md"
            page.write_text(text, encoding="utf-8")
            lines = len(text.splitlines())
            stage = bilbo(exe, env, "library", "stage", str(page), "--origin", f"url: {url}", "--fetched", fetched)
            stage_id = stage.splitlines()[0].split(": ", 1)[1].strip()
            landed = bilbo(exe, env, "library", "land", stage_id, ref, "--keep", f"3-{lines}", "--title", title)
            old = next(l.split(": ", 1)[1] for l in landed.splitlines() if l.startswith("id: "))
            src = store / "library" / f"{ref}.md"
            src.write_text(src.read_text(encoding="utf-8").replace(old, sid), encoding="utf-8")
            for landed_file in (store / ".bilbo/captures").glob("*/landed"):
                content = landed_file.read_text(encoding="utf-8")
                if old in content:
                    digest = content.split("\t")[1]
                    landed_file.write_text(f"{sid}\t{digest}\t{fetched}\n", encoding="utf-8")
        entries = "".join(f"## {name}\n\n{guide}\n\n" for _r, name, _t, _u, guide, _x in sorted(SOURCES, key=lambda s: s[1]))
        (store / "library/demo/guide.md").write_text(
            f"---\nid: {ulid(GUIDE_CREATED, 'demo/guide')}\ncreated: {GUIDE_CREATED}\n---\n\n# demo\n\nInvented pages that ground the fixture's "
            f"library queries.\n\n{entries}".rstrip("\n") + "\n", encoding="utf-8")
        lock = store / "library/.lock"
        lock.unlink(missing_ok=True)
        leftovers = bilbo(exe, env, "check")
        if leftovers.strip():
            raise Refused("bilbo check on the fixture store prints:\n" + leftovers)
    return ids


def build(out: Path, exe: Path, fetched: str) -> str:
    out.mkdir(parents=True, exist_ok=True)
    nid = write_store(out, exe, fetched)
    by_file = {n[1]: n for n in NOTES}
    A = {k: nid[f"{k}.md"] for k in ("decision-retry-limit", "decision-retry-limit-raised", "gotcha-edge-cache-eviction",
                                     "reference-edge-cache-ports", "research-lantern-rename", "plan-migracao-cache")}
    B = {k: nid[f"{k}.md"] for k in ("decision-batch-size", "decision-batch-size-lowered", "gotcha-importer-row-lock",
                                     "reference-importer-paths", "research-conveyor-rename", "plan-janela-importacao")}
    a1, a2, a3, a4, a5, a6 = A.values()
    b1, b2, b3, b4, b5, b6 = B.values()

    # world
    facts, notes_rows = [], []
    for project in ("alpha", "beta"):
        for i, (proj, file, kind, created, lang, statement, verbatim, _t) in enumerate((n for n in NOTES if n[0] == project), 1):
            fid = f"f-{project}-{i:03d}"
            notes_rows.append({
                "id": nid[file], "file": file, "kind": kind, "topic": file[len(kind) + 1:-3], "project": project,
                "lang": lang, "created": created, "facts": [fid], "filler": False, "noise": [],
                "near_duplicate_of": None, "render_attempts": 1, "status": "kept",
            })
            facts.append({
                "id": fid, "project": project, "component": "edge-cache" if project == "alpha" else "importer",
                "family": f"fam-{project}-{(i + 1) // 2 if i <= 2 else i - 1:03d}", "kind": kind, "statement": statement,
                "verbatim": verbatim, "lang": lang, "valid_from": created, "supersedes": None, "superseded_by": None,
                "joins": [], "kind_pair": None, "bridge": None, "note_id": nid[file], "status": "planted",
            })
    fx = {f["id"]: f for f in facts}
    for p, (old, new) in (("alpha", (1, 2)), ("beta", (1, 2))):
        fx[f"f-{p}-{old:03d}"]["superseded_by"] = f"f-{p}-{new:03d}"
        fx[f"f-{p}-{new:03d}"]["supersedes"] = f"f-{p}-{old:03d}"
        fx[f"f-{p}-{new:03d}"]["family"] = fx[f"f-{p}-{old:03d}"]["family"]
    fx["f-alpha-003"]["kind_pair"], fx["f-alpha-004"]["kind_pair"] = "f-alpha-004", "f-alpha-003"
    fx["f-beta-003"]["kind_pair"], fx["f-beta-004"]["kind_pair"] = "f-beta-004", "f-beta-003"
    fx["f-alpha-005"]["bridge"] = {"alias": "Lantern", "canonical": "edge-cache"}
    fx["f-beta-005"]["bridge"] = {"alias": "Conveyor", "canonical": "importer"}
    fx["f-alpha-006"]["joins"], fx["f-beta-006"]["joins"] = ["f-alpha-004"], ["f-beta-004"]
    write_jsonl(out / "world/facts.jsonl", facts)
    write_jsonl(out / "world/notes.jsonl", notes_rows)
    aliases = [
        {"project": "alpha", "component": "edge-cache", "canonical": "edge-cache", "alias": "Lantern", "type": "old-name", "bridge_notes": [a5]},
        {"project": "beta", "component": "importer", "canonical": "importer", "alias": "Conveyor", "type": "old-name", "bridge_notes": [b5]},
    ]
    write_json(out / "world/aliases.json", aliases)
    write_json(out / "world/noise.json", {"omission": [], "near-duplicate": [], "stale": [[a1, a2], [b1, b2]]})
    write_json(out / "world/profile.json", {"note_count": 10, "kinds": {"decision": 10, "gotcha": 10, "reference": 10}, "median_words": 30})
    write_json(out / "world/world.json", {"seed": SEED, "projects": [
        {"slug": "alpha", "name": "Alpha", "summary": "Invented sync service.", "technologies": ["Go"],
         "components": [{"slug": "edge-cache", "name": "edge-cache", "aliases": [{"alias": "Lantern", "type": "old-name"}]}], "candidate_facts": []},
        {"slug": "beta", "name": "Beta", "summary": "Invented importer.", "technologies": ["PostgreSQL"],
         "components": [{"slug": "importer", "name": "importer", "aliases": [{"alias": "Conveyor", "type": "old-name"}]}], "candidate_facts": []},
    ]})
    write_json(out / "world/splits.json", {
        "seed": SEED, "dev": ["alpha"], "test": ["beta"], "library": {"dev": ["demo/wal"], "test": ["demo/busy"]},
        "none_prompts": {"dev": ["p-none-001", "p-none-002"], "test": ["p-none-003", "p-none-004"]},
    })
    sources_meta = []
    captures = {}
    for landed in (out / "store/.bilbo/captures").glob("*/landed"):
        captures[landed.read_text(encoding="utf-8").split("\t")[0]] = sha256_bytes((landed.parent / "capture.md").read_bytes())
    for ref, name, title, url, guide, text in SOURCES:
        staged = captures[ulid(GUIDE_CREATED, ref)]
        sources_meta.append({"ref": ref, "url": url, "licence": LICENCE, "licence_evidence": "invented for the fixture",
                             "keep": f"3-{len(text.splitlines())}", "stage_sha256": staged, "guide_entry": guide})
    write_json(out / "world/library.json", sources_meta)

    # queries
    def q(qid, text, stratum, split, lang, project, family, gold, *, evidence=(), decoys=(), kind=None, facts_=(), heading=None):
        return {
            "id": qid, "text": text, "stratum": stratum, "split": split, "lang": lang, "project": project,
            "family": family, "gold": gold, "evidence_sets": [list(e) for e in evidence], "decoys": list(decoys),
            "kind": kind, "fact_ids": list(facts_), "zero_overlap": None, "gold_heading": heading,
            "gen": {**GEN, "call_id": f"queries/{qid}/1"}, "canary": CANARY,
        }
    queries = [
        q("q-alpha-001", "edge-cache ERR_EVICT_STORM eviction", "known-item", "dev", "en", "alpha", "fam-alpha-002", [a3], facts_=["f-alpha-003"]),
        q("q-alpha-002", "which two endpoints does the cache expose for management and traffic", "paraphrase", "dev", "en", "alpha", "fam-alpha-003", [a4], facts_=["f-alpha-004"]),
        q("q-alpha-003", "qual é a porta de dados do cache", "pt-en", "dev", "pt", "alpha", "fam-alpha-003", [a4], facts_=["f-alpha-004"]),
        q("q-alpha-004", "what port does Lantern listen on", "alias", "dev", "en", "alpha", "fam-alpha-003", [a4], facts_=["f-alpha-004", "f-alpha-005"]),
        q("q-alpha-005", "what is the current retry limit for the sync worker", "supersession", "dev", "en", "alpha", "fam-alpha-001", [a2], decoys=[a1], facts_=["f-alpha-002", "f-alpha-001"]),
        q("q-alpha-006", "which port reopens after the cache migration freeze and what does it serve", "multi-hop", "dev", "en", "alpha", "fam-alpha-005", [a6, a4], evidence=[[a6, a4]], facts_=["f-alpha-006", "f-alpha-004"]),
        q("q-alpha-007", "reference for edge-cache ports", "kind-filter", "dev", "en", "alpha", "fam-alpha-003", [a4], decoys=[a3], kind="reference", facts_=["f-alpha-004"]),
        q("q-alpha-na-01", "how is TLS terminated on the billing gateway", "no-answer", "dev", "en", "alpha", "fam-alpha-na-01", []),
        q("q-beta-001", "importer ERR_ROW_LOCK overlapping imports", "known-item", "test", "en", "beta", "fam-beta-002", [b3], facts_=["f-beta-003"]),
        q("q-beta-002", "where does the loader pick up and put away files", "paraphrase", "test", "en", "beta", "fam-beta-003", [b4], facts_=["f-beta-004"]),
        q("q-beta-003", "at what hour does the nightly import run", "pt-en", "test", "en", "beta", "fam-beta-005", [b6], facts_=["f-beta-006"]),
        q("q-beta-004", "which directory does Conveyor take its input from", "alias", "test", "en", "beta", "fam-beta-003", [b4], facts_=["f-beta-004", "f-beta-005"]),
        q("q-beta-005", "what is the current batch size of the importer", "supersession", "test", "en", "beta", "fam-beta-001", [b2], decoys=[b1], facts_=["f-beta-002", "f-beta-001"]),
        q("q-beta-006", "which folder reopens after the import window and what is it for", "multi-hop", "test", "en", "beta", "fam-beta-005", [b6, b4], evidence=[[b6, b4]], facts_=["f-beta-006", "f-beta-004"]),
        q("q-beta-007", "reference on importer folders", "kind-filter", "test", "en", "beta", "fam-beta-003", [b4], decoys=[b3], kind="reference", facts_=["f-beta-004"]),
        q("q-beta-na-01", "how are payroll exports signed", "no-answer", "test", "en", "beta", "fam-beta-na-01", []),
        q("q-lib-001", "when does a checkpoint copy pages back to the database", "library", "dev", "en", None, "fam-lib-demo-wal", ["demo/wal"], heading="Write-ahead log > Checkpoints"),
        q("q-lib-002", "how long does a writer wait before it reports busy", "library", "test", "en", None, "fam-lib-demo-busy", ["demo/busy"], heading="Busy timeout"),
    ]
    write_jsonl(out / "queries.jsonl", queries)
    write_jsonl(out / "digest/prompts.jsonl", [
        prompt_row("p-alpha-001", "what was the retry limit for the sync worker?", "dev", "positive", [a2], "alpha"),
        prompt_row("p-alpha-002", "any gotchas with the edge cache eviction?", "dev", "positive", [a3], "alpha"),
        prompt_row("p-none-001", "ok thanks, continue", "dev", "noise", [], None),
        prompt_row("p-none-002", "what is the capital of Australia", "dev", "off-topic", [], None),
        prompt_row("p-alpha-003", "how do I tune the cache of my browser", "dev", "near-miss", [], "alpha"),
        prompt_row("p-beta-001", "what batch size does the importer use now?", "test", "positive", [b2], "beta"),
        prompt_row("p-beta-002", "why does the importer fail with a row lock?", "test", "positive", [b3], "beta"),
        prompt_row("p-none-003", "great, go ahead", "test", "noise", [], None),
        prompt_row("p-none-004", "who wrote Dom Casmurro", "test", "off-topic", [], None),
        prompt_row("p-beta-003", "how do I import a CSV into a spreadsheet", "test", "near-miss", [], "beta"),
    ])

    # generation and the rest
    (out / "generation/pool").mkdir(parents=True)
    for name in ("candidates", "judgments", "audit", "resolutions"):
        (out / f"generation/pool/{name}.jsonl").write_text("", encoding="utf-8")
    (out / "generation/config.toml").write_text(f"seed = {SEED}\nmax_calls = 0\n", encoding="utf-8")
    write_json(out / "preregistration.json", {
        "primary_metric": "success@5", "principal": ["bilbo-full", "bm25-ref"],
        "secondary": ["bilbo-keyword", "ripgrep", "dense-ref", "random"], "min_effect": 0.10, "alpha": 0.05, "power": 0.80,
        "correction": "holm (secondary only)", "test": "paired sign-flip by fact family, 10000 resamples",
        "dev_run": "fixture", "dev_tree_hash": None, "discordance": 0.24, "psi_used": 0.24, "design_effect": 1.3,
        "n_iid": 0, "n_test": 0,
        "per_stratum": {s: 0 for s in ("known-item", "paraphrase", "pt-en", "alias", "supersession", "multi-hop", "kind-filter")}
        | {"library": 20, "no-answer": 20},
        "seed": 0, "resamples": 10000, "created": "2026-10-07T00:00:00Z",
    })
    (out / "README.md").write_text(
        f"# notes-fixture\n\nAn invented dataset for the harness's own tests: 12 notes in two projects, a library corpus of two "
        f"sources, 18 queries, 10 digest prompts. The claim of a real dataset is \"retrieval over curated synthetic notes\"; "
        f"this one claims nothing.\n\nGeneration: written by hand by `tests/fixtures/build_fixture.py`. Models: none.\n\n"
        f"Library documents (invented, licence {LICENCE}): demo/wal, demo/busy.\n\n{CANARY}\n", encoding="utf-8")

    leak_filter.run(out, ["dev", "test"])
    dataset.build_corpus(out)
    dataset.build_qrels(out)
    review.sample(out, "all")
    sheet = read_jsonl(out / "generation/review/sheet.jsonl")
    for row in sheet:
        row["verdict"], row["reviewer"] = "valid", "fixture"
    write_jsonl(out / "generation/review/sheet.jsonl", sheet)
    return dataset.freeze(out)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", type=Path, default=DEFAULT_OUT)
    ap.add_argument("--bilbo", type=Path, default=Path(os.environ.get("BILBO_BIN") or REPO_ROOT / "target/debug/bilbo"))
    ap.add_argument("--fetched", default="2026-01-15")
    ap.add_argument("--force", action="store_true", help="replace a non-empty --out")
    ap.add_argument("--check", action="store_true", help="rebuild in a temp folder and compare with --out")
    args = ap.parse_args()
    try:
        if args.check:
            with tempfile.TemporaryDirectory() as tmp:
                built = Path(tmp) / "notes-fixture"
                build(built, args.bilbo, args.fetched)
                want, got = dataset._tree(built), dataset._tree(args.out)
            diff = sorted(p for p in want.keys() | got.keys() if want.get(p) != got.get(p))
            for p in diff:
                print(f"differs: {p}")
            if diff:
                return 1
            print(f"fixture matches: {(args.out / 'FROZEN').read_text(encoding='utf-8').strip()}")
            return 0
        if args.out.exists() and any(args.out.iterdir()):
            if not args.force:
                raise Refused(f"{args.out} is not empty; pass --force to replace it")
            shutil.rmtree(args.out)
        print(build(args.out, args.bilbo, args.fetched))
        return 0
    except Refused as e:
        print(f"build_fixture: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
