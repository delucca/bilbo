"""Compare a run of the bilbo-next arm with the bm25 arm's own rows and contexts, without a judge. Report mode needs only
the standard library; --gate must run in the scripts' environment (see below).

  compare.py [options] <reference search.jsonl> <reference contexts> <run search.jsonl> <run contexts> <chunks.jsonl> <questions.jsonl>
  compare.py --gate [options] <reference dir> <run dir> <reference scores>

Report: for all forms and by form, the contexts byte-equal to the reference's (sha), the share of the reference
context's results that the run's context holds, the overlap of the first 8 results, the lists equal, and the fact-quote
proxy (the share of a question's facts with an evidence quote in the context). A bilbo result is the eval chunk of
<chunks.jsonl> with its repo, session, lines and part; its text only verifies that (the first line is the session's
`dated()` line, the rest the chunk's text or, for a cut result, a prefix of it). A run of the reference.py global rows
(refs are chunk ids) needs no mapping. --only-question-repo drops the results of other repositories from the run's
lists, keeps the first 50 and compares lists only (the contexts were packed with them).

--gate takes run folders holding search.jsonl and contexts-4000.jsonl and must run in the scripts' environment (it imports
`pack` and `answerable` from this folder and needs tiktoken's encoding): `uv run --project scripts --locked python -I
compare.py --gate ...` with TIKTOKEN_CACHE_DIR set. The wanted pairs are the answerable questions of --questions times
every form; each of the five inputs holds each wanted pair once and no other. A pair is checked in order and the first
failure is printed with its label: reference (every reference result is a chunk whose text is `expected`, with the
bits of its score in the scores, the ids distinct, the length the smaller of 50 and the positive scores, the score bits
the highest in order), cut (no run result is cut), unmapped (every run result is a chunk), text (every run text is
`expected`, no prefix), context (each list packed with pack.py's own function at budget 4000 gives the context it came
with). `expected` is the session's first line from the corpus, a line feed and the chunk's text. Then the two contexts
are equal, or the pair is a tie (the run's ids are distinct, the lists have one length, every run id has a reference
score, at every rank the run's score has the bits of the reference's, and cutting the ranks into runs of equal bits the
run's ids are the reference's set in every run but the last, in which each holds that score), else unexplained. The
gate exits 1 when any pair fails.

Options: --corpus <dir> (the unpacked corpus, for the first line of a text; required by --gate), --chunks <file> and
--questions <file> (the gate's inputs)."""

import json
import struct
import sys
from datetime import datetime, timezone
from pathlib import Path
from statistics import mean

sys.dont_write_bytecode = True

FORMS = ("query", "prompt")


def read_rows(path):
    """The rows by (id, form), and the pairs a file repeats."""
    rows, repeated = {}, []
    for line in Path(path).read_text(encoding="utf-8").split("\n"):
        if line.strip():
            r = json.loads(line)
            if (r["id"], r["form"]) in rows:
                repeated.append((r["id"], r["form"]))
            rows[r["id"], r["form"]] = r
    return rows, repeated


def read_strict(path):
    rows, repeated = read_rows(path)
    if repeated:
        sys.exit(f"compare: {path} repeats {repeated[0][0]} {repeated[0][1]}")
    return rows


def bits(score):
    return f"{struct.unpack('>I', struct.pack('>f', score))[0]:08x}"


class Chunks:
    def __init__(self, path):
        self.by_id, self.coords = {}, {}
        for line in Path(path).open(encoding="utf-8"):
            r = json.loads(line)
            self.by_id[r["id"]] = r
            self.coords[self.key(r["repo"], r["session"], r["lines"], r["part"])] = r["id"]

    @staticmethod
    def key(repo, session, lines, part):
        return (repo, session, lines[0], lines[1], tuple(part) if part else None)

    def of(self, result):
        """The eval chunk id of a result, None for a bilbo result no chunk matches."""
        if "entry" not in result:
            return result["ref"] if result["ref"] in self.by_id else None
        return self.coords.get(self.key(result["repo"], result["session"], result["lines"], result["part"]))


