"""Pack an arm's results into one context per question and form. Usage: pack.py <search.jsonl> <contexts.jsonl> <budget>.
Results go in whole, in the tool's order, joined by a blank line, until one does not fit: that one is cut to the tokens
left. The separators count toward the budget."""

import hashlib
import json
import sys
from pathlib import Path

import tiktoken

search, contexts, budget = Path(sys.argv[1]), Path(sys.argv[2]), int(sys.argv[3])
enc = tiktoken.get_encoding("o200k_base")
SEP = "\n\n"


def tokens(text):
    return enc.encode(text, disallowed_special=())


sep = len(tokens(SEP))


seen, rows = set(), []
for line in search.read_text().split("\n"):
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
        room = left - (sep if parts else 0)
        if len(toks) > room:
            if room > 0:
                parts.append(enc.decode(toks[:room]))
                used += 1
            cut = True
            break
        parts.append(r["text"])
        left = room - len(toks)
        used += 1
    text = SEP.join(parts)
    # Tokens merge across the joins and a cut part can re-encode longer: the budget holds for the context as judged.
    toks = tokens(text)
    if len(toks) > budget:
        text, toks = enc.decode(toks[:budget]), toks[:budget]
    rows.append({"id": row["id"], "form": row["form"], "sha": hashlib.sha256(text.encode()).hexdigest(),
                 "tokens": len(toks), "results": used, "cut": cut, "text": text})

tmp = contexts.with_name(contexts.name + ".part")
tmp.write_text("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in rows))
tmp.rename(contexts)
