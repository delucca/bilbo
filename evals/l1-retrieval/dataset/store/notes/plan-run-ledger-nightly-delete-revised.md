---
id: 01KW8M00ZJCRY54VN6D22RBCFJ
created: 2026-06-28T23:40-03:00
---

# run-ledger nightly pruning plan

This note replaces the earlier note "run ledger nightly delete". The new value is 90 days: the nightly pruning job will delete run-ledger rows older than 90 days, and the earlier plan of keeping 180 days is dropped. Anything that still says 180 days in a doc, a comment or a ticket is out of date and should be corrected when you run into it.

The rest of this note is the plan for getting there: what changes, in what order, what to check, and what could go wrong. It is written quickly and is meant to be edited as the work moves.

## Decision

The retention window for the run-ledger is 90 days. The pruning job runs nightly. Each night it removes every row whose age is past that window and leaves everything else alone. The window is the only retention rule. There is no per-repository override, no per-user override and no "keep failed runs longer" exception in this plan. If someone wants one of those, it is a separate change and needs its own note.

The earlier plan was to keep 180 days. That plan was never switched on in production as the pruning behaviour, so there is no already-deleted data that was removed under the longer window and no migration back. The change is a change of intent and of the configured value, not a reversal of something that already ran. Check this once more before rollout by looking at what the job did on the first dry pass, because a note like this can be wrong about what actually happened on a given machine.

The value should live in one place. Do not copy it into the workflow file, the Docker image and the TypeScript code separately. Pick one source, read it everywhere else, and make the default match the decision here. A short sketch of what the configured value should say:

```
retention: 90 days
```

That is the only form of the value this note commits to. How it is spelled in the real config (key name, unit handling, environment variable or file) is whatever the existing config code already does for similar settings. Follow that, do not invent a new style.

## Why the shorter window

PatchPilot opens dependency upgrade pull requests and runs targeted tests to verify them. The run-ledger is the record of those runs: which upgrade was attempted, which tests were chosen, what the outcome was, and enough detail to explain a result later. Platform engineers who maintain many repositories read it when they ask why a pull request was opened, why it was held back, or why a test selection looked too narrow.

Those questions are almost always asked soon after the run. A pull request that was opened, reviewed, merged or closed within a few weeks does not need its ledger rows after that. The longer window mostly kept rows that nobody opened. Meanwhile the rows are not free. The run-ledger sits in SQLite, and the file grows with every run across every repository PatchPilot watches. A bigger file means slower backups, slower startup when the file is copied into a Docker container, and slower queries that scan by time. The shorter window keeps the file small enough that those costs stay boring.

There is also a plain tidiness argument. Old rows describe dependency versions and test selections that no longer match the repositories. Reading them later is more likely to mislead than to help. If a platform engineer needs a long history of a particular upgrade, the pull request itself on the hosting side is the durable record, not the ledger.

The tradeoff is accepted: after the window passes, you can no longer ask the run-ledger what happened on a given run. If that turns out to hurt, the answer is to raise the window deliberately and say so in a note, not to quietly stop the job.

## What the pruning job does

The job is a nightly scheduled run. It is driven from GitHub Actions on a schedule, and it works against the SQLite file that holds the run-ledger. The intended behaviour, in order:

First, it works out the cutoff: now minus the window. The window is the configured value, which is 90 days. Use one clock source for "now" and use it for the whole run, so a job that crosses midnight does not apply two different cutoffs.

Second, it counts the rows that are older than the cutoff and logs the count before it deletes anything. This count is the main thing a person looks at the next morning. A count that is wildly bigger than the previous nights is a signal to stop and look, not to carry on.

Third, it deletes those rows. It should do so in a transaction, and in batches if the number is large, so that the database is not locked for a long stretch while PatchPilot is also trying to write new rows for runs in progress. Writers must not be starved. If a batch fails, roll back that batch, log the error, and stop. Do not retry in a tight loop.

Fourth, it logs the number actually deleted, and the number remaining. A mismatch between counted and deleted is worth a warning, because it may only mean new old rows appeared, but it may also mean a partial failure.

Fifth, it reclaims space only if that is cheap. SQLite does not shrink the file on delete; freed pages are reused by later writes. Do not add a full compaction step to the nightly job. If the file size becomes a problem, plan a separate, occasional maintenance step and keep it out of this job.

The job must be safe to run twice in a row. Running it again right after a successful run should delete nothing and report zero. It must also be safe to skip a night: the next run just deletes more, because the cutoff is computed from the clock and not from the last run.

## Rollout order

Do this in small steps, so each one can be checked on its own.

Step one is to make the window a configured value and set it to 90 days, with the code reading it from that single place. Nothing is deleted at this stage. Merge it, and confirm that the value shows up where the job will read it.

