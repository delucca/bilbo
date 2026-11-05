"""Search a mem0 store unpacked in /work/store for the questions in /work/questions.jsonl, the way the plugin's `repo`
scope does: the project lane (agent_id) and the personal lane (user_id), both scoped by app_id. Appends one row per
question and form to /work/search.jsonl. build-store.sh --search runs it in the build service."""

import collections
import json
import os
import re
import sys
from pathlib import Path

import httpx
from mem0 import Memory
from qdrant_client import QdrantClient

QDRANT = os.environ["QDRANT_URL"]
STORE = Path("/work/store")
COLLECTION = "mem0"
TOP_K = 100
NO_LABEL = {"", "main", "master", "unknown", "detached"}
client = QdrantClient(url=QDRANT)

for c in client.get_collections().collections:
    client.delete_collection(c.name)
for f in sorted((STORE / "qdrant").glob("*.snapshot")):
    with f.open("rb") as fh:
        httpx.post(f"{QDRANT}/collections/{f.stem}/snapshots/upload", params={"priority": "snapshot", "wait": "true"},
                   files={"snapshot": (f.name, fh)}, timeout=600).raise_for_status()

agents = collections.defaultdict(set)
offset = None
while True:
    batch, offset = client.scroll(COLLECTION, limit=256, offset=offset, with_payload=True, with_vectors=False)
    for p in batch:
        if p.payload.get("agent_id"):
            agents[p.payload.get("app_id", "")].add(p.payload["agent_id"])
    if offset is None:
        break

mem = Memory.from_config({
    "vector_store": {"provider": "qdrant", "config": {"collection_name": COLLECTION, "url": QDRANT, "embedding_model_dims": 1024}},
    "embedder": {"provider": "openai", "config": {"model": "embedder", "openai_base_url": os.environ["EMBEDDER_URL"], "api_key": "none"}},
    "llm": {"provider": "openai", "config": {"model": "none", "api_key": "none"}},
    "history_db_path": "/tmp/search-history.db",
})


def lane_hits(text, lane, filters, app):
    out = []
    for h in mem.search(text, filters=filters, top_k=TOP_K)["results"]:
        meta = h.get("metadata") or {}
        assert (h.get("app_id") or meta.get("app_id")) == app, f"{h['id']} is not in {app}"
        if meta.get("record_kind") != "task_episode":
            out.append({"text": render(h["memory"], meta), "ref": h["id"], "lane": lane, "score": h["score"]})
    return out


def render(memory, meta):
    text = re.sub(r"\s+", " ", memory).strip()
    branch = meta.get("branch") or ""
    return text if branch.lower() in NO_LABEL else f"{text} [learnt on branch {branch}]"


rows = [json.loads(line) for line in Path("/work/questions.jsonl").read_text().splitlines() if line.strip()]
with open("/work/search.jsonl", "a") as out:
    for i, q in enumerate(rows, 1):
        app = q["repo"].replace("__", "-", 1)
        if len(agents[app]) != 1:
            sys.exit(f"search: {q['id']}: the store has {len(agents[app])} agent_ids for {app}, not one")
        (agent,) = agents[app]
        for form in ("query", "prompt"):
            hits = (lane_hits(q[form], "project", {"agent_id": agent, "app_id": app}, app) +
                    lane_hits(q[form], "personal", {"user_id": "dev", "app_id": app}, app))
            hits.sort(key=lambda r: -r["score"])
            out.write(json.dumps({"id": q["id"], "form": form, "results": hits}, ensure_ascii=False) + "\n")
            out.flush()
        print(f"search: {i}/{len(rows)} {q['id']}", file=sys.stderr)
