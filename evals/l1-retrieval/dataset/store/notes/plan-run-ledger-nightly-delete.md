---
id: 01KG4RYVFN31REDMFGCKBXYY4P
created: 2026-01-29T08:42-03:00
---

# run-ledger retention plan

The plan is to stop run-ledger from growing forever. A nightly job will delete run-ledger rows older than 180 days by running `npm run ledger:prune`. Everything below is about getting to that point safely: what the job touches, what has to be true before it is switched on, how we roll it out, and how we know it worked. The related note on how the sandbox images are laid out is [[sandbox-image-structure-overview]]. Read it if you need to know where the ledger database file ends up inside a container, because that matters for the scheduling choice further down.

The short version for someone in a hurry: the retention window is `180 days`, the command is `npm run ledger:prune`, it runs once a night from a scheduled GitHub Actions workflow, and it must be safe to run twice, safe to interrupt, and must never delete anything a still-open pull request depends on. The rest of this note explains why and lists the work in order.

## Why the ledger needs pruning

run-ledger is the SQLite-backed record of what PatchPilot did. Every time PatchPilot opens or updates a dependency upgrade pull request, and every time it runs the targeted tests that verify that upgrade, a row goes into run-ledger. The rows carry the repository, the dependency and version range involved, the outcome of the targeted test selection, timing, and a pointer to the log output. Platform engineers who maintain many repositories use it to answer questions like: why did this upgrade pull request get opened, which tests were chosen, did they pass, and how long did it take. It is also what the scheduler reads to decide whether a dependency was already tried recently and failed, so it does not retry the same broken upgrade every day.

The problem is simple. Nothing ever removes rows. The number of managed repositories has grown, each repository can have several dependency upgrades in flight, and each upgrade produces more than one row because retries and re-verification after a rebase each write their own entries. The database file is getting larger, and the symptoms are the ones you expect from SQLite when a single file keeps growing:

- Backups and the copy of the database that gets baked or mounted into the Docker sandbox take longer than they used to.
- The queries that power the dashboard and the history view for a repository scan more rows than they need to, because most of the old rows are irrelevant to anyone.
- Vacuuming and integrity checks, when someone runs them by hand, are slow enough that people avoid doing them.
- Cold starts of a job that opens the ledger are slower, mostly because of page cache misses on a big file.

None of this is an outage. It is a slow tax, and it will get worse. The cheapest fix that does not change the data model is a retention window with a scheduled delete. We picked `180 days` because it covers a couple of release cycles for most of the dependencies we track, it is long enough that someone investigating a regression from last quarter still finds the history, and it is short enough to keep the file bounded. If the number turns out to be wrong, it should be a single setting that can be changed without code edits; see the configuration notes below.

What the ledger is not: it is not the audit log of record for compliance, and it is not the only place the pull request history lives. The pull requests themselves on the hosting side keep their own history and comments. So deleting old ledger rows loses PatchPilot's own bookkeeping, not the canonical record of what was merged. That is the argument that makes plain deletion acceptable instead of an archive-then-delete scheme. Revisit this if anyone starts depending on the ledger for something with a retention obligation.

## What the prune job does

The job runs `npm run ledger:prune`. That script is the only supported entry point for deleting ledger rows by age. Nobody should write ad hoc delete statements against the database in production; if a one-off cleanup is needed, extend the script so the behaviour is reviewed and repeatable.

Behaviour the script has to have, stated as requirements so the implementation can be checked against them:

- It deletes rows whose recorded time is older than `180 days` relative to the time the script starts. The cutoff is computed once at the start and used for the whole run, so a long run does not drift and delete rows that were inside the window when it began.
- It never deletes a row that belongs to a run which is still considered active. A run is active while its pull request is open or while a verification is queued or in progress. Age alone is not enough to qualify a row for deletion. In practice an active run will almost never be older than the window, but a stuck run or a pull request that sat open for a very long time can be, and deleting its rows would make the scheduler forget it and open a duplicate.
- It deletes in bounded batches rather than one giant statement. SQLite takes a write lock for the duration of a transaction, and PatchPilot workers write to the ledger while they run. A single huge delete would block them. Each batch is its own short transaction, and the script yields between batches so writers get a turn.
- It is idempotent. Running it twice in a row, or running it again after an interruption, produces the same final state as running it once. This falls out of using an age cutoff, but only if we do not keep any separate progress marker that could get out of sync.
- It is safe to interrupt. Because each batch commits independently, killing the process leaves the database consistent, just with some old rows still present. The next night's run picks up the rest.
- It reports what it did: how many rows it removed, how many it skipped because they belonged to active runs, how many batches it used, and how long it took. These go to standard output in a plain format that the workflow log keeps. The skipped-because-active count matters most, since a sudden jump there means something upstream is stuck.
- It has a dry-run mode that counts and reports but deletes nothing. The dry-run is what we use for the first rollout stage and whenever someone wants to see the effect of changing the window.
- It exits with a nonzero status on any failure to open the database, failure to acquire a write transaction after a reasonable wait, or any error mid-batch. It exits zero when there was simply nothing to delete.

