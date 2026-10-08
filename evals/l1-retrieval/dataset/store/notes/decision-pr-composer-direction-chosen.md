---
id: 01M2K6WG7FP8PH6VZTE95Q2D76
created: 2026-09-15T15:57-03:00
---

# pr-composer: general direction for building upgrade pull requests

We settled the overall approach for pr-composer. This note keeps the direction only. It does not hold tuned settings. Those live in config and in the code, and they change more often than the reasoning here.

pr-composer takes the output of the upgrade planner and the targeted test results, and turns them into a pull request that a platform engineer can review quickly. The choice below is to keep it a thin, deterministic formatter on top of data other components already produced, and not to let it grow into a second planner.

## What we chose

pr-composer builds the pull request from recorded facts. It does not re-run analysis, and it does not guess. If the planner said a package moved, the PR says that. If the test runner reported a result, the PR repeats it as reported. When a fact is missing, the PR says it is missing and does not fill the gap with a plausible sentence.

Output is the same for the same input. We want to be able to diff two composed PRs and see only real differences. That rules out free-form generated prose in the main body, and it rules out anything that depends on wall-clock ordering or on map iteration order.

One PR per logical upgrade unit, as the planner defines the unit. pr-composer does not decide grouping. It only renders what it is handed. If people dislike the grouping, the fix goes in the planner.

## Why

Reviewers who maintain many repositories read a lot of these. They want the same layout every time so they can find the test summary and the risk notes without hunting. A predictable shape matters more than a clever one.

Earlier experiments with richer, more narrative descriptions made reviews slower, since people had to check whether the narrative matched the data. Keeping the body mechanical removes that check.

Determinism also helps tests. Snapshot-style tests on composed output are cheap to write and cheap to read, and they only stay useful if the output does not wobble.

## Shape of the PR content

The body has a fixed set of sections in a fixed order: what changed, why it was proposed, what was tested, what was not tested, and what to look at by hand. The "not tested" part is deliberate. Targeted tests are a subset by design, and the PR should be honest about the gap instead of implying full coverage.

Release notes and changelog excerpts from upstream are included only as quoted, trimmed material, clearly marked as upstream text. We do not summarize them ourselves. Long excerpts are cut and linked, not pasted whole.

Labels, titles and branch naming follow a convention that is stable across repositories, so that tooling and humans can filter on them. Per-repository overrides are allowed but should be rare, and they come from configuration, not from code branches inside pr-composer.

## Boundaries with other components

pr-composer reads from the planner output and from the stored test results. It does not talk to the package registry directly. It does not run tests. It does not write to the database except for recording that a PR was composed and opened, so reruns can find it.

Talking to the hosting API is kept at the edge. The composing step produces a plain description of the PR, and a separate small step sends it. This lets us test composition without network stubs and lets us dry-run in CI workflows to see what would be opened.

Reruns update an existing PR for the same upgrade unit instead of opening another. Finding the existing one relies on what we stored when we opened it, not on searching titles.

## Failure handling

If composition fails for one unit, that unit is reported and skipped. The rest of the batch continues. A half-built PR is worse than none, so we do not open partial ones.

If the hosting API rejects or throttles a request, pr-composer retries in a bounded way and then surfaces the failure in the run summary. It should never loop quietly. The exact limits are configuration.

Logs say which unit and which stage failed, and do not dump full PR bodies unless debug output is turned on.

## Open points

- How much of the risk assessment belongs in the PR body versus a linked report is not settled. For now the body carries a short version only.
- Whether to let teams supply their own body templates is undecided. Leaning no until we see real demand, since templates weaken the same-layout-every-time goal.
- Handling of very large batches in a single run may need a second look once real usage shows up.

## When to revisit

Revisit this if reviewers start asking for narrative summaries, if the planner starts emitting data that cannot be rendered mechanically, or if the hosting API changes in a way that breaks the edge step separation. Until then, keep pr-composer thin and deterministic, and push new logic upstream to the component that owns the data.
