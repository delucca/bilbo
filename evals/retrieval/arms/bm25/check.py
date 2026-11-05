"""Check a BM25 store unpacked in /work/store: its chunks and manifest agree, every chunk lies within its session in the
corpus, five chunks have the token counts the embedder's tokenizer gives them now, and each repository's index holds
its chunks and ranks three queries as an index built again from the chunks does. build-store.sh --check runs it in the
build service."""

import http.client
import json
import os
import sys
from collections import Counter
from pathlib import Path
from urllib.parse import urlparse

import bm25s
import numpy as np
import Stemmer

STORE = Path("/work/store")
CORPUS = Path("/corpus")

manifest = json.loads((STORE / "manifest.json").read_text())
chunking, counts, config = manifest["chunking"], manifest["chunks"], manifest["bm25"]
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
print("chunks", n, "| ids, texts, tokens and windows", rows_ok, "| counts match the manifest", counts_ok)

corpus = {(p.parent.name, p.name) for p in CORPUS.glob("*/*.jsonl")}
limit = next(i.split()[1] for i in manifest["inputs"] if i.startswith("sessions "))
want = len(corpus) if limit == "all" else min(len(corpus), int(limit))
sessions = {(r["repo"], r["session"]) for r in rows}
lines = {s: sum(1 for _ in (CORPUS / s[0] / s[1]).open(encoding="utf-8")) for s in sessions & corpus}
in_range = (1 <= r["lines"][0] <= r["lines"][1] <= lines[r["repo"], r["session"]] for r in rows)
lines_ok = sessions <= corpus and all(in_range)
sessions_ok = manifest["sessions"] == want and len(sessions) <= want
print("sessions", manifest["sessions"], "of", want, "| with chunks", len(sessions), "| lines in range", lines_ok)

picks = sorted({n * k // 4 for k in range(4)} | {n - 1})
tokens = [len(post("/tokenize", {"content": rows[i]["text"], "add_special": False})["tokens"]) for i in picks]
tokens_ok = tokens == [rows[i]["tokens"] for i in picks]
print("tokens of chunks", picks, "now", tokens, "| as stored", tokens_ok)


def tokenize(texts, return_ids):
    return bm25s.tokenize(texts, lower=config["lower"], token_pattern=config["token_pattern"],
                          stopwords=config["stopwords"], stemmer=Stemmer.Stemmer(config["stemmer"]),
                          return_ids=return_ids, show_progress=False)


by_repo = {}
for r in rows:
    by_repo.setdefault(r["repo"], []).append(r["text"])
index_ok = sorted(p.name for p in (STORE / "index").iterdir()) == sorted(by_repo) and manifest["terms"].keys() == by_repo.keys()
for repo, texts in sorted(by_repo.items()):
    stored = bm25s.BM25.load(STORE / "index" / repo)
    fresh = bm25s.BM25(method=config["method"], k1=config["k1"], b=config["b"])
    fresh_tokens = tokenize(texts, True)
    fresh.index(fresh_tokens, show_progress=False)
    queries = tokenize([texts[len(texts) * k // 3][:300] for k in range(3)], False)
    k = min(10, len(texts))
    a = stored.retrieve(queries, k=k, show_progress=False, backend_selection="numpy")
    b = fresh.retrieve(queries, k=k, show_progress=False, backend_selection="numpy")
    ok = (stored.scores["num_docs"] == len(texts) and manifest["terms"][repo] == len(fresh_tokens.vocab)
          and np.array_equal(a.documents, b.documents) and np.array_equal(a.scores, b.scores))
    index_ok = index_ok and ok
    print(f"index {repo}: {stored.scores['num_docs']} documents, {manifest['terms'][repo]} terms, ranks as rebuilt {ok}")

ok = rows_ok and counts_ok and lines_ok and sessions_ok and tokens_ok and index_ok
print("OK" if ok else "FAILED")
sys.exit(0 if ok else 1)
