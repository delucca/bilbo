"""The contract fixtures of the chunker and the analyzer, cut from what the retrieval eval's bm25 arm built.
The eval lives outside this tree: every run names what it reads. Nothing here runs in a test.

    regenerate.py --store <folder>                    write expected.jsonl from sample.jsonl and edges.jsonl
    regenerate.py --store <folder> --check            fail when expected.jsonl or stems.tsv.zst is not what this writes now
    regenerate.py --store <folder> --stems            write stems.tsv.zst: every distinct word of the store's chunks
                                                      with PyStemmer's stem, as word<TAB>n<TAB>suffix (the stem is the
                                                      word without its last n characters, then suffix), zstd level 19
    regenerate.py --store <folder> --build <build.py> --corpus <folder>
                                                      pick sample.jsonl again, then write expected.jsonl
    regenerate.py --store <folder> --build <build.py> --whole <corpus> <out> [--questions <questions.jsonl>]
                                                      write messages.jsonl and digests.tsv for the ignored
                                                      whole-store tests of this crate and of the qwen crate, and with
                                                      --questions ranks-whole.jsonl: the 200 best chunks of each of the
                                                      eval's answerable questions' two forms in the question's repo
    regenerate.py --store <folder> --ranks --questions <questions.jsonl> [--check]
                                                      write ranks.jsonl from sample.jsonl's chunks, the synthetic
                                                      documents of rank-docs.jsonl and the queries: the first 40 question
                                                      texts with a term in the sample's vocabulary, then rank-edges.jsonl
    regenerate.py --table                             write src/analyze/word.rs
    --out <folder>                                    with --corpus: write sample.jsonl and expected.jsonl there

<folder> of --store is the unpacked eval store (store-bm25-cc0c68a903d7: chunks.jsonl); of --corpus the
unpacked eval corpus (<folder>/<repo>/*.jsonl). build.py is the bm25 arm's script; only its render_session
runs, to turn corpus lines into the messages the chunker reads.

Run it with the eval's own versions, and Python 3.12 for its Unicode 15.0:
    uv run --python 3.12 --with bm25s==0.3.11 --with PyStemmer==3.1.0 --with tokenizers==0.22.1 \\
        --with zstandard==0.25.0 python3 -I regenerate.py ...

sample.jsonl holds spans (an expected chunk names its span by its row number, from 0): consecutive messages of one session that start where an eval chunk starts and end
where one ends, so the eval's chunks for them are in its store as they are. edges.jsonl is written by hand:
texts for the analyzer that the corpus does not hold. rank-docs.jsonl and rank-edges.jsonl are written by hand too: synthetic
documents (lists of terms, in corpora named by `corpus`) and the queries that rank them or the sample. expected.jsonl,
ranks.jsonl and ranks-whole.jsonl are never written by hand."""

import collections
import importlib.util
import json
import pathlib
import re
import sys
import types
import unicodedata

HERE = pathlib.Path(__file__).resolve().parent
TOKENIZER = HERE.parent.parent / "qwen" / "tokenizer.json.zst"
TABLE = HERE.parent.parent / "src" / "analyze" / "word.rs"
STEMS = HERE / "stems.tsv.zst"
UNICODE = "15.0.0"
BUDGET, OVERLAP = 512, 64
EACH = 3  # spans picked per shape
LIMIT = 24000  # bytes of message text a span may hold
PATTERN = r"(?u)\b\w\w+\b"
MAX_TERM_BYTES = 4096


def fits(term):
    return len(term.encode()) <= MAX_TERM_BYTES


def fail(message):
    sys.exit(f"regenerate: {message}")


def dump(rows):
    return "".join(json.dumps(row, ensure_ascii=False) + "\n" for row in rows).encode()


def rows_of(path):
    """A text may hold U+2028 and its like: only a line feed ends a row."""
    return [json.loads(line) for line in path.read_text(encoding="utf-8").split("\n") if line]


