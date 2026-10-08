## Context

See proposal.md for why. The harness measures bilbo 0.19.0 from the outside, through the contracts bilbo already publishes, and changes none of them:

- `bilbo recall` plain view (`docs/reference/commands.md#recall`): one block per note, `<path>:<line>\t<kind>\t<created>`, the heading path, a 300-character snippet, blocks separated by a blank line; exit 1 with `bilbo: no notes match` when nothing matches; `--` ends the options. `--library` prints `<path>:<line>\tsource|guide\t<ref>\t<a>-<b>` per file and is keyword-only (`src/search/recall.rs` `library`).
- Ranking (`src/search/rank.rs`): BM25 over passages (k1 1.2, b 0.75) on folded words without stemming, a meaning ranking over passage vectors above `embedder.min_similarity` (0.5), fused by reciprocal rank (k 60) over the first 50 of each list, then the keyword tail. One hit per note.
- Passages (`rank::passages`, `rank::parts`, `rank::input`): one per heading section plus the preamble, cut into parts of at most 4,000 bytes at paragraph breaks; the embedder input is the heading path joined with ` > `, a newline and the text, cut to 4,000 bytes. The query is `embedder.query_prefix` plus the query, cut to 2,000 bytes.
- `bilbo index` prints `embedded <n>, kept <k>, dropped <d>` and, when the embedder rule withholds passages, `withheld <n> passages from <url>: ...` on stderr. `recall` warns `embedder unavailable (...); keyword results only` and `<n> passages not indexed; run bilbo index`.
- `bilbo digest` reads `{"session_id", "prompt"}` on stdin, shows at most 6 notes on a session's first prompt, gates at `digest.min_similarity` (0.55) within a 1,500 ms budget, and with `digest.log = on` appends `{time, session, prompt, ranking, passed, shown, elapsed_ms, error?}` to `<state>/bilbo/digest.jsonl`, `ranking` being `none`, `keywords` or `meaning` (`src/search/digest.rs` `append_log`). No score is logged.
- The local embedder (`src/host/model.rs`): `Qwen3-Embedding-0.6B-Q8_0.gguf`, 639,150,592 bytes, SHA-256 `06507c7b…e439`, served as `qwen3-embedding-0.6b` by `llama-server` with `--embedding --pooling last --ctx-size 4096 --batch-size 4096 --ubatch-size 4096 --parallel 1` on `127.0.0.1`. Its query prefix is `QWEN_PREFIX` in `src/shared/config.rs`: `Instruct: Given a question, retrieve notes that answer it\nQuery: `.
- Folders (`docs/reference/configuration.md#folders`): store `$BILBO_HOME`, config `$BILBO_CONFIG`, cache and state under `XDG_CACHE_HOME` and `XDG_STATE_HOME`. The vector cache file is named after the store root's path and stores the model name, not the URL.
- Library (`docs/guides/library.md`): `bilbo library stage <url>` or `stage <file> --origin ... --fetched ...`, then `land <stage> <corpus>/<name> --keep <a>-<b>`, which writes the source, a guide entry holding `TODO: describe this source.`, and the capture under `.bilbo/captures/`. There is no `ingest` verb.

An earlier, private harness established rules that carry over: queries frozen by a SHA-256 that every run checks before it runs, fixed stratum counts checked by a script, a lexical-overlap check on distinctive tokens (four or more letters after accent folding, not in an English and Portuguese stopword list), and arms that score pure ranking with similarity floors off. Its stopword list and folding function are reused.

## Goals / Non-Goals

**Goals:**
- One shared core of our own: the dataset and its hash, the per-item record, the statistics and the comparison. Runners and arms plug into it.
- Numbers comparable across bilbo releases on one frozen dataset and one embedder, and later reusable by L3 without changing the core.
- Every number traceable to a dataset hash, a bilbo binary hash, an embedder hash and the prompts that made the data.

**Non-Goals:**
- Making the harness fast enough for CI with the real embedder. CI runs the fixture; real runs are by hand.
- A generic eval framework. The core serves L1 now and L3 next; L2 and L4 runners are not designed here.

