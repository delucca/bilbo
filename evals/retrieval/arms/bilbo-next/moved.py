"""Compare the chunk lists of two searches of the same questions, with no judge. Usage: moved.py <search.jsonl of the store
with notes, searched with notes_budget 0> <search.jsonl of the store without notes>. It prints the questions compared, the
share whose list differs, the share whose first result differs, and how many lists lost every hit: each is 0 when the notes
never move the chunks' list."""

import json
import sys
from pathlib import Path


def lists(path):
    found = {}
    for line in Path(path).read_text().split("\n"):
        if line.strip():
            row = json.loads(line)
            found.setdefault((row["id"], row["form"]), [(r["session"], tuple(r["lines"])) for r in row["results"]])
    return found


def compare(with_notes, without):
    both = sorted(set(with_notes) & set(without))
    differ = sum(with_notes[k] != without[k] for k in both)
    first = sum(with_notes[k][:1] != without[k][:1] for k in both)
    lost = sum(not with_notes[k] and bool(without[k]) for k in both)
    return len(both), differ, first, lost


def main():
    if len(sys.argv) != 3:
        sys.exit("usage: moved.py <search.jsonl with notes> <search.jsonl without>")
    n, differ, first, lost = compare(lists(sys.argv[1]), lists(sys.argv[2]))
    share = lambda x: f"{x} ({100 * x / n:.1f}%)" if n else f"{x}"
    print(f"questions compared: {n}")
    print(f"chunk list differs: {share(differ)}")
    print(f"first result differs: {share(first)}")
    print(f"lists that lost every hit: {lost}")


if __name__ == "__main__":
    main()
