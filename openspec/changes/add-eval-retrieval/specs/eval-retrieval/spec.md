## ADDED Requirements

### Requirement: The harness project
The L1 eval SHALL live in `evals/` of the repo as one Python 3.12 project managed by `uv`, with every direct dependency pinned to an exact version in `pyproject.toml`, a committed `uv.lock`, and the Python version pinned exactly in `.python-version`. Its one command SHALL be `bilbo-evals`, run with `uv run bilbo-evals` from `evals/`. Every `bilbo-evals` command SHALL exit 0 on success, 1 when it refuses the request or a check fails, and 2 on a usage error, and SHALL print its result on stdout and every diagnostic on stderr. `bilbo-evals --help` and `bilbo-evals <command> --help` SHALL print the synopsis and exit 0.

#### Scenario: Locked install
- **WHEN** `uv sync --locked` runs in `evals/` on a clean checkout
- **THEN** it installs exactly the versions in `uv.lock` and exits 0

#### Scenario: A floating version is refused
- **WHEN** `pyproject.toml` declares a dependency as `ir-measures>=0.4`
- **THEN** the harness's own tests fail and name the unpinned dependency

#### Scenario: An unknown command
- **WHEN** `uv run bilbo-evals frobnicate` runs
- **THEN** stderr names the unknown command, stdout is empty, and the exit code is 2

### Requirement: Dataset versions
A dataset SHALL live in `evals/datasets/<name>/<version>/`, and the first one SHALL be `evals/datasets/notes-synth/v1/`. A frozen dataset SHALL never be changed or regenerated in place: a change to its contents SHALL become a new version in a new folder. Every command that reads a dataset SHALL default to `evals/datasets/notes-synth/v1` and accept `--dataset <dir>`.

#### Scenario: A new version
- **WHEN** a frozen `notes-synth/v1` needs a corrected query
- **THEN** the correction is generated into `evals/datasets/notes-synth/v2/` and `v1` is unchanged

#### Scenario: Generation into a frozen dataset is refused
- **WHEN** `bilbo-evals generate queries --dataset evals/datasets/notes-synth/v1` runs and `v1/FROZEN` exists
- **THEN** stderr says the dataset is frozen, no file in `v1/` changes, and the exit code is 1

### Requirement: Dataset contents
A dataset folder SHALL hold: `README.md`, stating the generation method, the models, the counts per stratum and split, the claim "retrieval over curated synthetic notes", the licence of every library document, and a canary string; `store/`, a bilbo store root with `notes/`, `library/` and the `.bilbo/captures/` that `bilbo library land` wrote; `corpus.jsonl`, one row per note and per source with `_id`, `title`, `text` and `metadata`; `queries.jsonl`, one row per query with `id`, `text`, `stratum`, `split`, `lang`, `project`, `family`, `gold`, `evidence_sets`, `decoys`, `kind`, `fact_ids`, `zero_overlap` and `gen`; `qrels/<split>.txt` and `qrels/library-<split>.txt` in TREC qrels format; `digest/prompts.jsonl`, one row per digest prompt with `id`, `prompt`, `split`, `label` (`positive`, `noise`, `off-topic` or `near-miss`), `gold` and `project`; `world/`, the seed, the facts, the alias table and the aggregate profile; `generation/`, every prompt, model, parameter and log of the generation and validation; `preregistration.json`; `MANIFEST`; and `FROZEN`. Note ids in `gold`, `decoys`, `evidence_sets` and qrels SHALL be the notes' frontmatter `id` values, and library ids SHALL be the `<corpus>/<name>` reference `bilbo recall --library` prints.

#### Scenario: Notes are real bilbo notes
- **WHEN** `bilbo check` runs with `BILBO_HOME` set to a copy of the dataset's `store/`
- **THEN** it prints nothing and exits 0

#### Scenario: A gold id that is not a note
- **WHEN** a row of `queries.jsonl` lists a gold id that no note in `store/notes/` has
- **THEN** `bilbo-evals dataset check` prints the query id and the missing id, and exits 1

