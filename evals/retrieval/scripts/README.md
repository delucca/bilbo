# Eval scripts

Commands for fetching data, building stores, and measuring retrieval. Run them from the
repository root. Shell entry points enter the eval’s pinned Nix environment automatically;
see the [prerequisites](../README.md#run-a-small-eval).

| Command | Purpose |
|---|---|
| `evals/retrieval/scripts/fetch-corpus.sh` | Fetch and verify the pinned session corpus. |
| `evals/retrieval/scripts/build-store.sh bm25 --check` | Fetch or build a store, then validate it. |
| `evals/retrieval/scripts/eval.sh bm25 --limit 3` | Search and judge a small set of questions. |
| `evals/retrieval/scripts/eval.sh bm25` | Score every answerable question. |
| `evals/retrieval/scripts/gates.sh stress` | Test the judge against known missing, stale, and unrelated facts. |
| `evals/retrieval/scripts/gates.sh audit` | Cross-check sampled verdicts with a second model family through Codex. |
| `evals/retrieval/scripts/vary.sh` | Generate checked question variations; this updates the question dataset. |
| `evals/retrieval/scripts/eval.sh bm25 --variation` | Score the alternate wording separately. |

## Rows

A row is what is judged: one search packed one way, named by its rooms in tokens: `4000` (the chunks alone), `6500`, `4000-2500`
(the chunks in 4,000 and the notes in 2,500). `BILBO_EVAL_ROWS=4000,4000-2500 eval.sh bilbo-next` packs, judges and scores each
(`pack.py <search.jsonl> <contexts.jsonl> <budget> [<notes budget>]`); the default is `4000` alone, with the files it always had.
A row's contexts are `contexts-<row>.jsonl`, its verdicts `judged.jsonl` (`4000`) or `judged-<row>.jsonl`, and its score
`score.json` (`4000`) or `score-<row>.json`, so a row never takes another row's verdicts. The gold and random controls are
packed, judged and scored at 4000 only.

## Reuse and resume

Archives are kept in `evals/retrieval/dataset/`; work and run outputs are under
`~/.cache/bilbo-evals/retrieval/`. Set `BILBO_EVALS_CACHE` to override the cache base.
Repeat a command to resume cached work. Use `eval.sh <arm> --rerun <label>` for a separate
retrieval run, or `build-store.sh <arm> --rebuild` to rebuild memory. `--rebuild` removes the work folder and nothing else:
what a paid build of bilbo-next was answered, and what it spent, is in `<cache>/answers/<arm>` and stays
([bilbo-next's derive step](../arms/README.md#bilbo-nexts-derive-step)).

The Python files implement question selection, context packing, judging, scoring, and
comparisons. Their entry points document the arguments; use the shell commands above for
normal runs.

See the reference for [build configuration](../reference.md#the-stores),
[judge gates and run options](../reference.md#running-the-eval), and
[publishing archives](../reference.md#publishing). Publishing requires dataset-owner access;
building does not upload anything.