Space reclamation is a separate question from row deletion. Deleting rows in SQLite marks pages as free but does not shrink the file. Freed pages are reused by later inserts, so the file stops growing once the steady state is reached, which is the real goal. Shrinking the file needs a vacuum, which rewrites the whole database and needs a lock for its duration plus temporary disk space. Plan: do not vacuum as part of the nightly job. Make it a separate, deliberate maintenance step that someone runs in a quiet window the first time after the initial large prune, and decide afterwards whether it is needed again. Incremental auto-vacuum is an option if the database is created with it, but turning it on for an existing database requires a full vacuum anyway, so it is not a quick win. Keep this out of the first version.

Indexes matter for the prune. The delete selects by time, so there needs to be an index that supports that selection, otherwise every batch scans the table and the job becomes the slow thing it was meant to prevent. Check what indexes the run-ledger table already has before writing the script. If the time column is not indexed, adding the index is a migration and goes through the same migration path as other schema changes. Do that first, and let it run before the first real prune, because building an index on a big table is itself a write-heavy operation.

Related rows. If run-ledger has child tables, such as per-test results or log pointers, the delete must remove them together with the parent in the same transaction, or the schema must cascade. Orphaned child rows are worse than old parent rows, because they are invisible to the history view and will never be cleaned up by an age rule on the parent. Verify the foreign key setting on the connection, since SQLite does not enforce foreign keys unless asked to per connection. If cascade is not in place, the script deletes children explicitly first. Add a test that creates a parent with children, ages it past the window, prunes, and asserts nothing is left behind.

Log files. If the ledger rows point to log output stored outside the database, deleting the rows leaves the log files behind. Decide whether the script should also delete those, or whether a separate sweep handles it. Our leaning is that the same script handles it, after the row delete commits, and that a missing file is not an error. The reasoning: rows first means a crash between the two steps leaves orphan files, which are harmless and can be swept up next night by comparing against what the ledger still references; files first would leave rows pointing nowhere, which breaks the history view. If this turns out to be complicated, ship the row delete alone and write the file sweep as a follow-up, noting the gap clearly.

## Scheduling and where it runs

The job is nightly. The natural home is a scheduled GitHub Actions workflow, since that is what the rest of PatchPilot's automation already uses and platform engineers know where to look for its logs. The workflow checks out the project, installs dependencies, obtains the ledger database, and runs `npm run ledger:prune`.

The awkward part is the word obtains. A scheduled runner is a fresh machine. The ledger database has to be reachable from it, and there are a few ways that can be true, with different consequences:

- If the live ledger is a file on a long-lived host, the job needs to run on that host, for example through a self-hosted runner, or it needs to reach the file through a mounted volume. Pruning a copy does nothing useful.
- If the live ledger lives inside a Docker volume used by the PatchPilot service containers, the job should run as a container attached to the same volume, using the same image the service uses, so the script and the schema version match. Check [[sandbox-image-structure-overview]] again only if you need the data directory layout; the point here is that the mount has to be right.
- If the ledger is shipped to and from an artifact store around each run, then pruning must happen inside the same window as the write-back, and two writers racing is a real danger. That would need a locking story before anything else.

Pick whichever of these matches how production is actually deployed today, and write the answer into this note once confirmed. Do not guess; ask whoever operates the deployment. The one hard rule is that there must be exactly one prune running at a time against a given database. Use the workflow concurrency setting so a slow run is not overlapped by the next night's run, and make the script take an advisory lock of its own, such as a lock row or lock file next to the database, and exit cleanly with a clear message if another prune holds it. Two layers because the workflow-level guard does not help if someone starts the script by hand.

Time of day. Choose a quiet period for PatchPilot's own work. The dependency upgrade scans and test runs are the main writers, so the prune should not collide with the bulk of them. If scans are spread across the day because repositories are in different regions, there may be no truly quiet time, and the batching and yielding in the script are what protect us, not the clock. Pick the least busy hour from the existing run timing data in the ledger itself, which is a nice use of the thing we are about to prune.

Failure notification. A scheduled workflow that fails silently is how this kind of job rots. Make a failed run visible: the workflow should fail loudly so the usual notification goes to the platform team's channel, and it should not retry blindly, because a retry that overlaps a half-finished first attempt is exactly the concurrency case above. A single retry after the lock is confirmed free is acceptable if we want it, but start without one.

Permissions. The workflow needs only what it needs to reach the database and to run the script. Do not give it write access to repositories or the ability to open pull requests. It is a maintenance job with a narrow job.

Configuration. The retention window should be read from configuration with a default equal to `180 days`, so the number lives in one place and the workflow, the dry-run, and tests all agree. Do not hard-code it in the workflow file as well as the script. If we allow overriding it by environment variable for experiments, the override should be logged in the first line of the output, so that a run with an unusual window is obvious in the log afterwards. Guard against a nonsense value: a zero, negative or missing window must make the script refuse to run rather than delete everything. This guard is the single most important safety check in the whole plan, and it deserves its own test.

