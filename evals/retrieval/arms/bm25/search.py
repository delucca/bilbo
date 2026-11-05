"""Search a BM25 store unpacked in /work/store for the questions in /work/questions.jsonl: each question's own
repository's index only, tokenized as the build tokenized the chunks (the manifest's `bm25`), each chunk opening with
its session's start time from the corpus. Appends one row per question and form to /work/search.jsonl.
build-store.sh --search runs it in the build service."""

import json
import sys
from datetime import datetime, timezone
from pathlib import Path

import bm25s
import Stemmer

STORE = Path("/work/store")
CORPUS = Path("/corpus")
TOP_K = 50  # about eight 512-token chunks fill the 4,000-token budget

config = json.loads((STORE / "manifest.json").read_text())["bm25"]
stemmer = Stemmer.Stemmer(config["stemmer"])
chunks = {}
for line in (STORE / "chunks.jsonl").open(encoding="utf-8"):
    row = json.loads(line)
    chunks.setdefault(row["repo"], []).append(row)
indexes = {}
starts = {}


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


def search(repo, text):
    if repo not in indexes:
        indexes[repo] = bm25s.BM25.load(STORE / "index" / repo)
    rows = chunks[repo]
    query = bm25s.tokenize([text], lower=config["lower"], token_pattern=config["token_pattern"],
                           stopwords=config["stopwords"], stemmer=stemmer, return_ids=False, show_progress=False)
    docs, scores = indexes[repo].retrieve(query, k=min(TOP_K, len(rows)), show_progress=False, backend_selection="numpy")
    return [{"text": dated(repo, rows[d]), "ref": rows[d]["id"], "session": rows[d]["session"], "lines": rows[d]["lines"],
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