## Decisions

### 1. Our own core on ir_measures, outside bilbo

The core is a Python package, `bilbo_evals`, in `evals/src/bilbo_evals/`. It scores with ir_measures over TREC files, the standard format for ranked retrieval. Rejected: Harbor or Inspect AI for L1 (they run agents in sandboxes; L1 runs no agent, and a container would put the embedder at a non-loopback host, which bilbo's embedder rule treats as remote); ranx (unmaintained since 2025-08); BEIR as a dependency (its format is used, not its code); DeepEval and Ragas (document-first query generation copies the note's words); a tracking server (plain files in git suffice).

The change extends no Rust module. It extends the dev shell in `flake.nix` with `uv` and `ripgrep`, and `.github/workflows/ci.yml` with one job. bilbo is reached only through its CLI and its documented files, so the harness keeps working whatever bilbo's internals do, and a change to those contracts shows up as a failed parity or parse check.

Layout:

```
evals/
  README.md  pyproject.toml  uv.lock  .python-version
  src/bilbo_evals/
    cli.py          the bilbo-evals entry point and its commands
    dataset.py      load, check, freeze, verify, materialize a store
    generate/       profile, world, facts, notes, fidelity, library, queries, prompts, filter
    llm.py          claude -p and codex exec calls, preflight, call log, budget
    pool.py         pooling, adjudication and `pool --apply`
    review.py       review sheet, check and `review apply`
    passages.py     the port of bilbo's passage splitting and input
    words.py        folding, stopwords, stemming, distinctive tokens
    sandbox.py      temporary roots, the guard, the bilbo config writer
    embedder.py     llama-server launch, the recording proxy, the vector cache
    arms/           random, ripgrep, bm25_ref, dense_ref, bilbo
    retrieval.py    run and qrels files, ir_measures, extra metrics
    runner.py       the `l1 run` orchestration
    digest.py       digest runs and the sweep
    stats.py        sign-flip test, bootstrap, Holm, power
    results.py      the record schema, run folders, report, compare
  datasets/notes-synth/v1/
  baselines/0.19.0/
  tests/            pytest, with fixtures/notes-fixture/
  test-runs.jsonl
```

### 2. Dependencies

| Dependency | Pin | Licence | Why, against the standard library and the others |
|---|---|---|---|
| `ir-measures` | 0.4.3 | Apache-2.0 | The reference implementations of Success, RR, nDCG, R and Judged over TREC files, through `pytrec-eval-terrier` (0.5.10, MIT, with cp312 wheels for macOS and Linux x86_64). Hand-written metrics would make our numbers incomparable with anyone else's. |
| `bm25s` | 0.3.11 | MIT | The reference BM25 arm. Pyserini needs Java 21 for one arm; `rank-bm25` has no stemming hook. 0.3.12 and 0.3.13 were published less than two weeks before this design, so 0.3.11 is pinned. |
| `PyStemmer` | 3.1.0 | MIT and BSD | Snowball stemmers for the BM25 arm (English) and for distinctive tokens in the leakage filter (English and Portuguese). The standard library has no stemmer. |
| `numpy` | 2.5.3 | BSD-3-Clause and others | Already required by `bm25s`. Used directly for cosine similarity over passage vectors and for vectorised bootstrap and sign-flip resampling. |
| `pytest` | 9.1.1 | MIT | The `dev` group only: the harness's tests. |

