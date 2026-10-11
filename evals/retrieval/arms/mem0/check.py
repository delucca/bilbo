"""Check a mem0 store unpacked in /work/store: restore it into a fresh Qdrant twice, check its counts against the
manifest, that every memory in mem0's history is in Qdrant with the same text and that both restores give the same
points, and search every lane after each restore. build-store.sh --check runs it in the build service."""

import collections
import json
import os
import sqlite3
import sys
from pathlib import Path

import httpx
from mem0 import Memory
from qdrant_client import QdrantClient

QDRANT = os.environ["QDRANT_URL"]
STORE = Path("/work/store")
COLLECTION = "mem0"
ENTITIES = f"{COLLECTION}_entities"
QUERIES = ("what was decided about the build", "which bug was fixed", "how are tests run", "what did the user prefer")
manifest = json.loads((STORE / "manifest.json").read_text())
client = QdrantClient(url=QDRANT)


def restore():
    for c in client.get_collections().collections:
        client.delete_collection(c.name)
    for f in sorted((STORE / "qdrant").glob("*.snapshot")):
        with f.open("rb") as fh:
            httpx.post(f"{QDRANT}/collections/{f.stem}/snapshots/upload", params={"priority": "snapshot", "wait": "true"},
                       files={"snapshot": (f.name, fh)}, timeout=600).raise_for_status()


def points():
    out, offset = {}, None
    while True:
        batch, offset = client.scroll(COLLECTION, limit=256, offset=offset, with_payload=True, with_vectors=True)
        for p in batch:
            out[str(p.id)] = (p.payload, p.vector)
        if offset is None:
            return out


def searches(lanes):
    mem = Memory.from_config({
        "vector_store": {"provider": "qdrant", "config": {"collection_name": COLLECTION, "url": QDRANT, "embedding_model_dims": 1024}},
        "embedder": {"provider": "openai", "config": {"model": "embedder", "openai_base_url": os.environ["EMBEDDER_URL"], "api_key": "none"}},
        "llm": {"provider": "openai", "config": {"model": "none", "api_key": "none"}},
        "history_db_path": "/tmp/check-history.db",
    })
    results = {}
    for lane in lanes:
        for q in QUERIES:
            hits = mem.search(q, filters=dict([lane]), top_k=10)["results"]
            results[(lane, q)] = [(h["id"], h["score"]) for h in hits]
    return results


restore()
first = points()
by_repo = collections.Counter(f"{p.get('app_id', '')}/{'project' if p.get('agent_id') else 'personal'}" for p, _ in first.values())
entities = client.count(ENTITIES, exact=True).count if client.collection_exists(ENTITIES) else 0
print("points", len(first), "manifest", manifest["memories"]["total"], "| by repo matches", by_repo == manifest["memories"]["by_repo"],
      "| entities", entities, "manifest", manifest["entities"])
lanes = sorted({("agent_id", p["agent_id"]) for p, _ in first.values() if p.get("agent_id")} |
               {("user_id", p["user_id"]) for p, _ in first.values() if p.get("user_id") and not p.get("agent_id")})
print("lanes", len(lanes), lanes)

db = sqlite3.connect(f"file:{STORE / 'history.db'}?mode=ro", uri=True)
events = collections.Counter(e for (e,) in db.execute("SELECT event FROM history"))
added = db.execute("SELECT memory_id, new_memory FROM history WHERE event = 'ADD'").fetchall()
missing = [m for m, _ in added if m not in first]
differ = [m for m, t in added if m in first and first[m][0].get("data") != t]
print("history events", dict(events), "| missing in qdrant", len(missing), "| text differs", len(differ))

first_search = searches(lanes)
restore()
second = points()
second_search = searches(lanes)
same_points = first.keys() == second.keys() and all(first[k] == second[k] for k in first)
# Informational: llama-server reuses a prompt prefix across requests, so scores drift a little between passes.
same_ids = all([i for i, _ in first_search[k]] == [i for i, _ in second_search[k]] for k in first_search)
drift = max((abs(a[1] - b[1]) for k in first_search for a, b in zip(first_search[k], second_search[k])), default=0)
with_hits = sum(bool(v) for s in (first_search, second_search) for v in s.values())
print("second restore: same points", same_points, "| same ranking", same_ids, f"| max score drift {drift:.2g}",
      "| searches with hits", with_hits, "of", 2 * len(first_search))
ok = (len(first) == manifest["memories"]["total"] and by_repo == manifest["memories"]["by_repo"] and entities == manifest["entities"]
      and set(events) == {"ADD"} and len(added) == len(first) and not missing and not differ and same_points
      and with_hits == 2 * len(first_search))
print("OK" if ok else "FAILED")
sys.exit(0 if ok else 1)
