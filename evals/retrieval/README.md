# Retrieval eval

Compare how well twelve memory strategies retrieve information from past coding sessions.
Each strategy builds a **store** from the same corpus, then retrieves context for questions
whose answers are in the sessions but absent from the project’s files.

The corpus contains 403 sessions from 22 projects. The eval scores 143 answerable questions
in seven classes, including decisions, work status, and procedures. Another 24 questions
have no answer and are reserved for an answer-generation eval that is not implemented yet.

## Run a small eval

You need Nix with flakes enabled, a running Docker engine, and Claude Code logged in to a
Claude subscription for judging. Commands below run from the repository root; the shell
scripts enter their pinned Nix environment automatically.

```sh
evals/retrieval/scripts/eval.sh bm25 --limit 3
```

This fetches the pinned corpus and store, searches three questions, and judges the retrieved
context. It prints scores and saves searches, contexts, and verdicts under
`~/.cache/bilbo-evals/retrieval/eval/`. Repeat the command to resume; omit `--limit 3` for
the full run. `BILBO_EVALS_CACHE` overrides the cache base directory.

Published stores avoid rebuilding memory. If no matching archive is available, the scripts
build one; most strategies then need an LLM endpoint. See the
[store instructions](reference.md#the-stores) before rebuilding.

## What the score means

Each question has a developer **prompt**, an agent’s search **query**, and evidence-backed
facts. The eval searches both forms, packs the returned text into 4,000 tokens, and asks
Claude Opus 5.5 which facts are present. A question scores the fraction of its facts found.
The headline is the weighted mean of class scores on the `query` form.

This measures retrieved context, not the quality of a generated answer. Searches are scoped
to one project, except bilbo-next’s unscoped row, which searches one store of every project,
and some adapters request more results than their tools’ defaults. Read the
[results and caveats](results/README.md) before comparing strategies.

## Find your way around

| Path | Purpose |
|---|---|
| [arms/](arms/README.md) | Strategies, store builders, and search adapters. |
| [questions/](questions/README.md) | Questions, reference facts, evidence, and class weights. |
| [scripts/](scripts/README.md) | Fetch, build, evaluate, validate, and publish. |
| [agent/](agent/README.md) | Check fixed read rules against a real agent. |
| [results/](results/README.md) | Saved scores and query-variation comparisons. |
| [dataset.lock](dataset.lock) | Archive revisions and checksums. |
| [reference.md](reference.md) | Full operating instructions, replay details, and research record. |

Archives live in the git-ignored `dataset/` directory. The scripts download pinned archives
and verify their checksums. See the [dataset card](dataset-card.md) for their format and
credits: the sessions come from SWE-chat, released under ODC-BY 1.0.