Everything else is the standard library: `argparse`, `json`, `tomllib` (generation config), `hashlib`, `subprocess`, `urllib.request` (embedder calls), `http.server` (the recording proxy and the tests' fake embedder), `math.comb` (power), `random`, `statistics`, `uuid`, `tempfile`, `unicodedata`. No scipy, no HTTP client library, no CLI framework, no schema library. Versions are exact in `pyproject.toml` and the whole tree is in `uv.lock`; a test asserts that every direct dependency is pinned with `==`.

Python is 3.12.13, pinned in `.python-version` and fetched by uv. Rejected: Python and packages from nixpkgs (no lock file per package, and nixpkgs lags PyPI), pip-tools (uv already gives a lock and an interpreter).

### 3. Generation: facts first, through subscription CLIs

Steps of `bilbo-evals generate <step>`, each resumable (it skips items whose output exists) and logged under `generation/`:

1. `profile`: read-only pass over a store given by path, writing aggregate numbers only, rounded (spec, Aggregate profile).
2. `world`: a seeded request to the renderer for at least 12 fictional projects built on real public technologies, with components, aliases (old names, codenames, abbreviations), and candidate facts: decisions, gotchas with exact error text, commands, paths, ports, plans, findings. Structured output, validated against a JSON Schema.
3. `facts`: a seeded script samples the fact graph: a kind per fact, validity intervals and supersession links, joins for multi-hop, a language per note from the profile's Portuguese share, an 18-month timeline of `created` times with ULIDs that match them, and which notes carry noise (omissions, near-duplicates, stale values).
4. `notes`: facts grouped into topics, one note per topic, as bilbo allows; about 30% fillers that carry no gold fact. The renderer writes each note from its fact manifest with the `note` skill's text from `plugins/bilbo/skills/note/` as the style guide, and never sees a real note. The script writes the frontmatter (`id`, `created`, optional `sources`).
5. `fidelity`: verbatim check of identifiers, numbers, paths and error strings, then the checking model confirms each fact; two re-renders, then the fact is dropped.
6. `library`: about 60 pages of the SQLite documentation, which is public domain, each staged from its URL with `bilbo library stage <url>` and landed with `bilbo library land` into corpus `sqlite` in a temporary store, keep ranges chosen by the renderer from the stage's line listing and recorded; guide entries are written by the renderer. The landed `library/` and `.bilbo/captures/` are copied into `store/`.
7. `queries` and `prompts`: per stratum, the script picks facts and builds an intent; the query model words it. For paraphrase, PT/EN and alias it sees the fact and the alias table, never the note. Library queries are worded from a section's content and are not leakage-filtered, because bilbo's library search is keyword-only by design; their overlap distribution is published. Digest prompts: positives phrased as task requests a note bears on, and noise, off-topic and near-miss negatives.
8. `filter`: the leakage rule (decision 6) and the alias bridge check; rejected queries go back to step 7.

Then `pool` (decision 7), `review` (decision 8), `power` (decision 9), and `dataset freeze`.

Model calls:
- Renderer: `claude -p --model claude-sonnet-5-5 --output-format stream-json --verbose --json-schema <schema> --tools "" --strict-mcp-config --setting-sources project --disable-slash-commands --no-session-persistence`, run in an empty temporary folder with every `CLAUDE*`, `ANTHROPIC_*`, `CODEX_*` and `BILBO_*` variable removed and `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1` set, so neither user settings (where plugins and hooks are enabled) nor MCP servers load. `--bare` is not used: it accepts only an API key.
- Query model and checker: `CODEX_HOME=<throwaway> codex exec -m <model> --ignore-user-config --ignore-rules --ephemeral --skip-git-repo-check -s read-only --disable plugins --disable hooks --disable apps --output-schema <schema> -o <file> -C <empty folder> --json`. `--ignore-user-config` alone still loads the user's `AGENTS.md` and skills from `CODEX_HOME`, so the throwaway home holds only a symlink to `auth.json`; after each step the harness checks whether Codex refreshed the token and, if so, copies the newer file back to the real home, only when its `last_refresh` is newer than the real file's.
- The preflight (spec, Generation keeps memory out) reads the `system/init` and hook events of every `claude` call (`--output-format stream-json --verbose`) and refuses when the session reports a plugin, hook, MCP server, skill or tool beyond the structured output. The `codex exec --json` events carry no such field, so the codex preflight checks that the throwaway home holds nothing but `auth.json`, then makes one counted probe call that must report no user instructions and no MCP tools; the `claude` preflight also makes one probe call that must report no user or project instructions (`NONE`), like the codex probe; every call's events may hold only agent messages and reasoning.
- The OpenAI-family model is pinned in `generation/config.toml` when generation starts and recorded with every call. It is neither Anthropic's family (the renderer's) nor Qwen's (the embedder's), so neither the notes' writer nor the dense arms share a family with the query writer.
- `generation/config.toml` also holds the seed and `max_calls`, the call budget. Generation is not bit-reproducible; the frozen output and the logged prompts are the record.