class Corpus:
    def __init__(self, root):
        self.root, self.starts = root, {}

    def first_line(self, repo, session):
        if (repo, session) not in self.starts:
            when = None
            for line in (Path(self.root) / repo / session).open(encoding="utf-8"):
                stamp = json.loads(line).get("timestamp") if line.strip() else None
                if stamp:
                    when = datetime.fromisoformat(stamp).astimezone(timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
                    break
            self.starts[repo, session] = f"Session started {when}:" if when else "Session date unknown:"
        return self.starts[repo, session]


def verify(chunks, corpus, result, chunk_id):
    """Why a bilbo result's text is not its chunk's, or None."""
    chunk = chunks.by_id[chunk_id]
    first, _, rest = result["text"].partition("\n")
    if corpus and first != corpus.first_line(chunk["repo"], chunk["session"]):
        return f"first line {first!r}"
    if rest == chunk["text"] or (result.get("cut") and chunk["text"].startswith(rest)):
        return None
    return "text differs from the chunk's" + (" (cut)" if result.get("cut") else "")


def overlap(a, b, k):
    kk = min(k, len(a))
    return len(set(a[:kk]) & set(b[:kk])) / kk if kk else 1.0


def norm(text):
    return " ".join(text.split())


def covered(contexts, questions, keys, form):
    shares = []
    for k in keys:
        if k[1] != form:
            continue
        text, facts = norm(contexts[k]["text"]), questions[k[0]]["facts"]
        found = 0
        for f in facts:
            quotes = [norm(e["quote"]).rstrip(". ").removesuffix("...").strip() for e in f["evidence"]]
            found += any(q and q in text for q in quotes)
        shares.append(found / len(facts))
    return mean(shares) if shares else None


def pct(x):
    return "-" if x is None else f"{100 * x:.1f}%"


def parse(argv):
    flags, positional, i = {}, [], 0
    while i < len(argv):
        a = argv[i]
        if a in ("--corpus", "--chunks", "--questions"):
            if i + 1 >= len(argv):
                sys.exit(__doc__)
            flags[a[2:]] = argv[i + 1]
            i += 2
        elif a in ("--gate", "--only-question-repo"):
            flags[a[2:]] = True
            i += 1
        elif a.startswith("--"):
            sys.exit(__doc__)
        else:
            positional.append(a)
            i += 1
    return flags, positional


def lists_of(rows, chunks, questions, only_repo):
    ids, unmapped = {}, 0
    for k, row in rows.items():
        found = [chunks.of(r) for r in row["results"]]
        unmapped += sum(c is None for c in found)
        if only_repo:
            found = [c for c in found if c is not None and chunks.by_id[c]["repo"] == questions[k[0]]["repo"]][:50]
        ids[k] = found
    return ids, unmapped


def report(flags, positional):
    ref_search, ref_ctx, run_search, run_ctx, chunks_path, questions_path = positional
    only = flags.get("only-question-repo")
    chunks = Chunks(chunks_path)
    questions = {q["id"]: q for q in map(json.loads, filter(str.strip, Path(questions_path).read_text().split("\n")))}
    corpus = Corpus(flags["corpus"]) if "corpus" in flags else None
    ref_rows, run_rows = read_strict(ref_search), read_strict(run_search)
    ref_c, run_c = read_strict(ref_ctx), (None if only else read_strict(run_ctx))
    keys = sorted(ref_rows)
    if sorted(run_rows) != keys:
        sys.exit(f"compare: the run has {len(run_rows)} forms, the reference {len(keys)}")
    for form in FORMS:
        if not any(k[1] == form for k in keys):
            sys.exit(f"compare: no question has the form {form}")
    ref_ids, _ = lists_of(ref_rows, chunks, questions, False)
    run_ids, unmapped = lists_of(run_rows, chunks, questions, only)
    differ = 0
    for k, row in run_rows.items():
        for r, c in zip(row["results"], lists_of({k: row}, chunks, questions, False)[0][k]):
            if "entry" in r and c is not None and verify(chunks, corpus, r, c):
                differ += 1
    print(f"results with no eval chunk: {unmapped}; verified texts that differ: {differ}")
    if only:
        print(f"forms with fewer than 50 results left: {sum(len(v) < 50 for v in run_ids.values())} of {len(keys)}")
    print(f"{'forms':<8}{'sha equal':>12}{'present':>10}{'overlap@8':>11}{'lists equal':>13}{'proxy ref':>11}{'proxy run':>11}")
    for name, forms in (("all", FORMS), *((f, (f,)) for f in FORMS)):
        ks = [k for k in keys if k[1] in forms]
        sha = None if only else sum(ref_c[k]["sha"] == run_c[k]["sha"] for k in ks)
        present = None if only else mean(
            len(set(ref_ids[k][: ref_c[k]["results"]]) & set(run_ids[k][: run_c[k]["results"]]))
            / max(1, ref_c[k]["results"]) for k in ks)
        ov8 = mean(overlap(ref_ids[k], run_ids[k], 8) for k in ks)
        same = sum(ref_ids[k] == run_ids[k] for k in ks)
        p_ref = mean(covered(ref_c, questions, ks, f) for f in forms)
        p_run = None if only else mean(covered(run_c, questions, ks, f) for f in forms)
        sha_text = "-" if sha is None else f"{sha}/{len(ks)}"
        print(f"{name:<8}{sha_text:>12}{pct(present):>10}{pct(ov8):>11}{f'{same}/{len(ks)}':>13}"
              f"{pct(p_ref):>11}{pct(p_run):>11}")
    return 0


BUDGET = 4000


def failure(label, k, text):
    print(f"{label}: {k[0]} {k[1]}: {text}")
    return label


def check_pair(k, ref_row, run_row, ref_ctx, run_ctx, held, chunks, expected, pack):
    """The first failure of a pair as its label, 'equal', or 'ties' with the lines to print."""
    ref = ref_row["results"]
    ids = [r["ref"] for r in ref]
    for rank, r in enumerate(ref, 1):
        c = r["ref"]
        if c not in chunks.by_id:
            return failure("reference", k, f"rank {rank}: {c} is not a chunk"), []
        if r["text"] != expected(c):
            return failure("reference", k, f"rank {rank}: chunk {c}: the text is not the chunk's"), []
        if c not in held or bits(held[c]) != bits(r["score"]):
            return failure("reference", k, f"rank {rank}: chunk {c}: the score is not the scores' ({r['score']!r})"), []
    if len(set(ids)) != len(ids):
        return failure("reference", k, "a chunk is listed twice"), []
    best = sorted(held.values(), reverse=True)
    if len(ref) != min(50, len(best)):
        return failure("reference", k, f"{len(ref)} results where {min(50, len(best))} are due"), []
    for rank, (r, s) in enumerate(zip(ref, best), 1):
        if bits(r["score"]) != bits(s):
            return failure("reference", k, f"rank {rank}: chunk {r['ref']} has bits {bits(r['score'])}, the {rank}th best score {bits(s)}"), []
    run = run_row["results"]
    for rank, r in enumerate(run, 1):
        if r.get("cut"):
            return failure("cut", k, f"rank {rank}: {r.get('ref')} is cut"), []
    run_ids = []
    for rank, r in enumerate(run, 1):
        c = chunks.of(r)
        if c is None:
            return failure("unmapped", k, f"rank {rank}: {r.get('ref')} is no chunk"), []
        run_ids.append(c)
    for rank, (r, c) in enumerate(zip(run, run_ids), 1):
        if r["text"] != expected(c):
            return failure("text", k, f"rank {rank}: chunk {c}: the text is not the chunk's"), []
    for label, rows, ctx in (("reference", ref, ref_ctx), ("run", run, run_ctx)):
        packed = pack([r["text"] for r in rows], BUDGET)
        if ctx["text"] != packed["text"] or ctx["sha"] != packed["sha"]:
            return failure("context", k, f"the {label}'s context is not what pack.py builds from its results"), []
    if ref_ctx["sha"] == run_ctx["sha"] and ref_ctx["text"] == run_ctx["text"]:
        return "equal", []
    sa = [bits(r["score"]) for r in ref]
    why = ties(ids, run_ids, sa, held)
    if isinstance(why, str):
        first = next(i for i in range(max(len(ids), len(run_ids))) if ids[i:i + 1] != run_ids[i:i + 1])
        ra = ref[first]["score"] if first < len(ids) else None
        c = run_ids[first] if first < len(run_ids) else None
        rb = held.get(c) if c is not None else None
        where = (f"rank {first + 1}: reference {ids[first] if first < len(ids) else None} score {ra!r} bits "
                 f"{bits(ra) if ra is not None else None}, run {c} score {rb!r} bits "
                 f"{bits(rb) if rb is not None else None}")
        return failure("unexplained", k, f"{where} ({why})"), []
    return "ties", [f"  ranks {s + 1}-{e}: score {ref[s]['score']!r} bits {sa[s]}, {e - s} results, reference "
                    f"{' '.join(map(str, ids[s:e]))}, run {' '.join(map(str, run_ids[s:e]))}" + extra
                    for s, e, extra in why]


def ties(a, b, sa, held):
    """The runs of equal reference scores in which the run's order differs, as (first, end, note), or why the lists are
    not a tie."""
    if len(set(b)) != len(b):
        return "the run's results are not distinct chunks"
    if len(a) != len(b):
        return f"{len(a)} and {len(b)} results"
    if any(c not in held for c in b):
        return "a run result has no reference score"
    if any(bits(held[c]) != sa[i] for i, c in enumerate(b)):
        return "a run result's score is not the reference's at its rank"
    runs, start = [], 0
    for i in range(1, len(a) + 1):
        if i == len(a) or sa[i] != sa[start]:
            runs.append((start, i))
            start = i
    shown = []
    for n, (s, e) in enumerate(runs):
        last = n == len(runs) - 1
        holders = {c for c, v in held.items() if bits(v) == sa[s]}
        if last:
            if not set(b[s:e]) <= holders:
                return f"ranks {s + 1}-{e} hold a chunk with another score"
        elif set(b[s:e]) != set(a[s:e]):
            return f"ranks {s + 1}-{e} hold other chunks"
        if b[s:e] != a[s:e]:
            split = last and set(b[s:e]) != set(a[s:e])
            shown.append((s, e, f" (at the cut: {len(holders)} documents hold this score)" if split else ""))
    return shown


def gate(flags, positional):
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from pack import pack
    from questions import answerable

    ref_dir, run_dir, scores_path = positional
    for need in ("chunks", "questions", "corpus"):
        if need not in flags:
            sys.exit(f"compare: --gate needs --{need}")
    chunks = Chunks(flags["chunks"])
    corpus = Corpus(flags["corpus"])
    wanted = {(q["id"], f) for q in answerable(flags["questions"]) for f in FORMS}
    files = {"reference rows": Path(ref_dir) / "search.jsonl", "run rows": Path(run_dir) / "search.jsonl",
             "reference contexts": Path(ref_dir) / f"contexts-{BUDGET}.jsonl",
             "run contexts": Path(run_dir) / f"contexts-{BUDGET}.jsonl", "reference scores": Path(scores_path)}
    data, input_errors = {}, []
    for label, path in files.items():
        rows, repeated = read_rows(path)
        data[label] = rows
        input_errors += [f"{label} repeat {k[0]} {k[1]}" for k in repeated]
        input_errors += [f"{label} lack {k[0]} {k[1]}" for k in sorted(wanted - rows.keys())]
        input_errors += [f"{label} hold {k[0]} {k[1]}, which --questions does not" for k in sorted(rows.keys() - wanted)]
    for line in input_errors:
        print(f"input: {line}")
    if not wanted:
        print("input: --questions holds no answerable question")
    if input_errors or not wanted:
        print("GATE FAILED")
        return 1

    def expected(c):
        chunk = chunks.by_id[c]
        return corpus.first_line(chunk["repo"], chunk["session"]) + "\n" + chunk["text"]

    scores = {k: {c: s for c, s in r["scores"]} for k, r in data["reference scores"].items()}
    counts = {"equal": 0, "ties": 0, "failed": 0}
    for k in sorted(wanted):
        outcome, lines = check_pair(k, data["reference rows"][k], data["run rows"][k], data["reference contexts"][k],
                                    data["run contexts"][k], scores[k], chunks, expected, pack)
        if outcome in counts:
            counts[outcome] += 1
        else:
            counts["failed"] += 1
        if outcome == "ties":
            print(f"ties: {k[0]} {k[1]}: {len(lines)} runs differ")
            for line in lines:
                print(line)
    print(f"equal {counts['equal']}, ties {counts['ties']}, failed {counts['failed']} of {len(wanted)}")
    print("GATE FAILED" if counts["failed"] else "GATE OK")
    return 1 if counts["failed"] else 0


if __name__ == "__main__":
    opts, args = parse(sys.argv[1:])
    if opts.get("gate") and len(args) == 3:
        sys.exit(gate(opts, args))
    if not opts.get("gate") and len(args) == 6:
        sys.exit(report(opts, args))
    sys.exit(__doc__)