### Requirement: Freeze and verify
`bilbo-evals dataset freeze <dir>` SHALL write `MANIFEST`, one line `<sha256>  <path>` per file of the dataset folder except `MANIFEST` and `FROZEN`, sorted by path, and `FROZEN`, the SHA-256 of `MANIFEST`, which is the dataset's tree hash. It SHALL refuse when `FROZEN` already exists, when `bilbo-evals dataset check` fails, or when the validity review has not passed. `bilbo-evals dataset verify <dir>` SHALL recompute both hashes, print the tree hash and exit 0 when they match, and print each differing, missing or extra path and exit 1 otherwise. Every command that runs arms on a dataset SHALL verify it first and refuse on a mismatch; only `bilbo-evals pool` and `bilbo-evals l1 run --draft` SHALL run on a dataset that has no `FROZEN` yet.

#### Scenario: A clean dataset
- **WHEN** `bilbo-evals dataset verify evals/datasets/notes-synth/v1` runs on an unmodified checkout
- **THEN** stdout is the tree hash and the exit code is 0

#### Scenario: An unfrozen dataset without --draft
- **WHEN** `bilbo-evals l1 run --split dev --arms all --bilbo ./target/release/bilbo` runs, without `--draft`, on a dataset that has no `FROZEN`
- **THEN** stderr says the dataset is not frozen and suggests `--draft`, no run folder is written, and the exit code is 1

#### Scenario: An edited query
- **WHEN** one character of `queries.jsonl` changed after the freeze, and `bilbo-evals l1 run --split dev --arms all --bilbo ./target/release/bilbo` runs
- **THEN** stderr names `queries.jsonl` as differing from `MANIFEST`, no run folder is written, and the exit code is 1

#### Scenario: Freezing twice
- **WHEN** `bilbo-evals dataset freeze` runs on a folder that has `FROZEN`
- **THEN** stderr says the dataset is already frozen, `MANIFEST` and `FROZEN` are unchanged, and the exit code is 1

### Requirement: Generation models and record
The dataset SHALL be generated facts first: a seeded world of fictional projects built on real public technologies, then facts sampled from it, then notes rendered from the facts, then queries and digest prompts written from the facts. Notes SHALL be rendered by Claude Sonnet 5.5 through `claude -p`; queries SHALL be worded, and rendered facts and pooled candidates checked, by one OpenAI-family model through `codex exec`. Generation SHALL use the CLIs' own logins and no API key. Every model call SHALL be recorded in `generation/` with the CLI and its version, the model id, the parameters, the prompt file and its SHA-256, and the output. Generation SHALL stop at the call budget its configuration sets.

#### Scenario: Calls are recorded
- **WHEN** `bilbo-evals generate notes` renders 600 notes
- **THEN** `generation/` holds one record per call naming `claude`, its version, `claude-sonnet-5-5`, the prompt's SHA-256 and the output

#### Scenario: A missing CLI
- **WHEN** `bilbo-evals generate queries` runs and `codex` is not on `PATH`
- **THEN** stderr says `codex` is missing, no query is written, and the exit code is 1

#### Scenario: The budget runs out
- **WHEN** the configured call budget is 500 and step `notes` would need call 501
- **THEN** the step stops before that call, keeps what it wrote, says how many notes are left, and exits 1

### Requirement: Generation keeps memory out
A generation call SHALL run with no bilbo plugin, hook, skill, MCP server or user memory loaded in the CLI's session, so that no note of the user's store reaches a generation prompt. Before its first call, a generation step SHALL check the session each CLI starts and refuse when it reports a loaded plugin, hook or MCP server.

#### Scenario: A clean session
- **WHEN** `bilbo-evals generate notes` starts and the `claude` session it opens reports no plugin, hook or MCP server
- **THEN** generation proceeds

#### Scenario: A plugin leaks in
- **WHEN** the `claude` session the step opens reports the bilbo plugin loaded
- **THEN** stderr names the plugin, no generation call is made, and the exit code is 1