Rejected: API keys (none is available for this project, and the subscription CLIs are what agents use anyway); one model for everything (it would phrase queries the way it wrote the notes).

### 4. Dataset files

As the spec's Dataset contents lists. Details:
- `corpus.jsonl` is BEIR-style, so outside tools read it without bilbo. `metadata` holds `kind`, `created`, `lang`, `project`, and `source` for library rows.
- `queries.jsonl` `gen` holds the model and the prompt SHA-256; `family` is the fact family, the resampling unit; `evidence_sets` is a list of lists of note ids for multi-hop and empty otherwise.
- qrels hold gold at relevance 1, decoys and pooled non-relevant notes at 0.
- The canary string is in `README.md` and in a `canary` field of every JSONL row, never in a note: the note format allows no extra frontmatter key, and a comment in the body would enter passages, BM25 lengths and embedder inputs. `dataset freeze` adds the field to any row that lacks it before writing `MANIFEST`.
- Hashing follows the spec: `MANIFEST` of per-file SHA-256 sorted by path, `FROZEN` the SHA-256 of `MANIFEST`. `dataset.py` materializes a store by copying `store/` into the run's temporary root, never by linking, so bilbo cannot write into the dataset.

### 5. Strata counts and the thin slice first

The dev split is built first at the earlier plan's counts: known-item 20, paraphrase 30, PT/EN 25, alias 20, supersession 20, multi-hop 20, kind-filter 20, library 20, no-answer 10, and 100 positive plus 40 of each negative digest prompt. On that pilot, before anything is frozen, two arms run as drafts, `bilbo-full` and `bm25-ref`, which is the principal comparison; the labels are validated on dev; then `power` sizes the test split; then the test split is generated with the same proportions and at least 20 queries per stratum, pooled, reviewed and frozen with dev. This orders the work as thin slice, validation, then scale, so pooling never waits for a frozen generator.

Supersession follows bilbo's rule of one note per topic: the newer decision lives on a sibling topic and says it replaces the older one. An in-place rewrite is invisible to retrieval and is out of scope.

### 6. Leakage filter

Tokens are folded as bilbo folds words (runs of letters and digits, lowercased, accents removed; `src/shared/text.rs` `each_word`), then stemmed with Snowball in the query's language and the note's language. A token is distinctive when it has four or more letters, is not in the reused stopword list, and is in fewer than 2% of notes (at most one note when 2% of the corpus is under one note, as in the fixture). Rejection starts at two shared distinctive tokens, so a query may keep one natural anchor such as a product or error name: rejecting any shared token, as the earlier harness did over a title and first paragraph, would remove the words a real user types and make the strata artificially hard. Zero-overlap queries are tagged and reported as their own subset instead.

### 7. Pooling

