---
id: 01K1EB243EYKEYPKVYJQJ7DCXV
created: 2025-07-30T15:56-03:00
---

# Run-ledger index on repo and finish time

We decided that the run-ledger table carries one composite index, `idx_run_ledger_repo_time`, covering repo_id and finished_at. The reason is the access pattern: the dashboard always asks for the latest runs of one repository. Nothing else reads run-ledger in a way that needs its own index right now, so we kept it to this one.

## Decision

Keep `idx_run_ledger_repo_time` on run-ledger, with repo_id first and finished_at second. Do not add further indexes to run-ledger unless a new, repeated query shows up that this one cannot serve. If someone proposes dropping it, check the dashboard queries first; they are the reason it exists.

The column order matters. repo_id leads because every dashboard query filters on a single repository. finished_at follows so SQLite can walk the entries for that repository in time order and stop after the first few rows, instead of sorting everything that repository ever ran.

## Why this index

PatchPilot opens upgrade pull requests across many repositories and runs targeted tests for each. Platform engineers who maintain many repositories open the dashboard to see what happened lately in one repo: which upgrades ran, which passed, which failed. That view is always scoped to one repository and always wants the newest runs first.

Without the index, SQLite would scan the whole run-ledger table for every dashboard load. The table only grows, since each verification run adds a row, and a busy fleet of repositories makes it large fast. A full scan plus a sort would get slower every week, and the dashboard is the part people actually watch.

With `idx_run_ledger_repo_time` the lookup is a range read on one repository, already ordered by finished_at. Reading it descending gives the latest runs directly, and a limit on the query keeps the work small no matter how many old runs exist.

## What it does not cover

- Queries across all repositories at once, for example a fleet-wide list of recent failures. The index leads with repo_id, so those cannot use the time ordering and would scan. We have no such query in the dashboard today. If one is added, consider a separate index rather than reordering this one.
- Runs that have not finished. A run with no finished_at yet sorts apart from finished runs in the index. Queries for in-progress work should be written with that in mind and not assume the index orders them.
- Lookups by anything other than repository and time, such as by upgrade target or by pull request. Those would need their own thinking.

## Costs

An index slows writes a little and takes disk space. For run-ledger the writes are one row per run, which is rare next to the reads, and SQLite handles that easily. The extra file size is small next to the data itself. We accepted both costs because dashboard latency is what users notice.

Because the database is SQLite inside the Docker image or its mounted volume, there is no separate database server to tune. The index lives in the same file as the table, so it travels with backups and copies of that file with no extra step.

## Things to remember

- If a migration rebuilds the run-ledger table, recreate `idx_run_ledger_repo_time` in the same migration. Rebuilding a SQLite table by copy and rename drops its indexes, and the dashboard will quietly get slow instead of failing.
- If finished_at changes type or meaning, for example from a stored timestamp to something else, review the index and the dashboard ordering together.
- When checking a slow dashboard, ask SQLite for the query plan and confirm it names `idx_run_ledger_repo_time`. If it does not, the query shape probably drifted away from repo plus time.
- Tests that seed run-ledger with many rows for several repositories are the best way to catch a lost index; a plan check there is cheap.

## Alternatives considered

We considered an index on finished_at alone. It would help a fleet-wide recent list but forces a filter on repo_id after the fact, so a single-repository view would still read many unrelated rows. We also considered no index and relying on small table size. That holds early on and fails later, and adding an index to a large live table is more disruptive than creating it from the start.

A separate index on repo_id alone was rejected as redundant, since the composite index already serves any lookup by repo_id as its leading column.