### Requirement: Aggregate profile
`bilbo-evals generate profile --store <root>` SHALL read a store without changing it and write `world/profile.json` holding only aggregate numbers: notes per kind, the distribution of note lengths, headings per note, and the shares of notes with code blocks, with `sources`, in Portuguese and with wiki links. It SHALL write no note text, title, topic, path, id or date of the store it read. The generator SHALL match the corpus to these numbers and to nothing else from that store.

#### Scenario: Only numbers leave the store
- **WHEN** `bilbo-evals generate profile --store ~/.local/share/bilbo` runs
- **THEN** `world/profile.json` holds counts, shares and length quantiles, no string from any note, and the store's files are unchanged

#### Scenario: Not a store
- **WHEN** the path has no `notes/` folder
- **THEN** stderr says it is not a bilbo store, nothing is written, and the exit code is 1

### Requirement: Fact fidelity and noise
Every planted fact SHALL be checked against the note rendered from it: the identifiers, numbers, paths and error strings of the fact SHALL appear verbatim, and the checking model SHALL confirm the fact can be read from the note. A note that fails SHALL be rendered again at most twice; a fact that still fails SHALL be dropped with its queries and logged. The corpus SHALL hold about 600 notes, of which some carry no gold fact, and SHALL include deliberate agent-style noise: notes that omit a detail, near-duplicate notes on one component, and stale values that a later note supersedes. `world/` SHALL record which notes carry each kind of noise.

#### Scenario: A fact survives rendering
- **WHEN** a fact holds the error string `SQLITE_BUSY: database is locked` and its note contains it verbatim
- **THEN** the fact is kept and its queries can use it

#### Scenario: A fact lost twice
- **WHEN** a note omits a planted port number in its first render and both re-renders
- **THEN** the fact and every query built on it are dropped, and `generation/` logs the drop with the note and fact ids

