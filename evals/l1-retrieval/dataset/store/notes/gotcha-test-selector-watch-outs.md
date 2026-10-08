---
id: 01K3CYFKX4SPPMR4R59EWMNT5K
created: 2025-08-23T23:28-03:00
---

# test-selector gotchas

Things that have bitten people when touching test-selector. Read before changing it. It decides which tests run for an upgrade pull request, so a quiet mistake here means a bad upgrade gets a green check.

## Why it matters
If test-selector picks too few tests, a broken dependency bump merges. If it picks too many, CI gets slow and platform engineers stop trusting it.

## Fail toward running more
When the mapping is unsure, run more tests, not fewer. Never let an error path return an empty selection that looks like "nothing affected".

## Empty selection
An empty result needs a different meaning from "no tests found". Keep those two cases apart in code and in logs.

## Dependency graph input
The selector depends on how the changed package is mapped to source files. Lockfile changes can move transitive packages too, so do not only look at the direct dependency.

## Transitive dependencies
A bump of one package can change what another package resolves to. Check that the selector follows these links and does not stop at the manifest diff.

## Monorepos
Many repositories are monorepos. Workspace boundaries matter. A change in one workspace can affect another through a shared package.

## Path handling
Normalize paths before comparing. Separators, relative segments and symlinks have all caused missed matches.

## Caching in SQLite
Cached results live in SQLite. Stale rows can hide a real change. When you change selection logic, invalidate old rows or key them on the logic as well as the input.

## Schema changes
Migrations on the SQLite store must work on existing databases, not only fresh ones. Test with a database that already has data.

## GitHub Actions context
Behavior differs between a local run and a run inside GitHub Actions. Shallow checkouts can hide the base commit, which breaks diff-based selection.

## Docker runs
Inside Docker the working directory and file ownership can differ. Do not assume the repo root equals the current directory.

## Flaky tests
Do not drop a test from selection because it is flaky. Handle flakiness elsewhere, so the selector stays a pure picker.

## Determinism
The same input should give the same selection in the same order. Sort before output, or diffs between runs become noise.

## Logging
Log why each test was chosen. Without reasons, nobody can debug a missed test after the fact.

## Tests for the selector
Add a case for each bug fixed. Include a fixture where the right answer is "run everything".

## Quick local check
Run the selector against a known upgrade and compare with the full suite:

```bash
npm test
```

## Before merging a change
Compare selections before and after on a few real past pull requests. Any test that disappeared from the list needs an explanation.
