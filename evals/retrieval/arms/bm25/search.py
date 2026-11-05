"""Search a BM25 store unpacked in /work/store for the questions in /work/questions.jsonl: each question's own
repository's index only, tokenized as the build tokenized the chunks (the manifest's `bm25`). Appends one row per
question and form to /work/search.jsonl. build-store.sh --search runs it in the build service."""

import json
import sys
from pathlib import Path

import bm25s
import Stemmer

STORE = Path("/work/store")
TOP_K = 50  # about eight 512-token chunks fill the 4,000-token budget

config = json.loads((STORE / "manifest.json").read_text())["bm25"]
stemmer = Stemmer.Stemmer(config["stemmer"])
chunks = {}
for line in (STORE / "chunks.jsonl").open(encoding="utf-8"):
    row = json.loads(line)
    chunks.setdefault(row["repo"], []).append(row)
indexes = {}


def search(repo, text):
    if repo not in indexes:
        indexes[repo] = bm25s.BM25.load(STORE / "index" / repo)
    rows = chunks[repo]
    query = bm25s.tokenize([text], lower=config["lower"], token_pattern=config["token_pattern"],
                           stopwords=config["stopwords"], stemmer=stemmer, return_ids=False, show_progress=False)
    docs, scores = indexes[repo].retrieve(query, k=min(TOP_K, len(rows)), show_progress=False, backend_selection="numpy")
    return [{"text": rows[d]["text"], "ref": rows[d]["id"], "session": rows[d]["session"], "lines": rows[d]["lines"],
             "score": float(s)} for d, s in zip(docs[0].tolist(), scores[0].tolist()) if s > 0]


questions = [json.loads(line) for line in Path("/work/questions.jsonl").read_text().splitlines() if line.strip()]
with open("/work/search.jsonl", "a") as out:
    for i, q in enumerate(questions, 1):
        if q["repo"] not in chunks:
            sys.exit(f"search: {q['id']}: the store has no chunks for {q['repo']}")
        for form in ("query", "prompt"):
            out.write(json.dumps({"id": q["id"], "form": form, "results": search(q["repo"], q[form])},
                                 ensure_ascii=False) + "\n")
            out.flush()
        print(f"search: {i}/{len(questions)} {q['id']}", file=sys.stderr)
