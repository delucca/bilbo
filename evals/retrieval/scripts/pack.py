"""Pack an arm's results into one context per question and form. Usage: pack.py <search.jsonl> <contexts.jsonl> <budget>.
Results go in whole, in the tool's order, until one does not fit: that one is cut to the tokens left."""

import hashlib
import json
import sys
from pathlib import Path

import tiktoken

search, contexts, budget = Path(sys.argv[1]), Path(sys.argv[2]), int(sys.argv[3])
enc = tiktoken.get_encoding("o200k_base")


def tokens(text):
    return enc.encode(text, disallowed_special=())


seen, rows = set(), []
for line in search.read_text().splitlines():
    if not line.strip():
        continue
    row = json.loads(line)
    if (row["id"], row["form"]) in seen:
        continue
    seen.add((row["id"], row["form"]))
    parts, left, used, cut = [], budget, 0, False
    for r in row["results"]:
        toks = tokens(r["text"]) if r["text"].strip() else []
        if not toks:
            continue
        if len(toks) > left:
            if left:
                parts.append(enc.decode(toks[:left]))
                used += 1
            left, cut = 0, True
            break
        parts.append(r["text"])
        left -= len(toks)
        used += 1
    text = "\n\n".join(parts)
    rows.append({"id": row["id"], "form": row["form"], "sha": hashlib.sha256(text.encode()).hexdigest(),
                 "tokens": budget - left, "results": used, "cut": cut, "text": text})

tmp = contexts.with_name(contexts.name + ".part")
tmp.write_text("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in rows))
tmp.rename(contexts)