Step two is a dry mode. The job computes the cutoff and logs how many rows it would delete, then exits without deleting. Run it on a schedule for a few nights. Compare the counts against what you expect from the rate of runs. The first night will be the largest, because it covers everything that has piled up past the window; later nights should be roughly the same size as one night of new runs.

Step three is to turn deletion on in a non-critical environment first, if there is one, and look at the file before and after. Make sure the remaining rows are the recent ones and that a recent run can still be looked up with all its detail.

Step four is to turn deletion on for real. Watch the first night closely. Because the first real pass is the biggest, run it at a quiet hour and keep the batching on so the writers are not blocked.

Step five is clean-up: remove any leftover mention of the old 180 days plan, in comments, docs, workflow descriptions and tickets. Link this note from the ticket and close the old note's thread by pointing at this one.

Before step four, take a backup copy of the SQLite file and keep it somewhere that is not pruned. The first real pass is the one moment where a mistake in the cutoff logic would remove a lot of rows at once, and a backup is cheap compared with explaining that. Delete the backup after the first week of clean nightly runs, once the counts look normal.

## Checks and failure modes

Things to check on the code side:

The cutoff direction. It is easy to flip the comparison and delete the recent rows instead of the old ones. Write a test with a few rows at different ages, run the job against an in-memory SQLite database, and assert that only the old ones are gone. Include a row just inside the window and a row just outside it, so the boundary is covered.

The time column. Make sure the age is computed from the column that records when the run happened, in a consistent time zone. If some rows store local time and others store UTC, the cutoff will be off by hours at the edges. That is not a disaster for a window this long, but it should be known and not discovered by accident.

Rows that are still in progress. A run that started long ago and has not finished is unusual, but a stuck run should not be pruned out from under the process that is writing to it. Decide whether the job skips rows that are not finished. The safe choice is to skip them and let a person deal with stuck runs separately. Write that choice down in the code comment next to the delete.

Related tables. If the run-ledger rows have child rows elsewhere in the SQLite file, such as per-test results, the delete must take them too, or the file ends up with orphans. Check how the schema ties them together and whether a cascade is declared. If it is not declared, do the child delete explicitly in the same transaction. Do not rely on memory about this; read the schema.

Things to check on the operations side:

The scheduled workflow should fail loudly. If the job errors, the workflow run should be red, and someone should see it. A pruning job that fails silently for weeks is the same as having no pruning, and the file just grows again with nobody noticing.

The job needs to reach the SQLite file. If the file lives inside a Docker volume or is baked into a container, make sure the job runs against the same file the application writes to, and not against a copy. This is a classic way to have a green job that prunes nothing. The dry mode counts help here: if the count is zero for many nights while the file keeps growing, the job is looking at the wrong file.

Concurrency. Two copies of the job must not run at once, and the job must not run while a restore or a backup is in the middle of copying the file. Use the workflow's own concurrency control to keep a second copy from starting, and keep backups out of the same time slot.

What to do when the count looks wrong. If the number of rows to delete is far larger than expected, stop and read the clock and the config first. A wrong clock or a wrong unit in the configured value (for instance a value read as a shorter span than intended) is the most likely cause. Do not rerun with a larger batch to "get through it".

The related traps with the verification runs, the part of PatchPilot that produces most of these rows, are collected in [[verify-runner-watch-outs]]. Read that before changing anything about which rows count as finished or how run times are recorded, since the pruning job depends on both.

## Open points

Whether anyone downstream reads the run-ledger beyond the window. Some platform engineers may have scripts or dashboards that look back further than the new window. Ask around before step four. If there are such readers, the answer might be an export before pruning, not a longer window; but that is for the owners of those readers to say, and it is not decided here.

Whether the first pass should be split across several nights. A single large first deletion is simple but heavy. The alternative is to move the cutoff gradually from old to new over a few nights, so each night removes a modest amount. That adds a small amount of logic that is used once. My lean is to rely on batching and the quiet hour and skip the gradual cutoff, unless the dry mode shows a very large backlog.

Whether to keep a tiny summary of what was pruned, such as only a count per night, in a place that is not pruned. The log lines already carry this, and the workflow keeps its logs for a while. I do not think a summary table is worth the extra schema, but note it if someone asks why the file has no record of old runs.

Whether the dry mode should stay as a permanent option. It is useful for any later change of the window, so keeping it is cheap. Leave it in, off by default.

## Status

Decision made: the window is 90 days, replacing the earlier 180 days. Not yet started: the single configured value, the dry mode, the tests for the cutoff boundary, the schema check for child rows, and the backup before the first real pass. Update this section as each one lands, and when the first real pass has run and the counts look normal, mark the plan done and leave the decision part as it is for whoever asks later why the number is what it is.
