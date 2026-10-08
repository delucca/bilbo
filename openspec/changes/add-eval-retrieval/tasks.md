# Tasks

## 1. Project scaffold, dev shell and CI

- [ ] 1.1 Add `uv` and `ripgrep` to the dev shell in `flake.nix` (not to the package's `lib.fileset`) and verify `nix develop -c uv --version` and `nix develop -c rg --version` print versions
- [ ] 1.2 Create `evals/pyproject.toml` with exact `==` pins (`ir-measures==0.4.3`, `bm25s==0.3.11`, `PyStemmer==3.1.0`, `numpy==2.5.3`; dev group `pytest==9.1.1`; build backend `uv_build==0.11.21`), `evals/.python-version` (`3.12.13`) and `evals/uv.lock`, and verify `nix develop -c uv --directory evals sync --locked` exits 0
- [ ] 1.3 Add `evals/runs/`, `evals/.venv/`, `evals/.cache/`, `__pycache__/` and `.pytest_cache/` under `evals/` to `.gitignore`, and verify `git check-ignore evals/runs/x evals/.venv/x evals/.cache/x` lists all three
- [ ] 1.4 Write the package skeleton `evals/src/bilbo_evals/` with `__init__.py`, `common.py` (exit codes, stderr prefix, hashing, JSONL, canary) and `schema.py` (row and record field lists with a validator), and verify `uv run python -c "import bilbo_evals.common, bilbo_evals.schema"` exits 0
- [ ] 1.5 Write `cli.py` with the whole `bilbo-evals` command tree and lazy dispatch to each module, and verify `uv run bilbo-evals --help`, `uv run bilbo-evals l1 run --help` exit 0 and `uv run bilbo-evals frobnicate` exits 2 with an empty stdout (`tests/test_cli.py`)
- [ ] 1.6 Write the shared test support (`tests/conftest.py` with `bilbo_bin` and `fixture_copy`, `tests/fake_embedder.py`, `tests/fakes/claude`, `tests/fakes/codex`) and `tests/test_project.py`, which fails naming any direct dependency not pinned with `==`; verify `uv run pytest tests/test_project.py tests/test_cli.py` passes
- [ ] 1.7 Add the `evals` job to `.github/workflows/ci.yml` (Nix install and cache by commit SHA, `nix develop -c cargo build --locked`, `uv sync --locked`, `uv run pytest` with `BILBO_BIN`), and verify `nix develop -c cargo test --locked --test workflows` passes

## 2. Words, dataset, review and the fixture

- [ ] 2.1 Write `words.py`: bilbo's word folding (`each_word`), the reused English and Portuguese stopword lists, Snowball stemming and distinctive tokens, and verify `uv run pytest tests/test_words.py` passes, including accent folding and the 2% document-frequency rule
- [ ] 2.2 Write `dataset.py` load, check (gold ids exist, splits by project, alias not in gold note, alias bridge, leakage of two or more distinctive tokens, no absolute home path in any file), `build_corpus` and `build_qrels`, and verify `uv run pytest tests/test_dataset.py -k check` passes for every negative scenario of the spec
- [ ] 2.3 Write `dataset.py` freeze, verify, `require_ready` and `materialize` (copy, never link), and verify `uv run pytest tests/test_dataset.py -k "freeze or verify"` covers a clean verify, an edited file, freezing twice and the draft refusal
- [ ] 2.4 Write `review.py` (`review sample`, `review check`, `review apply`) and verify `uv run pytest tests/test_review.py` prints `118/120 valid (98.3%)` for the passing case and exits 1 at 9 invalid of 120
- [ ] 2.5 Write `generate/filter.py` on top of the dataset checks (reject at two shared distinctive tokens, mark `zero_overlap`, write `generation/leakage.json`), and verify `uv run pytest tests/test_filter.py` passes
- [ ] 2.6 Build the invented fixture `tests/fixtures/notes-fixture/` with `tests/fixtures/build_fixture.py` (a dozen notes, a tiny corpus landed with `bilbo library stage <file>` and `land`, at least one query per stratum, digest prompts of every label, a passed review sheet), freeze it, and verify `uv run bilbo-evals dataset verify tests/fixtures/notes-fixture` prints its tree hash and `bilbo check` on a copy of its `store/` prints nothing

## 3. Isolation, embedder, passages and parity

- [ ] 3.1 Write `sandbox.py` (temporary root, cleared environment, bilbo config writer, the folder guard resolving real paths) and verify `uv run pytest tests/test_sandbox.py` passes, including the refusal when a resolved folder equals the invoking user's store
- [ ] 3.2 Write `passages.py`, the port of bilbo's passage splitting and embedder input, and verify `uv run pytest tests/test_passages.py` passes on headings, the preamble and 4,000-byte parts
- [ ] 3.3 Write `embedder.py` (pinned GGUF check, `llama-server` launch with bilbo's arguments, recording proxy, vector cache, index check) and verify `uv run pytest tests/test_embedder.py` passes against the fake embedder, including a refused GGUF hash and a `withheld` line
- [ ] 3.4 Write the `parity` command and verify `uv run pytest tests/test_parity.py` passes on a store whose inputs match and fails, naming the inputs, when the port is altered

## 4. Statistics and power

- [ ] 4.1 Write `stats.py`: the clustered sign-flip test, the paired cluster bootstrap, Holm, exact McNemar sizing and the design effect, and verify `uv run pytest tests/test_stats.py` passes against hand-computed values
- [ ] 4.2 Write the `power` command (refuses a test run, refuses after freeze, writes `preregistration.json`), and verify `uv run pytest tests/test_stats.py -k power` passes on the spec's 24% discordance and 1.3 design effect scenario

## 5. Generation: calls, world, facts, notes, fidelity, library

- [ ] 5.1 Write `llm.py`: the `claude -p` and `codex exec` invocations with memory kept out (clean flags, throwaway `CODEX_HOME`), the per-call preflight that fails closed, the call log, the call budget and rate-limit stops, and verify `uv run pytest tests/test_llm.py` passes with the fakes, including a leaked plugin, a missing `codex` and call 501 of 500
- [ ] 5.2 Write `generate/profile.py` and verify `uv run pytest tests/test_profile.py` shows only rounded numbers leave a store and a folder without `notes/` is refused
- [ ] 5.3 Write `generate/world.py` and `generate/facts.py` (seeded projects, aliases, facts, supersession, joins, kind pairs, alias bridges, timeline with matching ULIDs, noise plan) and verify `uv run pytest tests/test_world_facts.py` passes and is deterministic for a seed
- [ ] 5.4 Write `generate/notes.py` and `generate/fidelity.py` (render, verbatim check, model check, two re-renders, drop and log) and verify `uv run pytest tests/test_notes_fidelity.py` passes with the fakes, including a fact lost three times
- [ ] 5.5 Write `generate/library.py` (public-domain check, `stage <url>`, keep ranges and guide entries from the renderer, `land` into a temporary store, copy into `store/`) and verify `uv run pytest tests/test_library.py` passes offline with `stage <file>`

## 6. Generation: queries and digest prompts

- [ ] 6.1 Write `generate/queries.py` (intents per stratum from facts, query model never sees the gold note for paraphrase, PT/EN and alias, library queries from sections, counts from config or `preregistration.json`) and verify `uv run pytest tests/test_queries.py` passes with the fakes
- [ ] 6.2 Write `generate/prompts.py` (positive, noise, off-topic and near-miss digest prompts) and verify `uv run pytest tests/test_prompts.py` passes with the fakes

## 7. Arms and retrieval metrics

- [ ] 7.1 Write `arms/random.py`, `arms/ripgrep.py` and `arms/bm25_ref.py`, and verify `uv run pytest tests/test_arms_baselines.py` passes on the fixture, including the kind filter and zero-score documents dropped
- [ ] 7.2 Write `arms/dense_ref.py` and `arms/bilbo.py` (`--limit 100`, `--kind`, `--library`, `no notes match` and `no sources match` as empty, other failures as errors, fallback marks) and verify `uv run pytest tests/test_arms_bilbo.py` passes with the fake embedder
- [ ] 7.3 Write `retrieval.py` (TREC run with strictly decreasing scores, ir_measures over qrels, missing queries filled with 0, Evidence@10, new-above-old, empty share) and verify `uv run pytest tests/test_retrieval.py` passes
- [ ] 7.4 Document each arm's exact definition in `evals/docs/arms.md` and verify every arm name in `arms/__init__.py` has a section there (`tests/test_arms_baselines.py::test_arms_documented`)

## 8. Runs, records, digest, report and compare

- [ ] 8.1 Write `runner.py` (`l1 run`: verify or draft rules, sandbox, embedder, index check, arms, records) and `results.py` record writing, and verify `uv run pytest tests/test_runner.py` passes on the fixture, including the unfrozen, edited and draft-test refusals
- [ ] 8.2 Write `digest.py` (`l1 digest`: fresh session per prompt, `digest.log = on`, embedder at 0.55 and keyword-only, `--sweep` from 0.30 to 0.80) and verify `uv run pytest tests/test_digest.py` passes, with no line naming an AUROC
- [ ] 8.3 Write `report` (claim line first, all tables, DRAFT and exploratory labels, latency only for process arms, earlier test-run count) and `test-runs.jsonl` logging, and verify `uv run pytest tests/test_report.py` passes
- [ ] 8.4 Write `compare` (pairs by id, refuses on dataset or embedder mismatch, warns on `llama-server` version, names an edited baseline file) and verify `uv run pytest tests/test_compare.py` passes

## 9. Pooling

- [ ] 9.1 Write `pool.py` (every arm on every query and prompt, top 10 not gold, one checking call per item, quoted passage required, seeded audit of noes, `--apply` of resolutions into gold and qrels) and verify `uv run pytest tests/test_pool.py` passes with the fakes, and that `dataset freeze` refuses while a yes is unresolved

## 10. Documentation and end-to-end

- [ ] 10.1 Write `evals/README.md` (what L1 measures, the claim, commands, the data run order, the canary) and add the `bilbo-evals` commands and the rule that frozen datasets and baselines are generated, never edited, to `AGENTS.md`; verify every command in the README runs with `--help`
- [ ] 10.2 Write `tests/test_end_to_end.py`: verify, sandbox, index check and parity, all six arms, metrics, statistics, digest, report and compare on the fixture, and verify `uv run pytest tests/test_end_to_end.py` passes with no network

## 11. Data run: notes-synth/v1

- [ ] 11.1 Write `evals/datasets/notes-synth/v1/generation/config.toml` (seed, models, `max_calls`, concurrency) and run `generate profile` into the work folder for the user's approval; verify `world/profile.json` holds only rounded numbers
- [ ] 11.2 Generate the world, facts, all notes with fidelity, and the library; verify `bilbo-evals dataset check` reports no note or library problem and `bilbo check` on a copy of `store/` prints nothing
- [ ] 11.3 Generate the dev queries and digest prompts, filter them, and pool the dev split; verify `bilbo-evals dataset check` passes for dev
- [ ] 11.4 Have a reviewer agent that neither generated nor checked the items resolve every pooled yes, audit the noes and review the dev sample; verify `bilbo-evals review check` passes for dev
- [ ] 11.5 Run the draft dev run of `bilbo-full` and `bm25-ref` and `bilbo-evals power` on it; verify `preregistration.json` holds the inputs and counts per stratum
- [ ] 11.6 Generate, filter, pool and review the test split; verify `bilbo-evals review check` passes
- [ ] 11.7 Freeze the dataset; verify `bilbo-evals dataset verify evals/datasets/notes-synth/v1` prints the tree hash

## 12. Baseline 0.19.0

- [ ] 12.1 Run `l1 run --split test --arms all` and `l1 digest --split test --sweep` with the 0.19.0 release binary, copy the run to `evals/baselines/0.19.0/`, and verify `bilbo-evals compare evals/baselines/0.19.0 evals/baselines/0.19.0` exits 0 and `bilbo-evals report evals/baselines/0.19.0` prints every table

## 13. Integration checks

- [ ] 13.1 Run the full gate: `nix develop -c cargo fmt --check`, `nix develop -c cargo clippy --locked --all-targets -- -D warnings`, `nix develop -c cargo test --locked`, `nix flake check -L`, `nix develop -c uv --directory evals sync --locked`, `BILBO_BIN=$PWD/target/debug/bilbo nix develop -c uv --directory evals run pytest` and `openspec validate add-eval-retrieval --strict`, and verify all pass