`pool` runs all six arms, at the bilbo binary under evaluation (0.19.0 for v1), over every query and digest prompt, takes each arm's top 10 notes that are not gold, and asks the checking model per candidate, with the query and, for multi-hop, the current evidence sets: does it answer, or complete an evidence set, and which passage says so. The checker sees, per candidate note, its title and heading outline, the passage(s) each arm's hit pointed at, plus the top 2 passages by keyword overlap with the query (the prompt for digest items), deduplicated, each cut at 1,500 characters and at most 4,500 characters per candidate; multi-hop evidence-set judging keeps the same per-candidate view, and an item whose candidates exceed the prompt cap is split into calls and its judgments merged. Pool prompt text goes to the git-ignored cache and the call log commits only its SHA-256, and `generation/pool/records.jsonl` keeps, per call, what the prompt is built from (item text and evidence sets as judged, candidate ids with hit lines, seed, template name and SHA-256), so each pool prompt is rebuilt from its record, the corpus and the template (a changed template refuses); the frozen `queries.jsonl` alone cannot rebuild it, since `pool --apply` edits evidence sets after the call. A yes whose quote is not found after folding whitespace and Markdown emphasis counts as a no in the qrels, is flagged and needs a resolution. Every yes is resolved by a reviewer, and `pool --apply` writes the resolutions into the gold; 10% of each split's noes, at least 50, are audited, the sample topped up as judgments grow. Positive and near-miss digest prompts are pooled the same way; noise and off-topic prompts are not. So a near-miss that some note does answer becomes a positive or is dropped. Judged@10 in every report shows how much of a later system's top 10 the pool never judged, which is how L3 will see pool bias against new arms. Candidate and judgment rows carry the SHA-256 of the item's text, and each judgment the SHA-256 of its candidate's text and of the evidence notes it was shown, so a candidate or evidence note edited in review sends its item back to be judged again (the old judgments go to `superseded.jsonl`); an item is one call up to 200,000 characters of views; when it changes, `pool` moves the item's rows to `generation/pool/superseded.jsonl` and pools it again. An arm error or fallback stops pooling.

### 8. Review

The reviewer is a person or an agent that neither generated nor checked the item; the sheet records who reviewed. The bar is 95% valid, every invalid item fixed or dropped, and all supersession and multi-hop labels of the test split reviewed, because those are the labels the construction gets wrong most easily (temporal reading and evidence completeness). A failure that points at the generator (the same defect in several items) means fixing the generator and regenerating the affected step, not patching items.

### 9. Power

The principal comparison is `bilbo-full` against `bm25-ref` on success@5 over note queries of the test split: it answers whether bilbo beats a standard keyword search, which is the claim worth publishing. The minimum effect is 0.10 absolute success@5, α is 0.05 two-sided, the power target 0.80. `bilbo-keyword`, `ripgrep`, `dense-ref` and `random` are secondary, Holm-corrected among themselves.

`power` computes, from the dev draft run:
1. ψ, the share of dev note queries where the two arms disagree on success@5, and uses `ψ' = max(ψ, δ)` with δ the minimum effect.
2. The iid size: the smallest n for which the exact McNemar test at α rejects with probability ≥ 0.80 when `p10 = (ψ' + δ)/2` and `p01 = (ψ' − δ)/2`, by exact enumeration over the binomial count of discordant pairs (`math.comb`).
3. The design effect: the variance of the mean paired difference under a bootstrap over fact families, divided by its variance under a bootstrap over queries (10,000 seeded resamples each), floored at 1.
4. `n_test = ceil(n × design effect)`, split over strata in the dev proportions with a floor of 20 per stratum.

Exact McNemar is the right sizing test because, with one query per cluster, the sign-flip test of decision 10 on binary differences is exactly the conditional McNemar test; the design effect carries the clustering.

### 10. Statistics