def load_build(path):
    sys.dont_write_bytecode = True  # no __pycache__ beside build.py
    for name in ("bm25s", "Stemmer"):
        sys.modules.setdefault(name, types.ModuleType(name))
    spec = importlib.util.spec_from_file_location("bm25_build", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    for name in ("bm25s", "Stemmer"):
        if not hasattr(sys.modules[name], "__file__"):
            del sys.modules[name]
    return module


def store_chunks(store):
    by_file = collections.defaultdict(list)
    with open(pathlib.Path(store) / "chunks.jsonl", encoding="utf-8") as lines:
        for line in lines:
            row = json.loads(line)
            by_file[f"{row['repo']}/{row['session']}"].append(row)
    return by_file


class Analyzer:
    """bm25s's terms, as the eval's manifest set it up, without the words of more than MAX_TERM_BYTES
    bytes (UTF-8) and without the stems of more than that: an index holds no longer term."""

    def __init__(self):
        import bm25s
        import Stemmer

        self.bm25s, self.stemmer = bm25s, Stemmer.Stemmer("english")

    def tokenize(self, texts, stemmer):
        return self.bm25s.tokenize(texts, lower=True, token_pattern=PATTERN, stopwords="en",
                                   stemmer=stemmer, return_ids=False, show_progress=False)

    def terms(self, texts, stemmed=True):
        words = self.tokenize(texts, None)
        found = self.tokenize(texts, self.stemmer) if stemmed else words
        return [[term for word, term in zip(each, kept) if fits(word) and fits(term)]
                for each, kept in zip(words, found)]


class Qwen:
    """tokenizer.json as llama.cpp reads the GGUF: no normalizer, and each token's bytes."""

    def __init__(self):
        import zstandard
        from tokenizers import Tokenizer, normalizers

        file = zstandard.ZstdDecompressor().decompress(TOKENIZER.read_bytes()).decode()
        self.tokenizer = Tokenizer.from_str(file)
        self.tokenizer.normalizer = normalizers.Sequence([])
        self.added = {a["id"]: len(a["content"].encode()) for a in json.loads(file)["added_tokens"]}

    def ends(self, text):
        encoding, at, ends = self.tokenizer.encode(text, add_special_tokens=False), 0, []
        for token_id, token in zip(encoding.ids, encoding.tokens):
            at += self.added.get(token_id, len(token))
            ends.append(at)
        if at != len(text.encode()):
            fail("the tokens do not join back to the text")
        return ends

    def count(self, text):
        return len(self.tokenizer.encode(text, add_special_tokens=False).ids)


def fnv(data):
    digest = 0xCBF29CE484222325
    for byte in data:
        digest = ((digest ^ byte) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return f"{digest:016x}"


def shapes(qwen, message):
    """What chunking one message over the budget exercises, by build.py's windows() run again here."""
    text, heads = message["text"], message["heads"]
    data, ends = text.encode(), qwen.ends(text)
    offsets = [0] + ends
    width = BUDGET - qwen.count(heads[0][1] + "\n")
    found, start = set(), 0
    while True:
        end = min(start + width, len(ends))
        first = True
        while True:
            lo, hi = offsets[start], offsets[end]
            if (lo > 0 and data[lo] & 0xC0 == 0x80) or (hi < len(data) and data[hi] & 0xC0 == 0x80):
                found.add("a window edge inside a character")
            while lo > 0 and data[lo] & 0xC0 == 0x80:
                lo -= 1
            while hi < len(data) and data[hi] & 0xC0 == 0x80:
                hi += 1
            body = data[lo:hi].decode()
            if lo:
                label = next(h for at, h in reversed(heads) if at <= lo)
                if label != heads[0][1]:
                    found.add("a window that starts in a later piece")
                body = label + "\n" + body
            if qwen.count(body) <= BUDGET:
                break
            found.add("a window that loses tokens to fit" if first else "a window that loses tokens twice")
            first = False
            end -= 1
        if end == len(ends):
            return found or {"plain windows"}
        start = end - OVERLAP


def pick(build, store, corpus):
    """Spans by shape, the smallest first: a packed chunk, the windows of one message, and runs of three
    chunk groups that cross from packed to windows and back."""
    qwen, by_file = Qwen(), store_chunks(store)
    candidates = collections.defaultdict(list)
    for path in sorted(pathlib.Path(corpus).glob("*/*.jsonl")):
        name = f"{path.parent.name}/{path.name}"
        messages, _ = build.render_session(path)
        entries = [json.loads(line) for line in path.open(encoding="utf-8") if line.strip()]
        cursor = any(entry.get("type") is None and entry.get("role") for entry in entries)
        by_line = {m["line"]: m for m in messages}
        groups = []  # consecutive chunks that share their lines: one packed chunk, or one message's windows
        for chunk in by_file.get(name, []):
            if groups and chunk["part"] and chunk["part"][0] > 0:
                groups[-1].append(chunk)
            else:
                groups.append([chunk])
        spans = []
        for group in groups:
            first, last = group[0]["lines"][0], group[-1]["lines"][1]
            inside = [m for m in messages if first <= m["line"] <= last]
            size = sum(len(m["text"].encode()) for m in inside)
            if group[0]["part"]:
                found = shapes(qwen, by_line[first])
            elif first != last:
                found = {"several messages packed"}
            else:
                found = {"one message alone"}
            if any(h[1] == "compact summary:" for m in inside for h in m["heads"]):
                found.add("a compact summary")
            if cursor:
                found.add("a Cursor session")
            spans.append((found, size, inside, group))
            for shape in found:
                candidates[shape].append((size, name, first, inside, group))
        for at in range(len(spans) - 2):
            three = spans[at:at + 3]
            kinds = [bool(span[3][0]["part"]) for span in three]
            if kinds == [False, True, False]:
                inside = [m for span in three for m in span[2]]
                group = [c for span in three for c in span[3]]
                candidates["packed, windows, packed"].append((sum(s[1] for s in three), name, inside[0]["line"], inside, group))
    sample, taken = [], set()
    for shape in sorted(candidates):
        picked = 0
        for size, name, first, inside, group in sorted(candidates[shape], key=lambda c: c[:3]):
            if picked == EACH or size > LIMIT:
                break
            picked += 1
            if (name, first, len(group)) in taken:
                continue
            taken.add((name, first, len(group)))
            sample.append({"file": name, "shape": shape, "messages": [
                {"line": m["line"], "text": m["text"], "heads": [list(h) for h in m["heads"]]} for m in inside]})
        if picked == 0:
            print(f"regenerate: no span under {LIMIT} bytes for: {shape}", file=sys.stderr)
    return sample


K1, B = 1.2, 0.75
TOP = 10  # a ranking row keeps this many of the best, and every document that ties with the last
WHOLE_TOP = 200


def scores_of(analyzer, documents, queries):
    """bm25s's own scores (method lucene, k1 1.2, b 0.75) of each query's terms over `documents`, term lists:
    one float32 array a query, None for a query with no term in the vocabulary."""
    import numpy as np

    retriever = analyzer.bm25s.BM25(method="lucene", k1=K1, b=B)
    retriever.index(documents, show_progress=False)
    found = []
    for terms in queries:
        known = [term for term in terms if term in retriever.vocab_dict]
        found.append(retriever.get_scores(known).astype(np.float32) if known else None)
    return found


def best(scores, limit):
    """[document, score, bits] by score, then document, over the `limit` best with a score above 0 and every
    document that ties with the last of them; limit None keeps every document with a score above 0."""
    import numpy as np

    if scores is None:
        return []
    order = np.lexsort((np.arange(len(scores)), -scores))
    order = [int(at) for at in order if scores[at] > 0]
    if limit is not None and len(order) > limit:
        cut = scores[order[limit - 1]]
        order = [at for at in order if scores[at] >= cut]
    return [[at, float(scores[at]), int(scores[at].view(np.uint32))] for at in order]


def question_texts(questions):
    """The answerable questions' original wordings, `query` then `prompt`, in file order."""
    found = []
    for row in rows_of(pathlib.Path(questions)):
        if not row["abstain"]:
            original = row["wording"]["original"]
            found += [(row["id"], form, row["repo"], original[form]) for form in ("query", "prompt")]
    return found


def ranks(questions):
    """The rows of ranks.jsonl."""
    analyzer = Analyzer()
    sample = [row["terms"] for row in rows_of(HERE / "expected.jsonl") if "span" in row]
    corpora = {"sample": sample}
    for row in rows_of(HERE / "rank-docs.jsonl"):
        corpora.setdefault(row["corpus"], []).append(row["terms"])
    vocabulary = {term for terms in sample for term in terms}
    wanted = []
    for _, _, _, text in question_texts(questions):
        if len(wanted) < 40 and any(term in vocabulary for term in analyzer.terms([text])[0]):
            wanted.append(("sample", text))
    wanted += [(row["corpus"], row["query"]) for row in rows_of(HERE / "rank-edges.jsonl")]
    out = []
    for corpus, documents in corpora.items():
        chosen = [text for name, text in wanted if name == corpus]
        queries = analyzer.terms(chosen)
        for text, terms, scores in zip(chosen, queries, scores_of(analyzer, documents, queries)):
            out.append((corpus, text, terms, best(scores, TOP if corpus == "sample" else None)))
    order = {(name, text): at for at, (name, text) in enumerate(wanted)}
    out.sort(key=lambda row: order[row[0], row[1]])
    return [{"corpus": corpus, "query": text, "terms": terms, "top": top} for corpus, text, terms, top in out]


def ranks_whole(store, questions, out):
    """ranks-whole.jsonl: for each question form, the 200 best chunks of its repo by bm25s, as the eval's arm ranks."""
    analyzer, by_repo = Analyzer(), {}
    for name, chunks in store_chunks(store).items():
        by_repo.setdefault(name.split("/")[0], []).extend(chunks)
    asked = question_texts(questions)
    lines = []
    for repo in sorted(by_repo):
        chunks = by_repo[repo]
        documents = []
        for at in range(0, len(chunks), 2000):
            documents += analyzer.terms([chunk["text"] for chunk in chunks[at:at + 2000]])
        mine = [item for item in asked if item[2] == repo]
        queries = analyzer.terms([item[3] for item in mine])
        for (qid, form, _, text), scores in zip(mine, scores_of(analyzer, documents, queries)):
            top = [[chunks[at]["id"], score, bits] for at, score, bits in best(scores, WHOLE_TOP)]
            lines.append((qid, form, {"id": qid, "form": form, "repo": repo, "query": text, "top": top}))
    order = {(qid, form): at for at, (qid, form, _, _) in enumerate(asked)}
    lines.sort(key=lambda line: order[line[0], line[1]])
    (pathlib.Path(out) / "ranks-whole.jsonl").write_bytes(dump([row for _, _, row in lines]))
    print(f"regenerate: {len(lines)} lists")


def lowercase_row():
    """Every character Python lowercases, between two letters."""
    return " ".join(f"x{chr(c)}x" for c in range(sys.maxunicode + 1)
                    if not 0xD800 <= c <= 0xDFFF and chr(c).lower() != chr(c))


def edges_of_words():
    """The first and last character of every run of word characters, and the ones just outside, each between
    two letters: the table of src/analyze/word.rs against Python's own `\\w`."""
    word = re.compile(r"\w")
    found, was = [], False
    for c in range(sys.maxunicode + 2):
        now = c <= sys.maxunicode and not 0xD800 <= c <= 0xDFFF and bool(word.match(chr(c)))
        if now != was:
            found += [c - 1, c]
        was = now
    return " ".join(f"x{chr(c)}x" for c in found if 0 <= c <= sys.maxunicode and not 0xD800 <= c <= 0xDFFF)


def expected(store, sample, edges):
    analyzer, by_file = Analyzer(), store_chunks(store)
    rows = []
    for at, span in enumerate(sample):
        first, last = span["messages"][0]["line"], span["messages"][-1]["line"]
        chunks = [c for c in by_file[span["file"]] if first <= c["lines"][0] and c["lines"][1] <= last]
        if not chunks or chunks[0]["lines"][0] != first or chunks[-1]["lines"][1] != last:
            fail(f"{span['file']} lines {first}-{last} do not start and end with a chunk of the store")
        if chunks[0]["part"] and chunks[0]["part"][0] != 0:
            fail(f"{span['file']} line {first} starts inside a message's windows")
        if chunks[-1]["part"] and chunks[-1]["part"][0] != chunks[-1]["part"][1] - 1:
            fail(f"{span['file']} line {last} ends inside a message's windows")
        for chunk, terms in zip(chunks, analyzer.terms([c["text"] for c in chunks])):
            rows.append({"span": at, "file": span["file"], "lines": chunk["lines"], "part": chunk["part"],
                         "tokens": chunk["tokens"], "text": chunk["text"], "terms": terms})
    texts = [edge["text"] for edge in edges]
    for at, terms in enumerate(analyzer.terms(texts)):
        rows.append({"edge": at, "terms": terms})
    for name, text in (("lowercase", lowercase_row()), ("words", edges_of_words())):
        rows.append({"generated": name, "text": text, "terms": analyzer.terms([text])[0]})
    return rows


def whole(build, store, corpus, out, questions=None):
    if questions:
        ranks_whole(store, questions, out)
    analyzer, qwen, by_file = Analyzer(), Qwen(), store_chunks(store)
    out = pathlib.Path(out)
    with open(out / "messages.jsonl", "wb") as messages_file:
        for path in sorted(pathlib.Path(corpus).glob("*/*.jsonl")):
            messages, _ = build.render_session(path)
            messages_file.write(dump([{"file": f"{path.parent.name}/{path.name}", "messages": [
                {"line": m["line"], "text": m["text"], "heads": [list(h) for h in m["heads"]]} for m in messages]}]))
    vocab = {}
    with open(out / "digests.tsv", "w", encoding="utf-8") as digests:
        rows = [chunk for name in sorted(by_file) for chunk in by_file[name]]
        for at in range(0, len(rows), 2000):
            part = rows[at:at + 2000]
            texts = [row["text"] for row in part]
            for row, words, terms in zip(part, analyzer.terms(texts, stemmed=False), analyzer.terms(texts)):
                vocab.update(zip(words, terms))
                ends = qwen.ends(row["text"])
                digests.write(f"{row['id']}\t{len(terms)}\t{fnv(chr(10).join(terms).encode())}\t{len(ends)}\t"
                              f"{fnv(','.join(map(str, ends)).encode())}\n")
    (out / "vocab.tsv").write_text("".join(f"{word}\t{vocab[word]}\n" for word in sorted(vocab)), encoding="utf-8")
    print(f"regenerate: {len(rows)} chunks, {len(vocab)} words")


def stems(store):
    """The stems file's bytes: one row a distinct word of the store's chunks, sorted by word."""
    import zstandard

    analyzer, vocab = Analyzer(), {}
    rows = [chunk["text"] for name, chunks in sorted(store_chunks(store).items()) for chunk in chunks]
    for at in range(0, len(rows), 2000):
        texts = rows[at:at + 2000]
        for words, terms in zip(analyzer.terms(texts, stemmed=False), analyzer.terms(texts)):
            vocab.update(zip(words, terms))
    lines = []
    for word in sorted(vocab):
        stem = vocab[word]
        common = 0
        while common < min(len(word), len(stem)) and word[common] == stem[common]:
            common += 1
        lines.append(f"{word}\t{len(word) - common}\t{stem[common:]}\n")
    return zstandard.ZstdCompressor(level=19).compress("".join(lines).encode())


def table():
    word = re.compile(r"\w")
    ranges, start = [], None
    for c in range(sys.maxunicode + 2):
        now = c <= sys.maxunicode and bool(word.match(chr(c)))
        if now and start is None:
            start = c
        if not now and start is not None:
            ranges.append((start, c - 1))
            start = None
    rows = "".join(f"    (0x{a:04X}, 0x{b:04X}),\n" for a, b in ranges)
    TABLE.write_text(
        f"//! Python's `\\w` over Unicode {UNICODE}: written by `tests/contract/regenerate.py --table`, never by hand.\n\n"
        f"/// Inclusive, rising.\npub const WORD: [(u32, u32); {len(ranges)}] = [\n{rows}];\n", encoding="utf-8")


def main():
    args = sys.argv[1:]
    if unicodedata.unidata_version != UNICODE:
        fail(f"this Python has Unicode {unicodedata.unidata_version}; the eval ran {UNICODE} (Python 3.12)")
    if args == ["--table"]:
        return table()

    def value(flag, count=1):
        if flag not in args:
            return None
        at = args.index(flag)
        taken = args[at + 1:at + 1 + count]
        if len(taken) != count:
            fail(f"{flag} needs {count} value(s)")
        del args[at:at + 1 + count]
        return taken[0] if count == 1 else taken

    store, build_path = value("--store"), value("--build")
    corpus, out, everything = value("--corpus"), value("--out"), value("--whole", 2)
    questions = value("--questions")
    only_ranks = "--ranks" in args
    if only_ranks:
        args.remove("--ranks")
    check = "--check" in args
    if check:
        args.remove("--check")
    only_stems = "--stems" in args
    if only_stems:
        args.remove("--stems")
    if args or not store:
        fail("usage: see the top of this file")
    if (corpus or everything) and not build_path:
        fail("--corpus and --whole need --build <build.py>")
    if only_ranks:
        if not questions:
            fail("--ranks needs --questions <questions.jsonl>")
        data = dump(ranks(questions))
        if check:
            if (HERE / "ranks.jsonl").read_bytes() != data:
                fail("ranks.jsonl is not what bm25s gives now")
            return print("regenerate: ranks.jsonl is current")
        (HERE / "ranks.jsonl").write_bytes(data)
        return print(f"regenerate: {len(data)} bytes of ranks.jsonl")
    if everything:
        return whole(load_build(build_path), store, *everything, questions)
    if only_stems:
        data = stems(store)
        if check:
            if STEMS.read_bytes() != data:
                fail("stems.tsv.zst is not what the eval's store and PyStemmer give now")
            return print("regenerate: stems.tsv.zst is current")
        STEMS.write_bytes(data)
        return print(f"regenerate: {len(data)} bytes of stems.tsv.zst")
    target = pathlib.Path(out) if out else HERE
    if corpus:
        sample = pick(load_build(build_path), store, corpus)
        (target / "sample.jsonl").write_bytes(dump(sample))
    sample = rows_of(target / "sample.jsonl")
    edges = rows_of(HERE / "edges.jsonl")
    data = dump(expected(store, sample, edges))
    if check:
        if (target / "expected.jsonl").read_bytes() != data:
            fail("expected.jsonl is not what the eval's store and bm25s give now")
        if STEMS.read_bytes() != stems(store):
            fail("stems.tsv.zst is not what the eval's store and PyStemmer give now")
        return print("regenerate: expected.jsonl and stems.tsv.zst are current")
    (target / "expected.jsonl").write_bytes(data)
    print(f"regenerate: {len(sample)} spans, {len(data)} bytes of expected.jsonl")


if __name__ == "__main__":
    main()
