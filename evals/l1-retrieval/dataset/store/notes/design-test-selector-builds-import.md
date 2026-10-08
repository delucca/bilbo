---
id: 01KY1BYXBCH109RYS4P5ZBC55A
created: 2026-07-21T00:36-03:00
sources:
  - "code: src/selector/graph.ts"
---

# test-selector design

The test-selector decides which tests PatchPilot runs for a dependency upgrade pull request. It builds an import graph with `ts-morph` and maps each changed file to the test files that transitively import it. Those test files, and only those, are handed to the runner in the GitHub Actions job. The goal is to keep verification fast on repositories where a full test run is too slow to do for every upgrade PR.

The earlier exploration that led to this shape is in [[test-selector-investigation-historical]]. This note covers the design as it stands.

## Inputs and outputs

Input is the list of files changed by the upgrade PR. For a typical upgrade that is the lockfile, the package manifest, and any source files the bot had to touch to keep things compiling. Output is a list of test files, plus a reason for each one: which changed file pulled it in and through which chain of imports.

The reason matters. When a selected test fails, the engineer reading the PR comment should see why that test was picked, not just that it failed. The reasons are stored in SQLite next to the run record, so a later run can compare what was selected before.

## How the graph is built

The selector loads the project through `ts-morph`, using the repository's own tsconfig so path aliases and project references resolve the same way the compiler resolves them. Each source file becomes a node. Each import, re-export and dynamic import with a literal specifier becomes an edge from the importing file to the imported one.

Mapping a changed file to tests is a reverse walk. Start at the changed file, follow edges backwards, and collect every file on the way. Any collected file that matches the repository's test file pattern is selected. The walk tracks visited nodes, so cycles do not loop.

Barrel files are the main cost. A barrel that re-exports everything makes any change under it reach nearly every test. We accept that and do not special-case barrels, because skipping them would silently drop tests that really do depend on the change.

## Dependency changes

A changed lockfile or package manifest is not a source file, so the import graph cannot map it directly. The selector first works out which packages changed version. It then finds every source file that imports one of those packages, by module specifier, and treats those files as changed. From there the normal reverse walk applies.

If a package changed but nothing in the repository imports it, the selector selects nothing from that package. That is correct for pure transitive bumps, but the pipeline still runs the type check and build, so a break there is still caught.

## Fallbacks

The selector prefers running too much over running too little. It falls back to the full test suite when any of these hold:

- The tsconfig cannot be loaded or the graph build throws.
- A changed file is outside the graph, such as a generated file or a config file the graph never sees.
- The changed set touches the test runner config or shared test setup.
- A dynamic import has a non-literal specifier in a file on the reverse path.

Each fallback is logged with its cause. Platform engineers who maintain many repositories use those logs to find repos where the fallback fires too often and the graph needs attention.

## Known gaps

- Tests that reach code only through runtime lookups, such as plugin loading by name, are invisible to static imports. The selector does not catch these.
- Non-TypeScript files read at runtime, like JSON fixtures or SQL files, are not edges in the graph. A change to one falls under the outside-the-graph fallback.
- Docker-based integration tests run in separate containers and are not selected by the graph. They are scheduled by a separate rule, not by this component.
- Graph building cost grows with repository size. On large repos the load step dominates the selector's runtime, well above the walk itself.

## Things to keep in mind when changing it

Keep the selection deterministic. The same changed set on the same commit must give the same list in the same order, because the stored reasons are compared across runs.

Do not add heuristics that drop tests based on past pass rates. The selector's value is that its choices can be explained by imports alone, and that property is what makes a green run trustworthy.

Add a test for every new fallback, and make the log cause string distinct, so the per-repo reports stay readable.