### Requirement: Strata and gold
Each note query SHALL belong to one stratum: `known-item` (names the subject in the note's own words), `paraphrase` (same intent in other words), `pt-en` (in the other language from its gold note, both directions), `alias` (names an entity by an alias its gold note never uses), `supersession` (asks for the current value; gold is the newer note, the older one is a decoy), `multi-hop` (needs two or more notes; gold is one or more evidence sets), `kind-filter` (asks with a kind; gold is the note of that kind while a note of another kind on the same subject exists), or `no-answer` (nothing in the corpus answers; gold is empty). Library queries SHALL form the `library` stratum, with their own qrels. For paraphrase, PT/EN and alias, the query model SHALL see the fact and the alias table but never the gold note. `no-answer` queries SHALL be reported only as a diagnostic, never in a metric that needs a relevant note.

#### Scenario: A supersession query
- **WHEN** a decision note created in March sets the retry limit to 3 and a later note replaces it with 5
- **THEN** the query asking for the current limit has the later note as gold and the earlier one in `decoys`, with relevance 0 in the qrels

#### Scenario: An alias the gold note uses
- **WHEN** an alias query names `Lantern` and its gold note contains the word `Lantern`
- **THEN** `bilbo-evals dataset check` prints the query id and exits 1

### Requirement: Alias bridges
Every alias an `alias` query uses SHALL be reachable in the corpus: at least one note other than the gold note SHALL contain both the alias and the name the gold note uses for that entity.

#### Scenario: A reachable alias
- **WHEN** the query names `Lantern`, the gold note says `edge-cache`, and another note says `Lantern, the old name of edge-cache`
- **THEN** `bilbo-evals dataset check` accepts the query

#### Scenario: An alias with no bridge
- **WHEN** no note contains both `Lantern` and `edge-cache`
- **THEN** `bilbo-evals dataset check` prints the query id and the alias, and exits 1

### Requirement: Splits by project
Every query and digest prompt SHALL be in the `dev` or the `test` split, and the split SHALL be decided by project: all queries, prompts and fact families of one project SHALL be in one split. Library queries SHALL be split by source. The world SHALL hold at least 12 projects. Tuning SHALL use only `dev`.

#### Scenario: One project, one split
- **WHEN** `bilbo-evals dataset check` reads a dataset whose project `ledger` has queries only in `test`
- **THEN** it accepts the split

#### Scenario: A project in both splits
- **WHEN** project `ledger` has one query in `dev` and the rest in `test`
- **THEN** `bilbo-evals dataset check` prints the project and both query ids, and exits 1

### Requirement: Lexical leakage
For `paraphrase`, `pt-en` and `alias` queries, a distinctive token SHALL be a word of four or more letters after accent folding and stemming, not an English or Portuguese stopword, found in fewer than 2% of the notes. A query of these strata that shares two or more distinctive tokens with its gold note SHALL be rejected and rewritten. A query that shares none SHALL be marked `zero_overlap: true`; those queries SHALL form a diagnostic subset reported apart, and the filter SHALL NOT require zero overlap. `generation/` SHALL hold the distribution of shared distinctive tokens per stratum.

#### Scenario: One natural anchor is allowed
- **WHEN** a paraphrase query shares only the distinctive token `wal` with its gold note
- **THEN** the query is kept with `zero_overlap: false`

#### Scenario: An echoing query is rejected
- **WHEN** a paraphrase query shares `checkpoint` and `fsync` with its gold note, both distinctive
- **THEN** `bilbo-evals dataset check` prints the query id and both tokens, and exits 1

### Requirement: Pooled gold completeness
Before a dataset is frozen, `bilbo-evals pool` SHALL run every arm of the eval, bilbo included, on every query and digest prompt of both splits, and collect each one's top 10 notes that are not gold. The checking model SHALL judge each collected note: whether it answers the query on its own, or, for `multi-hop`, whether it completes an evidence set, quoting the passage that supports a yes. Every yes SHALL be resolved by a reviewer, by adding the note to the gold or an evidence set, or by rewriting or dropping the query; a sample of the noes SHALL be audited by a reviewer. Judged notes SHALL enter the qrels with relevance 0 or 1. `generation/` SHALL log every judgment and resolution.

#### Scenario: A second note also answers
- **WHEN** the pool for query `q-ledger-12` holds a note the checking model judges to answer it, and the reviewer agrees
- **THEN** the note is added to the query's gold with relevance 1 in the qrels, and the log records the judgment and the resolution

#### Scenario: An unresolved yes blocks the freeze
- **WHEN** one pooled yes has no resolution
- **THEN** `bilbo-evals dataset freeze` prints its query and note ids, writes no `FROZEN`, and exits 1

### Requirement: Validity review
`bilbo-evals review sample` SHALL write a seeded review sheet to `generation/review/`: every `supersession` and `multi-hop` query of the test split, at least 5 queries per other stratum per split, at least 10 digest prompts per label per split, and 20 notes. A reviewer SHALL mark each item valid or invalid with a reason. `bilbo-evals review check` SHALL pass only when at least 95% of the reviewed items are valid and every invalid item is fixed or dropped; it SHALL print the share and the open items.

#### Scenario: The review passes
- **WHEN** 2 of 120 reviewed items were invalid and both were fixed
- **THEN** `bilbo-evals review check` prints `118/120 valid (98.3%)`, no open items, and exits 0

#### Scenario: Too many invalid items
- **WHEN** 9 of 120 reviewed items were invalid
- **THEN** `bilbo-evals review check` prints the share and exits 1, and `bilbo-evals dataset freeze` refuses

### Requirement: The library stratum
The library SHALL be built from real documents under a public-domain licence, added through bilbo's own library verbs, `bilbo library stage` and `bilbo library land`, into the dataset's `store/library/`, so that every source has the id and frontmatter bilbo gave it. The origin and licence of every document SHALL be recorded in `world/` and `README.md`. A library query's gold SHALL be a source reference, with the heading path of the section that answers it.

#### Scenario: A landed source
- **WHEN** a SQLite documentation page is added to the dataset
- **THEN** `store/library/sqlite/<name>.md` has the frontmatter `bilbo library land` wrote, and `world/` records its origin URL and licence

#### Scenario: A document without a known licence
- **WHEN** a candidate document's licence is not recorded as public domain
- **THEN** `bilbo-evals generate library` skips it, says so on stderr, and adds nothing to `store/library/`

### Requirement: Preregistered power
`bilbo-evals power <dev-run>` SHALL read a dev run of the principal comparison, measure on it the share of queries where the two arms disagree on success@5 and the design effect of clustering by fact family, and compute the number of test note queries that gives at least 80% power to detect the preregistered minimum effect at a two-sided α of 0.05. It SHALL write `preregistration.json` with the primary metric, the principal comparison, the minimum effect, α, the power target, the measured discordance, the design effect, the resulting count per stratum and the secondary comparisons. It SHALL refuse a test run as input, and SHALL refuse to change `preregistration.json` once the dataset is frozen.

#### Scenario: Sizing from dev
- **WHEN** the dev run shows full bilbo and the reference BM25 disagreeing on 24% of note queries, with a design effect of 1.3, and the minimum effect is 0.10
- **THEN** stdout prints the required test count and `preregistration.json` holds all of those inputs and the count per stratum

#### Scenario: A test run as input
- **WHEN** `bilbo-evals power` is given a run of the test split
- **THEN** stderr says power is sized on dev only, nothing is written, and the exit code is 1

### Requirement: Run isolation
Every run SHALL give bilbo a fresh temporary root holding `HOME`, `BILBO_HOME`, `BILBO_CONFIG`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME` and `XDG_STATE_HOME`, an environment cleared of every other variable except `PATH`, the locale and `TMPDIR`, a copy of the dataset's store, and a config written by the harness. Before running bilbo, the harness SHALL resolve the store root, config, cache and state folders as bilbo does for that environment, and refuse when any of them is outside the temporary root or equal to the store or config the invoking user's environment resolves. `run.json` SHALL record the temporary root and the resolved folders. No run SHALL write to the user's real store, config, cache or state folders.

#### Scenario: A run stays in its root
- **WHEN** `bilbo-evals l1 run --split dev --arms all --bilbo ./target/release/bilbo` finishes
- **THEN** `run.json` lists the temporary root and four resolved folders inside it, and no file under the invoking user's bilbo store, config, cache or state folders changed

#### Scenario: A leaking path is refused
- **WHEN** a run's resolved store root equals the invoking user's `~/.local/share/bilbo`
- **THEN** stderr names the path, bilbo is never started, and the exit code is 1

### Requirement: The pinned embedder
A run that needs an embedder SHALL start its own `llama-server` on a free loopback port with bilbo's local embedder settings, from a GGUF file whose SHA-256 equals the one bilbo pins for `qwen3-embedding-0.6b`, and SHALL configure bilbo with `embedder.model = qwen3-embedding-0.6b` and the query prefix bilbo's setup writes for that model. `run.json` SHALL record the model, the GGUF's SHA-256, the `llama-server` version and the query prefix.

#### Scenario: The pinned model
- **WHEN** `--model` points at `Qwen3-Embedding-0.6B-Q8_0.gguf` with SHA-256 `06507c7b42688469c4e7298b0a1e16deff06caf291cf0a5b278c308249c3e439`
- **THEN** the run starts `llama-server` on `127.0.0.1` and `run.json` records the hash and the server version

#### Scenario: Another model
- **WHEN** `--model` points at a file with any other SHA-256
- **THEN** stderr prints both hashes, no arm runs, and the exit code is 1

### Requirement: Every note indexed
Before scoring an arm that uses bilbo's embedder, the run SHALL check that `bilbo index` exited 0, reported no withheld passage, and embedded the input of every passage with text in every note, and SHALL refuse to score the arm otherwise. During the run, every query on which bilbo reported a keyword fallback, an unavailable embedder or passages not indexed SHALL be marked in its result record.

#### Scenario: All passages embedded
- **WHEN** `bilbo index` exits 0 with no withheld line and every passage of the store was embedded
- **THEN** the `bilbo-full` arm is scored

#### Scenario: A withheld passage
- **WHEN** `bilbo index` prints `withheld 2 passages` on stderr
- **THEN** the arm is not scored, stderr repeats bilbo's line, and the exit code is 1

#### Scenario: A fallback during queries
- **WHEN** the embedder stops answering after 40 queries and bilbo prints `embedder unavailable` for the rest
- **THEN** those queries carry `fallback: true` in `per_item.jsonl`, and the report shows the count of fallback queries for the arm

### Requirement: The arms
`bilbo-evals l1 run` SHALL run these arms, each documented in `evals/` with its exact definition: `random`, a seeded random order of all notes averaged over 20 seeds; `ripgrep`, notes ordered by how many distinct query words `rg` finds in each, then by total matches, then by path; `bm25-ref`, BM25 with Snowball stemming over whole notes; `dense-ref`, notes ordered by the best cosine similarity of their passages to the query under the pinned embedder, with no similarity floor; `bilbo-keyword`, `bilbo recall` with no embedder configured; and `bilbo-full`, `bilbo recall` with the pinned embedder after `bilbo index`. bilbo arms SHALL call `bilbo recall --limit 100`, with `--kind` for `kind-filter` queries and `--library` for library queries; other arms SHALL keep only notes of the asked kind for `kind-filter` queries. A bilbo exit of 1 with `no notes match` SHALL score as an empty list; any other failure SHALL be recorded as that query's error.

#### Scenario: A kind-filter query on a baseline
- **WHEN** `bm25-ref` ranks a `plan` note first for a query asked with kind `decision`
- **THEN** the plan note is removed from the arm's list for that query before scoring

#### Scenario: Nothing matches
- **WHEN** `bilbo recall` exits 1 and prints `bilbo: no notes match` for a query
- **THEN** the query's record has an empty ranking, every metric 0 and no error

#### Scenario: bilbo crashes on a query
- **WHEN** `bilbo recall` exits 101 for a query
- **THEN** the query's record holds the exit code and stderr as its error, and the report counts it among the arm's errors

### Requirement: Dense reference parity
The `dense-ref` arm SHALL split notes into passages and build each passage's embedder input exactly as bilbo does. `bilbo-evals parity` SHALL check this by running `bilbo index` on a store and comparing the set of inputs bilbo sent to the embedder with the set the harness builds for the same store; it SHALL print the inputs that differ and exit 1 on any difference. A run of `dense-ref` SHALL record the parity result for the bilbo binary it ran against, and SHALL refuse to score when parity fails.

#### Scenario: Parity holds
- **WHEN** `bilbo-evals parity --bilbo ./target/release/bilbo` runs on the dataset's store
- **THEN** it prints the number of inputs compared, no difference, and exits 0

#### Scenario: bilbo changed its splitting
- **WHEN** a bilbo build cuts passages at 3,000 bytes instead of 4,000
- **THEN** `bilbo-evals parity` prints the differing inputs and exits 1, and `dense-ref` is not scored in a run against that build

### Requirement: Retrieval metrics
Retrieval SHALL be scored with ir_measures 0.4.3 from TREC run and qrels files: success@5, MRR, nDCG@10 and R@10, overall over note queries, per stratum, for the zero-overlap subset, and for the library stratum apart. `multi-hop` SHALL also report the share of queries with every note of some evidence set in the top 10; `supersession` SHALL also report the share where the gold note ranks above every decoy; every arm SHALL report Judged@10. A query for which an arm returns nothing SHALL score 0, not be skipped. `no-answer` SHALL report the share of queries for which each arm returned nothing.

#### Scenario: A stratum table
- **WHEN** `bilbo-evals report` reads a run
- **THEN** it prints one row per arm and stratum with n, success@5, MRR, nDCG@10 and R@10

#### Scenario: An empty ranking counts
- **WHEN** `bilbo-keyword` returns nothing for 12 of 40 paraphrase queries
- **THEN** those 12 count as 0 in the stratum's means, whose n is 40

### Requirement: Paired statistics
The primary metric SHALL be success@5 over note queries of the test split, and the principal comparison SHALL be the one `preregistration.json` names. Each other arm SHALL be compared with `bilbo-full` per query, with a paired test whose resampling unit is the fact family and that accepts a per-query value averaged over seeds, so that the `random` arm is compared like the others. The principal comparison SHALL be tested at α 0.05; the secondary comparisons SHALL be Holm-corrected among themselves. MRR, nDCG@10 and R@10 differences SHALL carry 95% intervals from a paired bootstrap over fact families. Per-stratum differences SHALL be labelled descriptive.

#### Scenario: The principal comparison
- **WHEN** a test-split run holds `bilbo-full` and `bm25-ref`, and `preregistration.json` names that pair
- **THEN** the report prints the difference in success@5, its interval and its p-value, marked as the principal test

#### Scenario: The random floor
- **WHEN** the report compares `bilbo-full` with `random`
- **THEN** it uses each query's success@5 averaged over the 20 seeds, and prints a Holm-adjusted p-value, not an error

#### Scenario: A dev run is not confirmatory
- **WHEN** the report reads a dev-split run
- **THEN** every p-value is labelled exploratory

### Requirement: Latency
For the arms that start a process per query (`ripgrep`, `bilbo-keyword` and `bilbo-full`), each result record SHALL hold the wall time from start to exit in milliseconds, and the report SHALL print p50 and p95 per arm. In-process arms SHALL record no latency.

#### Scenario: Process arms
- **WHEN** the report reads a run with all six arms
- **THEN** it prints p50 and p95 for `ripgrep`, `bilbo-keyword` and `bilbo-full` only

#### Scenario: An in-process arm
- **WHEN** a record of `bm25-ref` is read
- **THEN** its `latency_ms` is null and the report prints `-` for that arm's latency

### Requirement: Digest metrics
`bilbo-evals l1 digest` SHALL run `bilbo digest` on every digest prompt of a split, each with a fresh session id and `digest.log = on`, once with the pinned embedder at `digest.min_similarity = 0.55` and once with no embedder. It SHALL report, per configuration: the false-injection rate for each of `noise`, `off-topic` and `near-miss`, the share of prompts that got any digest; coverage, the share of `positive` prompts that got a digest; and hit-given-inject, the share of positive prompts with a digest that lists a gold note. Results SHALL be stratified by the `ranking` and `error` bilbo logged in `digest.jsonl`. With `--sweep`, it SHALL repeat the embedder configuration for `digest.min_similarity` from 0.30 to 0.80 in steps of 0.025 and print the points as a curve labelled "approximate operating curve", with 0.55 marked; it SHALL NOT print an AUROC.

#### Scenario: The operating point
- **WHEN** `bilbo-evals l1 digest --split dev --bilbo ./target/release/bilbo` finishes
- **THEN** the report prints the three false-injection rates, coverage and hit-given-inject at 0.55, and the same for the keyword-only configuration

#### Scenario: A fallback is not hidden
- **WHEN** `digest.jsonl` logs `ranking: keywords` with an error for 7 prompts of the embedder configuration
- **THEN** the report prints those 7 apart from the prompts ranked by meaning

#### Scenario: No AUROC
- **WHEN** `--sweep` is given
- **THEN** the curve is labelled approximate and no line of the report names an AUROC

### Requirement: Result records
A run SHALL write `evals/runs/<run-id>/`, with one folder per arm holding `run.json`, `per_item.jsonl` and, for ranking arms, `run.trec`. `run.json` SHALL record the schema version, run id, layer `L1`, arm, split, the dataset name, version and tree hash, the bilbo version, commit and binary SHA-256, the embedder record, the bilbo config keys used, the versions of Python, the harness's dependencies and every external tool, the seeds, the host's platform and CPU, the start and end times, whether the run is a draft, and the SHA-256 of every other file in the arm's folder. `per_item.jsonl` SHALL hold one row per item and trial with `item`, `stratum`, `split`, `trial`, `ranking`, `metrics`, `latency_ms`, `exit`, `warnings`, `fallback`, `error`, `tokens_in`, `tokens_out` and `cost_usd`, the last three null in L1. `evals/runs/` SHALL be ignored by git.

#### Scenario: One record per query
- **WHEN** a dev run of `bilbo-full` covers 185 queries
- **THEN** its `per_item.jsonl` has 185 rows, each with the fields above

#### Scenario: Draft runs are marked
- **WHEN** `bilbo-evals l1 run --draft --split dev` runs on a dataset without `FROZEN`
- **THEN** `run.json` has `draft: true` and the report prints `DRAFT` in its title

#### Scenario: A draft test run is refused
- **WHEN** `bilbo-evals l1 run --draft --split test` runs
- **THEN** stderr says draft runs use dev only, nothing is written, and the exit code is 1

### Requirement: Report
`bilbo-evals report <run>` SHALL print to stdout, and write as `report.md` in the run folder, the dataset and bilbo identity, the per-arm and per-stratum metrics, the zero-overlap and library tables, the paired statistics, the latency table, the digest results when present, and the counts of errors and fallbacks per arm. Its first lines SHALL state that the numbers measure retrieval over curated synthetic notes.

#### Scenario: A full report
- **WHEN** `bilbo-evals report evals/runs/<run-id>` reads a run with six arms and a digest run
- **THEN** stdout and `report.md` hold every table above and the claim line

#### Scenario: Not a run folder
- **WHEN** the path holds no `run.json`
- **THEN** stderr says the folder is not a run, and the exit code is 1

### Requirement: Comparing runs
`bilbo-evals compare <baseline> <run>` SHALL pair items by id and print, per arm and stratum, the metric deltas with the intervals of the paired statistics. It SHALL refuse when the dataset tree hashes differ, or when the embedders' model, GGUF hash or query prefix differ; it SHALL print a warning when only the `llama-server` versions differ, and SHALL print latency deltas only when both runs record the same platform and CPU.

#### Scenario: Two releases
- **WHEN** both runs used `notes-synth/v1` with the same tree hash and the same embedder record
- **THEN** stdout holds the deltas and the exit code is 0

#### Scenario: Different datasets
- **WHEN** the runs record different tree hashes
- **THEN** stderr prints both hashes, nothing is compared, and the exit code is 1

### Requirement: Test-split runs are logged
Every non-draft run of the test split SHALL append a line to `evals/test-runs.jsonl` with the run id, the dataset tree hash, the bilbo version and the time, and the report of a test run SHALL print how many test runs that dataset had before it.

#### Scenario: A logged test run
- **WHEN** a test run of bilbo 0.19.0 is the third on `notes-synth/v1`
- **THEN** `evals/test-runs.jsonl` gains its line and the report prints `2 earlier test runs on this dataset`

#### Scenario: A dev run is not logged
- **WHEN** a dev run finishes
- **THEN** `evals/test-runs.jsonl` is unchanged

### Requirement: The committed baseline
`evals/baselines/0.19.0/` SHALL hold the test-split run of bilbo 0.19.0 on `notes-synth/v1` for all six arms and the digest, as written by the harness, with its `report.md`. A baseline SHALL never be edited by hand, and `bilbo-evals compare` SHALL accept it as `<baseline>`.

#### Scenario: Comparing with the baseline
- **WHEN** `bilbo-evals compare evals/baselines/0.19.0 evals/runs/<run-id>` runs with a test run of a later bilbo on the same dataset and embedder
- **THEN** it prints the deltas against 0.19.0

#### Scenario: An edited baseline
- **WHEN** a row of `evals/baselines/0.19.0/bilbo-full/per_item.jsonl` no longer matches the hash its `run.json` records for that file
- **THEN** `bilbo-evals compare` names the file and exits 1

### Requirement: Offline tests on a fixture
`evals/` SHALL hold a small invented fixture dataset, frozen like a real one, with at least one query of every stratum, a library corpus and digest prompts of every label. The harness's own tests SHALL run on it offline, with a fake embedder that the tests serve on loopback and a bilbo binary built from the same checkout, and CI SHALL run them on every pull request.

#### Scenario: CI runs the harness tests
- **WHEN** a pull request changes a file in `evals/`
- **THEN** CI builds bilbo, runs `uv sync --locked` and the harness's tests in `evals/`, and the job fails when a test fails

#### Scenario: No network in tests
- **WHEN** the harness's tests run with no network
- **THEN** they pass, and no test starts `llama-server`, `claude` or `codex`
