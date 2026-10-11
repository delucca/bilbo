"""Check a dense store unpacked in /work/store: its vectors, chunks and manifest agree, every chunk lies within its
session in the corpus, and five chunks embedded again point the way their stored vectors do, with the same number of
tokens. build-store.sh --check runs it in the build service."""

import http.client
import json
import os
import sys
from collections import Counter
from pathlib import Path
from urllib.parse import urlparse

import numpy as np

STORE = Path("/work/store")
CORPUS = Path("/corpus")
# A chunk embedded on a CPU and on a GPU differs by floating-point noise, down to a cosine of 0.9976 measured; two
# different chunks of the store rarely come above 0.99.
MIN_COSINE = 0.99

manifest = json.loads((STORE / "manifest.json").read_text())
chunking, counts = manifest["chunking"], manifest["chunks"]
vectors = np.load(STORE / "vectors.npy")
rows = [json.loads(line) for line in (STORE / "chunks.jsonl").open(encoding="utf-8")]
n = len(rows)
if not n:
    sys.exit("check: the store has no chunks")


def post(path, body):
    conn = http.client.HTTPConnection(urlparse(os.environ["EMBEDDER_URL"]).netloc, timeout=600)
    conn.request("POST", path, json.dumps(body), {"Content-Type": "application/json"})
    r = conn.getresponse()
    raw = r.read()
    if r.status != 200:
        sys.exit(f"check: POST {path}: {r.status} {raw[:300]!r}")
    return json.loads(raw)


norms = np.linalg.norm(vectors.astype(np.float32), axis=1)
vectors_ok = (vectors.shape == (n, manifest["embedder"]["dims"]) and vectors.dtype == np.dtype("<f2")
              and bool(np.isfinite(norms).all()) and float(np.abs(norms - 1).max()) <= 1e-3)
print("vectors", vectors.shape, vectors.dtype, f"| norms {norms.min():.5f} to {norms.max():.5f}", "| chunks", n)

split = {}
for r in rows:
    if r["part"]:
        split.setdefault((r["repo"], r["session"], r["lines"][0]), []).append(r)
windows_ok = all([p["part"] for p in ps] == [[k, len(ps)] for k in range(len(ps))]
                 and [p["id"] for p in ps] == list(range(ps[0]["id"], ps[0]["id"] + len(ps))) for ps in split.values())
rows_ok = (windows_ok and [r["id"] for r in rows] == list(range(n))
           and all(r["text"] and 0 < r["tokens"] <= chunking["budget"] for r in rows))
packed = sum(r["part"] is None for r in rows)
counts_ok = (counts["total"] == n and counts["packed"] == packed and counts["windows"] == n - packed
             and counts["split_messages"] == len(split) and counts["by_repo"] == Counter(r["repo"] for r in rows)
             and manifest["tokens"]["content"] == sum(r["tokens"] for r in rows))
print("ids, texts, tokens and windows", rows_ok, "| counts match the manifest", counts_ok)

corpus = {(p.parent.name, p.name) for p in CORPUS.glob("*/*.jsonl")}
limit = next(i.split()[1] for i in manifest["inputs"] if i.startswith("sessions "))
want = len(corpus) if limit == "all" else min(len(corpus), int(limit))
sessions = {(r["repo"], r["session"]) for r in rows}
lines = {s: sum(1 for _ in (CORPUS / s[0] / s[1]).open(encoding="utf-8")) for s in sessions & corpus}
in_range = (1 <= r["lines"][0] <= r["lines"][1] <= lines[r["repo"], r["session"]] for r in rows)
lines_ok = sessions <= corpus and all(in_range)
sessions_ok = manifest["sessions"] == want and len(sessions) <= want
print("sessions", manifest["sessions"], "of", want, "| with chunks", len(sessions), "| lines in range", lines_ok)

# Five chunks spread over the store, embedded again as the build sent them: each after the sentinel.
picks = sorted({n * k // 4 for k in range(4)} | {n - 1})
texts = [rows[i]["text"] for i in picks]
data = post("/v1/embeddings", {"input": [t for text in texts for t in (chunking["sentinel"], text)]})["data"]
if len(data) != 2 * len(texts):
    sys.exit(f"check: asked for {2 * len(texts)} embeddings, got {len(data)}")
fresh = np.array([d["embedding"] for d in sorted(data, key=lambda d: d["index"])[1::2]], dtype=np.float32)
stored = vectors[picks].astype(np.float32)
cosines = (fresh * stored).sum(axis=1) / np.linalg.norm(fresh, axis=1) / np.linalg.norm(stored, axis=1)
diffs = np.abs(fresh.astype("<f2").astype(np.float32) - stored).max(axis=1)
tokens = [len(post("/tokenize", {"content": t, "add_special": False})["tokens"]) for t in texts]
for i, c, d, t in zip(picks, cosines, diffs, tokens):
    print(f"chunk {i} ({rows[i]['repo']}, part {rows[i]['part']}, {rows[i]['tokens']} tokens, {t} now): cosine {c:.6f},"
          f" max abs diff {d:.2e}")
embedded_ok = bool((cosines >= MIN_COSINE).all()) and tokens == [rows[i]["tokens"] for i in picks]

ok = vectors_ok and rows_ok and counts_ok and lines_ok and sessions_ok and embedded_ok
print("OK" if ok else "FAILED")
sys.exit(0 if ok else 1)
