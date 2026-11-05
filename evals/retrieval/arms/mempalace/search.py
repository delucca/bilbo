"""Search a MemPalace store unpacked in /work/store for the questions in /work/questions.jsonl, as a developer with one
palace per repository would: the store's palace is split back into a palace for each repository the questions name,
its drawers with their stored vectors in the order the lane filed them, and searched with MemPalace's own
search_memories, as check.py calls it (cosine, then BM25 over the top vector candidates), no wing and no room. Appends
one row per question and form to /work/search.jsonl. build-store.sh --search runs it in the build service."""

import gc
import json
import os
import re
import shutil
import sqlite3
import sys
from collections import defaultdict
from pathlib import Path

import numpy as np

STORE = Path("/work/store")
CORPUS = Path("/corpus")
SPLIT = Path("/tmp/palaces")
API_MODEL = "embedder"
EF_NAME = f"openai_compat_emb_{API_MODEL}"  # mempalace/embedding.py:632-636
LIMIT = 100  # the MCP tool's most (mcp_server/_session.py:447); it defaults to 5, and pack.py cuts at the budget
PAGE = 1000  # chromadb 1.5.9 fails an unbounded get past 32,766 rows; MemPalace pages by 1,000 (palace/mined.py:169-171)
TRANSCRIPT = re.compile(r"^/(Users|home)/[^/]+/\.claude/projects/[^/]+/([0-9a-f-]{36})\.jsonl$")
CHECKPOINT = re.compile(r"^CHECKPOINT:\d{4}-\d{2}-\d{2}\|session:([0-9a-f-]{36})\|")
# The settings the build ran with (build.py CONFIG): MemPalace opens a palace under another embedder without a word.
os.environ.update({"MEMPALACE_CONFIG_DIR": str(STORE / "config"), "MEMPALACE_PALACE_PATH": str(STORE / "palace"),
                   "MEMPALACE_EMBEDDING_MODEL": "openai-compat", "MEMPALACE_EMBEDDING_API_URL": os.environ["EMBEDDER_URL"],
                   "MEMPALACE_EMBEDDING_API_MODEL": API_MODEL, "ANONYMIZED_TELEMETRY": "False"})
Path(os.environ["HOME"]).mkdir(parents=True, exist_ok=True)

from mempalace.palace import get_collection  # noqa: E402
from mempalace.repair import _close_chroma_handles  # noqa: E402
from mempalace.searcher import search_memories  # noqa: E402

manifest = json.loads((STORE / "manifest.json").read_text())
if manifest["arm"] != "mempalace" or manifest["embedder"]["ef_name"] != EF_NAME or not manifest.get("lanes"):
    sys.exit(f"search: the manifest is not a mempalace store built in lanes with {EF_NAME}")
identity = (STORE / "palace" / "mempalace_embedder.json").read_text()
markers = [p for p in (STORE / "palace").iterdir() if p.is_file() and p.name.startswith(".")]

rows = [json.loads(line) for line in Path("/work/questions.jsonl").read_text().splitlines() if line.strip()]
by_repo = defaultdict(list)
for q in rows:
    by_repo[q["repo"]].append(q)
if unknown := set(by_repo) - set(manifest["lanes"]):
    sys.exit(f"search: the store has no lane for {sorted(unknown)}")

# A drawer's repository, which the merge does not record: a transcript or sweep drawer's by the session its transcript
# path names, a diary drawer's by the session its checkpoint names (a chunked entry's later chunks by their entry's).
session_repo = {p.stem.split("--")[-1]: p.parent.name for p in CORPUS.glob("*/*.jsonl")}
db = sqlite3.connect(f"file:{STORE / 'palace' / 'chroma.sqlite3'}?mode=ro&immutable=1", uri=True)
order = [i for (i,) in db.execute("SELECT embedding_id FROM embeddings ORDER BY id")]  # the merge's insertion order
db.close()
col = get_collection(str(STORE / "palace"), create=False, read_only=True)  # raises on a recorded identity mismatch
if col.count() != len(order) or col.count() != manifest["drawers"]["total"]:
    sys.exit(f"search: the palace holds {col.count()} drawers, its rows {len(order)}, the manifest {manifest['drawers']['total']}")
