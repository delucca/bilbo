---
id: 01KB1YPK40RF5VWSVK24W0H0TV
created: 2025-11-27T03:06-03:00
sources:
  - "code: migrations/002_run_ledger.sql"
---

# run-ledger design

The run-ledger is the SQLite table where PatchPilot records every dependency upgrade attempt and how it ended. One row is one attempt to upgrade one package in one repository. This note covers the table shape, why it is that shape, and how the rest of PatchPilot is expected to use it. It is written quickly; check the code before relying on fine detail.

## Purpose

PatchPilot opens upgrade pull requests across many repositories and runs targeted tests on each. Platform engineers need to answer simple questions afterwards: what did we try, what passed, what is still open, and what keeps failing. The run-ledger is the single place those answers come from. It is not a log of test output; it only holds the summary of each attempt.

## Table shape

The run-ledger table has the primary key `run_id`. The other columns are repo_id, package, from_version, to_version, status and `finished_at`. That is the whole list; there is no separate column for test output or PR text. Anything bigger than a summary lives elsewhere and is linked by `run_id`.

## Primary key

`run_id` identifies one attempt. It is the only key other code should use to refer to a run. Do not build a composite key out of repo, package and versions, because the same upgrade can be retried and each retry is its own run with its own `run_id`.

## Columns about the target

repo_id says which repository the upgrade was for. package is the dependency name. from_version is the version the repository had before the upgrade and to_version is the version PatchPilot tried to move to. Together they describe the change without needing to open the pull request.

## Status

status is the current state of the run. It is written when the run starts and updated as the run moves along, so a reader can tell running attempts from finished ones. Keep the set of values small and fixed. Adding a new value means checking every query and dashboard that filters on it.

## Finished time

`finished_at` is set when a run reaches a final state. While a run is still going it stays empty. That makes it the easy way to find unfinished work: a row without `finished_at` is either in progress or was abandoned by a crash. Do not fill it for in-progress rows to make a query simpler.

## Writers

The orchestrator that creates the upgrade pull request inserts the row and sets the initial status. The test runner step, which runs in GitHub Actions or in a Docker container depending on the setup, reports back and the orchestrator updates status and `finished_at`. Only the orchestrator writes to the table. Runner containers should not open the SQLite file directly.

## Readers

Reports, retry logic and any status command read the table. Retry logic looks at earlier rows for the same repo_id, package and to_version to decide whether another attempt is worth making. Reports group by status and by repo_id. Readers should open the database read-only where they can.

## Why SQLite

One file, no server, easy to ship in the Docker image and easy to copy for debugging. Write volume is low: a few rows per upgrade attempt. If the tool ever needs several orchestrators writing at once, revisit this choice before adding workarounds.

## Gotchas

Retries produce several rows for the same upgrade, so counting rows overcounts upgrades. Count distinct combinations or take the latest run per combination. Crashed orchestrators leave rows with no `finished_at` and a non-final status; these need a cleanup pass, not manual edits scattered around. Version strings are stored as given by the package registry, so compare them with a proper version parser, not as plain text.

## Open questions

Whether to add an index on repo_id and package once the table grows. Whether to keep a reason for failure as a short column or leave it in the linked output. Whether old rows should be pruned or kept forever; for now they are kept.

## Change rules

Schema changes go through a migration that runs at startup. Do not rename `run_id` or `finished_at` without updating every reader, since reports and retry logic use them by name. Keep this note in sync when the column list changes.
