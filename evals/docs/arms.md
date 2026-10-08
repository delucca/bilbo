# The arms

Every arm ranks note ids (source references for library queries), best first, at most 100. A query that finds nothing
scores 0; it is not skipped. For `kind-filter` queries the arms that are not bilbo keep only notes of the asked kind;
the bilbo arms pass `--kind`. For `library` queries `ripgrep`, `bm25-ref` and `dense-ref` rank the dataset's sources
and the corpus guide files (id `<corpus>`, never relevant); the bilbo arms pass `--library`.

## random

All candidate notes in a seeded random order: `random.Random(seed).shuffle` over the sorted ids, for seeds 0 to 19. The
arm gives 20 rankings per query (trial = seed) and the report averages them. In-process, no latency.

## ripgrep

For each distinct query word (`words.words`, stopwords removed, accents and case folded) it runs
`rg --ignore-case --word-regexp --fixed-strings --count-matches -e <word> ...`, one `-e` for the folded word and one for each
distinct lowercase spelling the query uses for it (`migração` as well as `migracao`), over `<store>/notes` (or the corpus folders
of `<store>/library`) in the run's copy of the store. A note's score is the number of distinct words found in it, then
its total matches; ties go by path ascending. Notes with no match are not listed. Latency is the wall time of every
`rg` process of the query. The version is the first line of `rg --version`.

## bm25-ref

`bm25s.BM25(method="lucene", k1=1.2, b=0.75)` over one document per note (title and body), tokenised with English
stopwords and the Snowball English stemmer (PyStemmer), built once per run. A query retrieves `k = min(100, n_docs)`
documents; documents with score 0 are dropped (bm25s returns them), then notes of another kind than asked are removed.
In-process, no latency.

## dense-ref

The query is embedded as `QUERY_PREFIX + query`, cut at 2000 bytes, by the pinned embedder. Each note is split into
passages and each passage's embedder input is built as bilbo does (`passages.py`); the vectors come from the vector
cache that the checked `bilbo index` run filled. A note's score is the best cosine between the query and any of its
passages. There is no similarity floor: every note is ranked. The arm refuses to score when the index check reported a
parity difference or bilbo withheld a passage, and when a passage vector is missing from the cache. Sources are
embedded the same way, outside bilbo (its library is keyword-only). In-process, no latency.

## bilbo-keyword

`bilbo recall --limit 100 [--kind <kind>] [--library] -- <query>` in the sandbox with a config that holds no
`embedder.*` key. Passage hits collapse to their note, first hit first. Exit 1 with `no notes match` or
`no sources match` is an empty ranking with no error; any other non-zero exit is the query's error
(`exit <n>: <stderr>`). Latency is the wall time of the process.

## bilbo-full

The same call after `bilbo index` with the pinned embedder configured (`embedder.url`, `embedder.model`,
`embedder.query_prefix`). The run first checks that `bilbo index` exited 0, reported no withheld passage and embedded the
input of every passage; otherwise the arm is not scored. A query whose stderr holds `embedder unavailable` or
`not indexed` is marked `fallback: true`. Library queries are keyword-only in bilbo, so they match `bilbo-keyword`.
