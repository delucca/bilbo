"""Search a dense store unpacked in /work/store for the questions in /work/questions.jsonl: embed each form of a question
with the build's embedder, after Qwen3's query instruction and the build's sentinel, and rank the chunks of the
question's repository by cosine, best first, each opening with its session's start time from the corpus. Appends one
row per question and form to /work/search.jsonl. build-store.sh --search runs it in the build service."""

import http.client
import json
import os
import sys
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import urlparse

import numpy as np

STORE = Path("/work/store")
CORPUS = Path("/corpus")
TOP_K = 100
# Qwen3-Embedding's query format, from its model card; chunks were embedded with none, as the card says for documents.
TASK = "Given a question about a coding project, retrieve passages of past coding sessions that answer it"

manifest = json.loads((STORE / "manifest.json").read_text())
sentinel, dims = manifest["chunking"]["sentinel"], manifest["embedder"]["dims"]
vectors = np.load(STORE / "vectors.npy")
starts = {}
chunks = [json.loads(line) for line in (STORE / "chunks.jsonl").open(encoding="utf-8")]
if len(chunks) != len(vectors):
    sys.exit(f"search: {len(chunks)} chunks for {len(vectors)} vectors")

by_repo = {}
for i, c in enumerate(chunks):
    by_repo.setdefault(c["repo"], []).append(i)
by_repo = {r: (np.array(ids), vectors[ids].astype(np.float32)) for r, ids in by_repo.items()}


def started(repo, session):
    """The session's first timestamp in the corpus, which the chunks do not carry; Cursor's transcripts have none."""
    if (repo, session) not in starts:
        path = CORPUS / repo / session
        when = None
        for line in path.open(encoding="utf-8"):
            stamp = json.loads(line).get("timestamp") if line.strip() else None
            if stamp:
                when = datetime.fromisoformat(stamp).astimezone(timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
                break
        starts[repo, session] = when
    return starts[repo, session]


def dated(repo, chunk):
    when = started(repo, chunk["session"])
    return f"Session started {when}:\n{chunk['text']}" if when else f"Session date unknown:\n{chunk['text']}"


def embed(text):
    conn = http.client.HTTPConnection(urlparse(os.environ["EMBEDDER_URL"]).netloc, timeout=600)
    body = {"input": [sentinel, f"Instruct: {TASK}\nQuery:{text}"]}
    conn.request("POST", "/v1/embeddings", json.dumps(body), {"Content-Type": "application/json"})
    r = conn.getresponse()
    raw = r.read()
    if r.status != 200:
        sys.exit(f"search: POST /v1/embeddings: {r.status} {raw[:300]!r}")
    data = sorted(json.loads(raw)["data"], key=lambda d: d["index"])
    if len(data) != 2 or len(data[1]["embedding"]) != dims:
        sys.exit("search: the embedder did not return one query vector of the store's size")
    v = np.array(data[1]["embedding"], dtype=np.float32)
    return v / np.linalg.norm(v)


def hits(text, repo):
    ids, matrix = by_repo[repo]
    scores = matrix @ embed(text)
    best = np.argsort(-scores, kind="stable")[:TOP_K]
    return [{"text": dated(repo, chunks[ids[k]]), "ref": int(ids[k]), "session": chunks[ids[k]]["session"],
             "lines": chunks[ids[k]]["lines"], "score": float(scores[k])} for k in best]


rows = [json.loads(line) for line in Path("/work/questions.jsonl").read_text().splitlines() if line.strip()]
with open("/work/search.jsonl", "a") as out:
    for i, q in enumerate(rows, 1):
        if q["repo"] not in by_repo:
            sys.exit(f"search: {q['id']}: the store has no chunks of {q['repo']}")
        for form in ("query", "prompt"):
            out.write(json.dumps({"id": q["id"], "form": form, "results": hits(q[form], q["repo"])},
                                 ensure_ascii=False) + "\n")
            out.flush()
        print(f"search: {i}/{len(rows)} {q['id']}", file=sys.stderr)
