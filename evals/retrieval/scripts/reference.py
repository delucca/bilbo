"""bm25s itself over the bm25 arm's chunks, for what the comparison with bilbo needs. Run it in the bm25 arm's environment:

  uv run --project arms/bm25 --locked python reference.py scores <store> <questions.jsonl> <out.jsonl>
  uv run --project arms/bm25 --locked python reference.py global <store> <corpus> <questions.jsonl> <out.jsonl>

<store> is an unpacked bm25 store (chunks.jsonl and manifest.json; its index folders are not needed: the indexes are
built again here from the chunks with the manifest's `bm25` settings, as the arm's build.py does).

scores: for each question and form, the question's repository's index, and its 200 best chunks with a score above 0 as
[chunk id, score], in bm25s's order, and every chunk after them that scores what the 200th does (a tie across the cut).
global: one index over every chunk, searched with a weight mask of the question's repository, so the 50 best are that repository's own and the statistics (N, document frequencies, average length) are
the whole corpus's; written as the bm25 arm writes its rows."""

import json
import sys
from datetime import datetime, timezone
from pathlib import Path

import bm25s
import Stemmer

SCORES_K = 200
TOP_K = 50


def load(store):
    config = json.loads((Path(store) / "manifest.json").read_text())["bm25"]
    rows = [json.loads(line) for line in (Path(store) / "chunks.jsonl").open(encoding="utf-8")]
    return config, rows


def tokenize(config, texts, return_ids):
    return bm25s.tokenize(texts, lower=config["lower"], token_pattern=config["token_pattern"],
                          stopwords=config["stopwords"], stemmer=Stemmer.Stemmer(config["stemmer"]),
                          return_ids=return_ids, show_progress=False)


def index(config, texts):
    retriever = bm25s.BM25(method=config["method"], k1=config["k1"], b=config["b"])
    retriever.index(tokenize(config, texts, True), show_progress=False)
    return retriever


def questions_of(path):
    return [json.loads(line) for line in Path(path).read_text().splitlines() if line.strip()]


def retrieve(config, retriever, text, k, mask=None):
    query = tokenize(config, [text], False)
    docs, scores = retriever.retrieve(query, k=k, show_progress=False, backend_selection="numpy", weight_mask=mask)
    return [(d, float(s)) for d, s in zip(docs[0].tolist(), scores[0].tolist()) if s > 0]


def scores(store, questions, out):
    config, rows = load(store)
    by_repo = {}
    for r in rows:
        by_repo.setdefault(r["repo"], []).append(r)
    with open(out, "w") as f:
        indexes = {}
        for i, q in enumerate(questions_of(questions), 1):
            repo = q["repo"]
            if repo not in indexes:
                indexes[repo] = index(config, [r["text"] for r in by_repo[repo]])
            for form in ("query", "prompt"):
                found = retrieve(config, indexes[repo], q[form], len(by_repo[repo]))
                kept = len(found)
                if kept > SCORES_K:
                    kept = SCORES_K
                    while kept < len(found) and found[kept][1] == found[SCORES_K - 1][1]:
                        kept += 1
                found = found[:kept]
                f.write(json.dumps({"id": q["id"], "form": form,
                                    "scores": [[by_repo[repo][d]["id"], s] for d, s in found]}) + "\n")
            print(f"reference: scores {i} {q['id']}", file=sys.stderr)


def started(corpus, repo, session, cache={}):
    """The session's first timestamp, as the bm25 arm's search.py reads it."""
    if (repo, session) not in cache:
        when = None
        for line in (Path(corpus) / repo / session).open(encoding="utf-8"):
            stamp = json.loads(line).get("timestamp") if line.strip() else None
            if stamp:
                when = datetime.fromisoformat(stamp).astimezone(timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
                break
        cache[repo, session] = when
    return cache[repo, session]


def dated(corpus, chunk):
    when = started(corpus, chunk["repo"], chunk["session"])
    return f"Session started {when}:\n{chunk['text']}" if when else f"Session date unknown:\n{chunk['text']}"


def everything(store, corpus, questions, out):
    import numpy as np

    config, rows = load(store)
    retriever = index(config, [r["text"] for r in rows])
    masks = {}
    with open(out, "w") as f:
        for i, q in enumerate(questions_of(questions), 1):
            repo = q["repo"]
            if repo not in masks:
                masks[repo] = np.array([1.0 if r["repo"] == repo else 0.0 for r in rows])
            for form in ("query", "prompt"):
                found = retrieve(config, retriever, q[form], TOP_K, masks[repo])
                results = [{"text": dated(corpus, rows[d]), "ref": rows[d]["id"], "session": rows[d]["session"],
                            "lines": rows[d]["lines"], "score": s} for d, s in found]
                f.write(json.dumps({"id": q["id"], "form": form, "results": results}, ensure_ascii=False) + "\n")
                f.flush()
            print(f"reference: global {i} {q['id']}", file=sys.stderr)


if __name__ == "__main__":
    args = sys.argv[1:]
    if len(args) == 4 and args[0] == "scores":
        scores(*args[1:])
    elif len(args) == 5 and args[0] == "global":
        everything(*args[1:])
    else:
        sys.exit(__doc__)