entry_repo, metas = {}, {}
for b in range(0, len(order), PAGE):
    page = col.get(ids=order[b : b + PAGE], include=["metadatas", "documents"])
    for i, m, doc in zip(page["ids"], page["metadatas"], page["documents"]):
        metas[i] = m
        if m.get("type") == "diary_entry" and m.get("chunk_index") == 0 and (found := CHECKPOINT.match(doc)):
            entry_repo[m.get("parent_entry_id") or i] = session_repo.get(found.group(1))
repo_ids = defaultdict(list)
for i in order:
    m = metas[i]
    if m.get("type") == "diary_entry":
        repo = entry_repo.get(m.get("parent_entry_id") or i)
    else:
        found = TRANSCRIPT.match(m.get("source_file") or "")
        repo = session_repo.get(found.group(2)) if found else None
    if repo is None:
        sys.exit(f"search: no repository for the drawer {i}")
    repo_ids[repo].append(i)
del metas
for lane, facts in manifest["lanes"].items():
    if len(repo_ids[lane]) != facts["drawers"]["total"]:
        sys.exit(f"search: {lane} has {len(repo_ids[lane])} drawers, its lane {facts['drawers']['total']}")


def split(repo):
    """The repository's palace: a fresh one MemPalace's get_collection creates, its drawers added 1,000 at a time as
    its miner upserts, with their stored vectors (never embedded again); then the store's embedder identity and the
    marker files MemPalace writes on opening a palace, as the merge leaves them."""
    path = SPLIT / repo
    shutil.rmtree(path, ignore_errors=True)
    out = get_collection(str(path), create=True)
    for b in range(0, len(repo_ids[repo]), PAGE):
        want = repo_ids[repo][b : b + PAGE]
        page = col.get(ids=want, include=["metadatas", "documents", "embeddings"])
        at = {i: n for n, i in enumerate(page["ids"])}
        out.add(ids=want, documents=[page["documents"][at[i]] for i in want], metadatas=[page["metadatas"][at[i]] for i in want],
                embeddings=[np.asarray(page["embeddings"][at[i]], dtype=np.float32).tolist() for i in want])
    if out.count() != len(repo_ids[repo]):
        sys.exit(f"search: {repo}'s palace holds {out.count()} drawers, not {len(repo_ids[repo])}")
    del out
    (path / "mempalace_embedder.json").write_text(identity)
    for marker in markers:
        if not (path / marker.name).exists():
            shutil.copy2(marker, path / marker.name)
    return path


def kind(hit):
    return "diary" if hit["room"] == "diary" else "transcript" if hit["wing"] == "sessions" else "sweep"


def render(hit):
    return f"{hit['text'].strip()} [filed {hit['filed_at'][:10]}]"


searched = 0
with open("/work/search.jsonl", "a") as out:
    for repo, questions in sorted(by_repo.items()):
        path = split(repo)
        print(f"search: {repo}: a palace of {len(repo_ids[repo])} drawers", file=sys.stderr)
        for q in questions:
            for form in ("query", "prompt"):
                found = search_memories(q[form], str(path), n_results=LIMIT)
                if "error" in found or not found.get("results"):
                    sys.exit(f"search: {q['id']} {form}: {found.get('error') or 'MemPalace found nothing'}")
                hits = [{"text": render(h), "ref": h["drawer_id"], "kind": kind(h),
                         "similarity": h["similarity"]} for h in found["results"]]
                out.write(json.dumps({"id": q["id"], "form": form, "results": hits}, ensure_ascii=False) + "\n")
                out.flush()
            searched += 1
            print(f"search: {searched}/{len(rows)} {q['id']}", file=sys.stderr)
        _close_chroma_handles(str(path))
        gc.collect()
        shutil.rmtree(path)