## Rollout, verification, and open questions

Order of work. Do these in sequence, each as its own change so it can be reviewed and reverted separately.

- First, inspect the schema and the indexes of run-ledger and the child tables, and write down the answer to the foreign key and cascade question. Add the time index if missing.
- Second, write the script behind `npm run ledger:prune` with the dry-run mode only, plus the configuration, the window guard, the active-run exclusion, and the output format. Unit tests use an in-memory or temporary SQLite database seeded with rows on both sides of the cutoff, with active and finished runs, with children, and with an empty table.
- Third, add the real delete in batches, with the lock, and extend the tests: interruption between batches, a second run being a no-op, a concurrent writer inserting during the prune, and the guard refusing a bad window.
- Fourth, add the workflow in dry-run mode on the schedule. Let it run for several nights. Read the counts. Compare them with a hand query of how many rows are older than the window, to confirm the script and the query agree. Look at the skipped-because-active number and make sure it is small and explainable.
- Fifth, take a backup of the database immediately before the first real prune, and keep it until the result has been checked for a reasonable period. The first real run will be by far the largest, since it removes everything that has accumulated since the beginning. Consider running that first one by hand, watched, in a quiet window, rather than waiting for the schedule, and consider doing it in stages by using a longer window first and stepping down to `180 days`. Stepping down is more cautious and costs only a couple of extra manual runs.
- Sixth, switch the scheduled workflow from dry-run to real. Keep the dry-run option available.
- Seventh, decide about the vacuum step after seeing the file size and the free page count following the first big prune. Write that decision back here.

Checks after the first real run. The database passes its integrity check. The row count dropped by about the number the dry-run predicted. The scheduler still behaves: it does not reopen upgrade pull requests that are already open, and it still remembers recent failures, which are the two behaviours that depend on the rows we must keep. The dashboard and the history view for a few repositories still show recent runs correctly and no longer show anything older than the window. Workers that were writing during the prune did not report lock errors beyond what their normal retry handles. The workflow log contains the summary counts.

What could go wrong, and the response.

- The window is mis-set and too much is deleted. Response: the guard against nonsense values, the config default, the backup before the first real run, and the dry-run history as evidence of the expected counts. Restoring from backup loses whatever was written since, so the window of exposure should be short; that is another reason to do the first prune by hand and watch it.
- Active runs lose their rows. Response: the exclusion rule and its test; the skipped count in the output as an early warning.
- Writers are starved or time out. Response: small batches and yielding; if it still happens, shrink the batch or lengthen the pause, both configuration values rather than code changes. Watch for busy errors in worker logs the first few nights.
- Two prunes overlap. Response: workflow concurrency plus the script's own lock.
- The file never shrinks and someone assumes the job is broken. Response: say clearly in the docs for the script and in this note that the file size is expected to plateau rather than fall, and that free pages are reused.
- The job runs against a copy and nothing real changes. Response: the first stage of rollout confirms, by comparing counts with the live database, that the script is looking at the right file.
- The retention window conflicts with something that quietly reads old rows, such as a report, an export, or a metric that computes long-term averages. Response: search the codebase for readers of the ledger before the real switch-on, and ask the platform engineers whether anyone keeps a long-range chart that depends on rows older than the window. If so, either materialize the aggregates into a small summary table before pruning, or lengthen the window.

Documentation. Add a short section to the operations docs that says what the job deletes, what it keeps, how to run the dry-run, how to run the script by hand, where the logs are, and what to do if it fails. Link the script from the workflow file with a comment. Mention in the contributor docs that anything new written to run-ledger needs a decision about how it is pruned, so new tables do not quietly escape the retention rule.

Open questions, to be answered before the real switch-on and written back into this note.

- Where does the live run-ledger database physically live in production, and how will the scheduled job reach it. This decides runner type, image, and mount.
- Does the schema cascade deletes to child tables, and is foreign key enforcement on for the script's connection.
- Is the time column indexed, and which time is authoritative for a row: when the run started, when it finished, or when the row was written. The cutoff should use the one that best represents when the run stopped mattering, which is probably its finish time, falling back to the start time for runs that never finished. Be explicit in the script about which one it uses.
- Are there log files or other artifacts referenced by rows, and does the script remove them or does a separate sweep.
- Does anyone consume rows older than the window, and do they need a summary kept.
- Is the chosen retention of `180 days` acceptable to the people who investigate old regressions. Ask once, early, rather than discovering it after the first big delete.
- Who gets the failure notification, and who owns the job after it ships.

Done means: the nightly workflow runs `npm run ledger:prune` for real, the summary shows a small steady number of deleted rows each night, the file size has stopped growing, the scheduler behaviour is unchanged, and the operations docs describe it. Until the open questions above are answered, keep the workflow in dry-run mode and do not run a real prune against production data.
