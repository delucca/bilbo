---
id: 01M0T6GCV3X8K228P3C4GT4Z3V
created: 2026-08-24T12:33-03:00
---

# run-ledger spec

This note specifies the run-ledger: the SQLite-backed record PatchPilot keeps of every verification run it starts for a dependency upgrade pull request. It is written quickly and covers what a later session needs to change the ledger without breaking its readers. The one hard rule pinned down here is the allowed set of values for the status column. It is enforced by the database itself, not only by application code: `CHECK (status IN ('pending','passed','failed','skipped'))`.

If you touch anything that writes a status, read the status section below first. Most of the rest is general guidance on how the ledger is meant to behave. Check the code for current column names and file locations before relying on this note for them, because the note deliberately does not repeat them.

## Purpose

PatchPilot opens upgrade pull requests across many repositories and then runs targeted tests to decide whether each upgrade is safe. Platform engineers who maintain those repositories need to answer a few plain questions afterwards. Did the tests run for this pull request? Did they pass? If nothing ran, why not? The run-ledger is the single place those answers live.

The ledger is a history, not a queue. It records what happened and what is happening. It does not decide what to run next. Scheduling and test selection belong to other parts of the system, which read the ledger but should not depend on its internal layout beyond what this spec states.

The ledger is also what the GitHub Actions workflows consult when they need to know whether a pull request has already been verified, so that a rerun of a workflow does not duplicate work or hide an earlier failure.

## Storage

The run-ledger lives in a SQLite database. SQLite was chosen because PatchPilot runs as a single Node.js process per deployment, often inside a Docker container, and a file-based database avoids another service to operate. Keep it that way: do not add features that assume a networked database or several concurrent writer processes.

Because the container filesystem is usually ephemeral, the database file must sit on a mounted volume when the ledger needs to outlive a container restart. If the volume is missing, the ledger starts empty and past runs are lost. This is a deployment concern, not a bug in the ledger, but it explains reports of history disappearing after a redeploy.

All access goes through one thin TypeScript module. Other code should not open the database file itself or build its own queries against the table. Keeping access in one place is what makes the status rule below enforceable in practice.

## Status values

The status column of the run-ledger table is restricted by a CHECK constraint. The exact constraint text is:

```sql
CHECK (status IN ('pending','passed','failed','skipped'))
```

That gives four allowed values and no others. An insert or update that sets any other value, including a different casing, is rejected by SQLite with a constraint error. Do not catch and swallow that error. It means a caller is wrong, and hiding it would put a run in a state nobody can interpret.

The four values mean the following.

- `pending`: the run has been recorded but has not reached a verdict. This covers both queued and in progress. There is deliberately no separate running value.
- `passed`: the targeted tests ran and all of them succeeded.
- `failed`: the targeted tests ran and at least one failed, or the run could not finish because of an error in the tests or the build.
- `skipped`: no tests were run for this upgrade on purpose, for example because nothing relevant was selected.

## Status transitions

A run starts as `pending`. It then moves once to one of the three final values: `passed`, `failed` or `skipped`. Final values are terminal. A run that ended as `failed` is never edited back to `pending`. If someone retries the verification, the retry is a new row, so the history of the earlier failure stays visible.

This append-style behavior matters for the people reading the ledger. A pull request with a failed run followed by a passed run should show both. Overwriting the first would make flaky tests look like they never existed.

A run can in principle be left as `pending` forever if the process dies mid-run. Handling that is a reconciliation job's responsibility: on startup it should find stale pending rows and settle them as `failed`, with a note about the interruption. Do not invent a fifth status for abandoned runs; if one is really needed, it is a schema change and needs a migration, covered below.

## Why a CHECK constraint

Status used to be a free-form string in an early prototype. Over time different call sites wrote slightly different spellings, and reports that grouped by status silently split a single outcome into several buckets. Moving the rule into the schema removes that whole class of problem, because the database refuses bad values no matter which code path wrote them.

The constraint also acts as documentation. Anyone reading the schema sees the full set of states without hunting through the TypeScript source. When the TypeScript type for status and the constraint disagree, the constraint wins, since it is what actually decides whether a write succeeds.

Keep the TypeScript union type for status in step with the constraint. A test should assert that the two lists match, so that a change to one without the other fails in CI rather than in production.

## Changing the allowed values

SQLite cannot alter an existing CHECK constraint in place. Changing the allowed set means creating a new table with the new constraint, copying the rows across, dropping the old table and renaming the new one, all inside a single transaction. Write that as a numbered migration and test it against a copy of a populated database, not only an empty one.

Before adding a value, check every reader. Dashboards, the GitHub Actions steps that gate merges, and any report that counts runs by status may all assume the current four values. A new value that a reader does not recognize can be mistaken for a failure or ignored altogether. List the readers in the pull request description and say how each handles the new value.

Removing a value is harder still, because old rows may still carry it. Migrate those rows to a suitable remaining value first, and record the reasoning in the migration itself.

## Reading the ledger

Readers should treat status as a closed set and handle each value explicitly. A switch over the four values with a default branch that throws is better than a default that quietly treats unknown input as success. If the constraint ever changes and a reader is missed, a loud failure is the better outcome.

When deciding whether a pull request is safe to merge, only `passed` counts as a green signal. `skipped` is not a pass. It means nothing was checked, and the merge policy should treat it as its own case, usually requiring a human to look. `pending` means wait, and `failed` means block.

For summaries across many repositories, report all four values separately. Merging `skipped` into `passed` makes a repository look healthier than it is, and platform engineers have asked specifically not to see that.

## Gotchas

- Case matters. The stored values are lowercase, and a capitalized value fails the constraint.
- Do not write a status of null or an empty string to mean unknown. Use `pending` until there is a verdict.
- Tests that create the table by hand must use the same constraint text as the real schema, or they will pass while production rejects the write. Prefer building the test database from the real migrations.
- A constraint error during a run usually points at a bug in the caller, not at the database. Look at which value was being written before touching the schema.
- Docker images that bake in an old database file can carry an outdated constraint. After a migration, confirm the deployed database actually has the new schema.

## Open questions

- Whether interrupted runs deserve their own status instead of being settled as `failed`. The current answer is no, to keep the set small, but reconciliation notes may not be enough for reporting.
- Whether `skipped` should carry a reason that readers can filter on, so reports can tell an intentional skip from a selection gap.
- How long ledger history should be kept, and whether old rows should be archived out of the main database once it grows large.
