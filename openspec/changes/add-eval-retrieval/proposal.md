## Why

Nothing measures whether `bilbo recall` and the digest find the right notes. The only check today is a small, legacy query set over private notes, tuned on the same queries it scores, so no release can show that it found notes better or worse than the last one, or than a plain keyword search. A frozen public dataset and a harness that scores bilbo against simple baselines give every release a number that can be compared, published and rerun by anyone.

## What Changes

- Add `evals/`, a Python 3.12 project under `uv` with exact pins and a committed `uv.lock`, whose `bilbo-evals` command runs the L1 eval: the quality of retrieval (`bilbo recall`) and of the digest (`bilbo digest`).
- Add the synthetic dataset `notes-synth/v1` under `evals/datasets/`: about 600 agent-style notes generated facts first, with deliberate agent-style noise, a library corpus landed from real public-domain documents through `bilbo library stage` and `land`, queries in nine strata with gold labels split into dev and test by project, labelled digest prompts, and a record of every model and prompt used. It is frozen by a SHA-256 tree hash and never regenerated in place.
- Generate it without API keys: Claude Sonnet 5.5 renders notes through `claude -p`, and an OpenAI-family model words and checks queries through `codex exec`.
- Validate the labels before freezing: a lexical leakage filter, alias bridges that the corpus can reach, gold completed by pooling every arm's top hits, and a logged human review that must pass a threshold.
- Size the test split from discordance measured on dev and a preregistered minimum effect, instead of a fixed count.
- Run six arms on the same dataset: ripgrep, bilbo keyword-only, a reference BM25 with stemming, a dense-only reference, full bilbo, and a random floor. Score them with ir_measures 0.4.3 per stratum, with paired statistics against full bilbo and latency for the arms that run a process.
- Measure the digest at its default threshold of 0.55, the false-injection rate, coverage and hit-given-inject, plus a threshold sweep labelled as an approximate curve.
- Isolate every run in temporary `HOME`, XDG and `BILBO_HOME` folders, refuse to touch the real store or config, run bilbo's pinned local embedder, and check that bilbo indexed every note.
- Write one per-item result record per arm, a report, a comparison between runs, and a committed baseline for bilbo 0.19.0 on the test split.
- Add a small invented fixture dataset so the harness's own tests run offline in CI, and a CI job that runs them.

bilbo's own behavior does not change: no Rust code, CLI output or config key changes.

## Non-goals

- L0, regression floors in `cargo test`; L2, agent behavior with the plugin; L3, head-to-head comparisons with other memory tools; L4, coding outcomes. The dataset, the result record and the statistics are built so that L3 can reuse them later, but nothing here specifies L3.
- A true AUROC of the digest gate. It needs the gate's score in `digest.jsonl`, which bilbo does not log; adding it is a behavior change for a separate change.
- A dense-only mode inside bilbo. The dense-only arm is an outside reproduction.
- Running the harness in a container, or in CI against the real embedder. Runs happen by hand, before a release.
- Notes written by an agent through the `note` skill, and any claim beyond "retrieval over curated synthetic notes".
- The user's own notes as data. No private note, query or real store enters the dataset.
- A tracking server or hosted dashboard. Results are files in the repo.

## Capabilities

### New Capabilities

- `eval-retrieval`: the `bilbo-evals` harness in `evals/`: the frozen synthetic dataset and its generation and validation, the six L1 arms, the retrieval and digest metrics, the statistics, run isolation, the per-item result record, reports, comparisons and the committed baseline.

### Modified Capabilities

None. No requirement in `openspec/specs/` changes.

## Impact

- New: `evals/` (project, package, dataset, fixture, tests, baselines), with `evals/runs/`, `evals/.venv/` and `evals/.cache/` ignored by git.
- `flake.nix`: the dev shell gains `uv` and `ripgrep`. `evals/` stays out of the package's `lib.fileset`, since neither the build nor `cargo test` reads it.
- `.github/workflows/ci.yml`: a job that builds bilbo and runs the harness's tests offline on the fixture.
- `AGENTS.md`: the `bilbo-evals` commands and the rule that frozen datasets and baselines are generated, never edited.
- New Python dependencies: `ir-measures`, `bm25s`, `PyStemmer` and `pytest`, with their locked transitive trees.
- Tools used at generation time only: the `claude` and `codex` CLIs on a subscription login. At run time: a bilbo binary, `llama-server` and the pinned GGUF of bilbo's local embedder.