- Per query, each arm's value is its metric, or for `random` the mean over seeds 0 to 19.
- Test: a paired sign-flip permutation test on per-query differences, with signs flipped per fact family (the sum of a family's differences flips as one), 10,000 seeded resamples, two-sided. It handles fractional values, so `random` needs no special case, and it respects clustering.
- Intervals: percentile 95% intervals from 10,000 seeded paired bootstrap resamples of fact families, for differences in MRR, nDCG@10 and R@10 (and success@5).
- Holm over the four secondary comparisons; a comparison missing from a run enters the family at p = 1; the principal comparison alone at α. Dev p-values are labelled exploratory; per-stratum results are descriptive.
- Rejected: exact McNemar as the test (it assumes independent queries and binary outcomes on both sides, so it cannot take the averaged random arm).

### 11. Isolation, not a container

`sandbox.py` makes `$TMPDIR/bilbo-evals-<run-id>/` with `home/`, `store/`, `config/bilbo/config`, `cache/`, `data/` and `state/`, and runs bilbo with `HOME`, `BILBO_HOME`, `BILBO_CONFIG` and the four XDG variables pointing there, plus `PATH`, `LANG`, `LC_ALL` and `TMPDIR`, nothing else. `run.json` records the root and folders as `$TMPDIR/bilbo-evals-<run-id>/...`, never the expanded path. The guard resolves the four bilbo folders with the precedence of `docs/reference/configuration.md#folders`, both for the run's environment and for the invoking environment, and refuses when a run folder is outside the root or equals a real one. The config is written in bilbo's format (`key = value`, the query prefix quoted with `\n`).

Rejected: one sealed container per run. It adds Docker on macOS and an embedder at `host.docker.internal`, which bilbo calls remote, and fixes none of L1's actual contamination risks, which are in the data.

### 12. The embedder, its proxy and the index check

`embedder.py` starts `llama-server` with the same arguments as `model::server_args`, on a free port of `127.0.0.1`, from `--model` (by default the GGUF in the invoking user's bilbo cache folder, opened read-only) after checking its SHA-256 against the pinned one, and from `--llama-server` (by default the one on `PATH`). It waits for `/health`, then checks one embedding.

During `bilbo index`, bilbo's `embedder.url` points at a recording proxy in the harness that forwards to `llama-server`, records every input, and stores vectors in `evals/.cache/<gguf-sha256>/<llama-server version>/` keyed by the input's SHA-256. After indexing, the run checks the index output (exit 0, no `withheld` line, `embedded` equal to the number of distinct inputs) and that the recorded inputs equal the passage port's inputs for the store; that one comparison is both the parity check and the "every note indexed" check. For queries the config is rewritten to point at `llama-server` directly, so latency has no proxy hop; bilbo's vector cache is keyed by model, not URL, so the vectors stay valid. `dense-ref` reads passage vectors from the proxy's cache, so it ranks with exactly the vectors bilbo stored, and embeds its queries through the same server.

Each recall's stderr is kept; `embedder unavailable` or `not indexed` marks the query `fallback: true`.

### 13. The arms

- `random`: `random.Random(seed)` shuffles note ids, seeds 0 to 19. The per-item file holds one row per seed (`trial` is the seed) with no ranking, since a seed reproduces it.
- `ripgrep`: for each distinct query word (folded as in decision 6, stopwords removed), `rg --ignore-case --word-regexp --fixed-strings --count-matches -e <word>` over `notes/`; a note's score is the number of distinct words found, then total matches, then path. The word is searched in its folded form and in every spelling it has in the query. Its latency is the wall time of all its processes for the query. Version recorded.
- `bm25-ref`: `bm25s` with method `lucene`, k1 1.2 and b 0.75 (bilbo's constants, so the difference is stemming and whole-note documents), English stopwords and the English Snowball stemmer, one document per note (title and body).
- `dense-ref`: decision 12; notes ranked by their best passage's cosine, no floor. It is not bilbo's meaning ranking: it skips the 0.5 floor and the fusion.
- `bilbo-keyword`: no `embedder.*` key in the config.
- `bilbo-full`: the pinned embedder, after the index check.
- Library queries run every arm over the library's sources (`dense-ref` through the same port, `--library` for bilbo). A guide hit keeps its rank and is never relevant.

### 14. Metrics

`retrieval.py` writes `run.trec` and calls `ir_measures.iter_calc` with `Success@5`, `RR`, `nDCG@10`, `R@10` and `Judged@10`. ir_measures skips queries absent from the run, so the harness fills those with 0 before averaging. Evidence@10 (multi-hop), new-above-old (supersession) and the empty-result share (no-answer) are computed by the harness from the rankings.

### 15. Digest runs

`digest.py` runs `bilbo digest` once per prompt with `{"session_id": <uuid4>, "prompt": ...}`, so every prompt is a session's first; it parses the block's `- <path>:<line> (...)` lines into note ids and reads `ranking`, `passed`, `shown` and `error` from `digest.jsonl`. The sweep reuses one indexed temporary root and rewrites only `digest.min_similarity`: 21 points over about 440 prompts per split. On a CPU embedder that is in the order of an hour; it is not on the release path unless asked. AUROC is not computed, because inject/no-inject at fixed thresholds leaves the score order inside each step unknown.

### 16. Results, report, compare, baseline

Run folders and records follow the spec. Rankings are cut to the top 100 ids. `run.json` records each arm folder's file hashes so `compare` can detect an edited baseline. `report` writes Markdown tables; `compare` pairs by item id and reuses the bootstrap of decision 10. `evals/baselines/0.19.0/` is the test-split run of the 0.19.0 release binary, all six arms and the digest at 0.55, plus the sweep. Repeated test runs are logged so that a later comparative claim can say how many times that test split was seen; a new published comparison after many runs uses a new dataset version with fresh test projects.

### 17. Fixture and CI

`evals/tests/fixtures/notes-fixture/` holds about a dozen invented notes, one tiny invented library corpus landed through `bilbo library stage <file>` and `land`, a few queries per stratum and digest prompts of every label, frozen with `MANIFEST` and `FROZEN`. The tests serve a fake embedder on loopback with `http.server`: a deterministic hashed bag-of-words vector, so `dense-ref` and `bilbo-full` produce real orders without a model. They run the whole path: verify, sandbox and guard, index check and parity, every arm, metrics, statistics, report and compare.

The CI job `evals` installs Nix as the `nix` job does, builds bilbo with `nix develop -c cargo build --locked`, then runs `nix develop -c uv sync --locked` and `nix develop -c uv run pytest` in `evals/` with `BILBO_BIN` pointing at the build. `evals/` stays out of the package's `lib.fileset`: neither the build nor `cargo test` reads it.

### 18. Reuse by L3

L3 adds arms and possibly a QA scorer. It needs no change to the dataset format (BEIR corpus, TREC qrels), the record (`tokens_in`, `tokens_out` and `cost_usd` already exist, null in L1), the statistics, or `compare`, which already refuses mismatched datasets and embedders. Arms share one interface: given the materialized store and a query, return ranked ids, with latency when they run a process.

## Risks / Trade-offs

- [Synthetic notes are cleaner than agent-written ones, and numbers may not transfer] → deliberate noise, matching the aggregate profile, the claim limited to curated synthetic notes in every report, and agent-written notes as a later check.
- [Gold misses relevant notes and punishes the better system] → pooling over every arm, reviewer resolution of every yes, an audit of noes, Judged@10 in every report.
- [The pool is built with 0.19.0 and the six arms; a later bilbo or a competitor can surface unjudged notes] → Judged@10 shows it; a large drop means pooling again into a new dataset version.
- [The generator's sessions load user memory despite the flags] → the `claude` preflight reads each session's own `system/init` report and the codex preflight checks the throwaway home and one probe call; both fail closed when an expected field is missing. Managed hooks cannot be disabled by flags, so the check requires their output to be empty.
- [Subscription rate limits stop generation midway] → resumable steps and the call budget.
- [The leakage threshold of two tokens is too loose or too strict] → the published overlap distribution and the zero-overlap subset make either visible; the review checks naturalness.
- [`llama-server` versions differ between runs and shift vectors slightly] → recorded per run; `compare` warns.
- [A bilbo release changes passage splitting] → parity fails and `dense-ref` is not scored until the port follows.
- [CPU time for runs and the sweep] → the vector cache per GGUF hash, and the sweep only when asked.
- [The design effect from a small dev set is noisy] → it is floored at 1, recorded, and the test size is preregistered before the test split exists.
- [Pinning `bm25s` behind its newest release] → a later re-pin is a recorded harness change; a result's arm definition includes its version.

## Migration Plan

No user-facing migration: bilbo's behavior is unchanged. Order of work: the project skeleton, fixture and CI job; the arms, metrics, statistics and records on the fixture; the generator and the dev pilot; the draft dev run and power sizing; the test split, pooling, review and freeze; the 0.19.0 baseline; the `AGENTS.md` lines. Rollback is deleting `evals/`, the CI job and the two dev-shell packages.

## Open Questions

- Which OpenAI-family model words the queries. It is recorded in `generation/config.toml` and every call; the choice changes no requirement.
- The value of the call budget, set before generation starts.
- How many SQLite pages the library takes beyond about 60, decided by how many library queries the test size needs.
