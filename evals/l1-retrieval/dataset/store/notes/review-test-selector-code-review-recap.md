---
id: 01K5A2M9ZMDXTY7YJJ8J7K7DKA
created: 2025-09-16T17:14-03:00
---

# Test selector code review

Quick pass over the test-selector after the last round of changes. Not a full audit, just what stood out while reading. The component takes the files touched by a dependency upgrade and decides which tests are worth running before the PR is marked as verified.

## What it does

test-selector maps the changed manifest and lockfile entries to the modules that import the upgraded package, then walks the import graph a few levels out and picks the test files that cover those modules. The mapping is cached in SQLite so repeat runs on the same repo don't rebuild it from scratch. The output is a list of test targets handed to the GitHub Actions job, which runs them inside the Docker image.

## Things that looked fine

The graph walk stops at the configured depth and does not loop on cycles. The cache key includes the lockfile hash, so a stale mapping is not reused after an upgrade. Error handling around missing test files is reasonable: it logs and skips instead of failing the whole selection.

## Concerns

The fallback when nothing matches is the weak spot. Right now it picks a small default set, and I'm not sure that is enough for packages that are only used through dynamic imports or config files. Those get missed by the static walk. Either the fallback should widen to the full suite for that package, or we need a way to flag such packages by hand.

Monorepos are the other worry. Workspace packages that depend on each other through local links are treated as ordinary imports, but the changed-file detection only looks at the root manifest in some paths. A bump inside a nested workspace may select too little.

## Performance

Building the graph on a large repo is slow the first time. The cache helps after that, but the cold run is what platform engineers will notice when onboarding a repo. Worth checking whether the walk can be done lazily from the changed modules instead of indexing everything. Also the SQLite writes happen one row at a time in a couple of places; batching them in a transaction would help.

## Tests for the selector itself

Coverage is thin on the edge cases above. There are fixtures for simple single-package repos but nothing for dynamic imports, nested workspaces, or a lockfile that changed without any manifest change. I'd add those before touching the fallback logic, so the behavior change is visible.

## Open questions

- Should the depth limit be per repo, or stay a global setting?
- Who decides when a package is declared as always needing the full suite?
- Does the cache need eviction, or is the usual repo count small enough that it doesn't matter?

## Next steps

Add the missing fixtures, then rework the fallback, then look at lazy graph building. Batch the cache writes whenever someone is in that file anyway.
