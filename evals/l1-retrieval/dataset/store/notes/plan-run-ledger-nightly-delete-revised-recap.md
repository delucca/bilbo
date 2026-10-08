---
id: 01M1RVQ7AX20MYVCVGRQ9PV87G
created: 2026-09-05T10:21-03:00
---

# Run-ledger nightly delete, revised plan

Second pass at the nightly delete for the run-ledger. The first idea was too eager and I want the revised shape written down before I lose it. This is loose and partial. It covers the direction and the open questions, and it leaves out the exact values. Those live in config and should stay there.

The short version: the run-ledger keeps growing because every upgrade attempt, every targeted test run and every retry writes rows. Nobody reads most of them after a short while. A nightly job should remove the old ones, but only the ones that are safe to remove, and it should do so in small batches so it never holds the SQLite file locked for long. The earlier draft deleted everything older than the retention window in one statement. That is the part being revised.

## What the run-ledger holds and why it grows

The run-ledger is the SQLite store where PatchPilot records what it did for each dependency upgrade. There is a row per run, and child rows for each step: the branch it cut, the pull request it opened, the targeted tests it chose, the result of each test invocation, and the final verdict. There are also rows for retries and for the times a run was superseded by a newer one for the same dependency.

The growth comes from three places.

- Platform engineers maintain many repositories, so one scheduled sweep produces a run for every repository with something outdated. Most of those runs are small, but there are a lot of them.
- Test output is the heavy part. Even when the log text itself is kept elsewhere, the ledger keeps summaries, durations and per-test status rows. Those child rows dwarf the parent rows.
- Superseded and abandoned runs are never cleaned up today. A run that was replaced by a newer upgrade for the same package stays forever, along with all of its children.

The file lives on a volume mounted into the Docker container. When it gets large, backups get slow, the container start gets slow because of the integrity work we do on boot, and the GitHub Actions jobs that restore a copy for read-only reporting take noticeably longer. That is the practical reason to care. Nobody has hit a hard disk limit yet.

I keep wanting to treat all of this as one retention problem, but it is really two. One is how long to keep the detail, meaning the child rows and per-test results. The other is how long to keep the fact that a run happened, meaning the parent row and its verdict. The revised plan separates them.

## The revised shape of the nightly delete

The first draft was: once a night, delete every run older than the configured retention window, cascade to the children, done. Problems with that, in the order I noticed them:

- It can delete a run whose pull request is still open. The upgrade PR may sit for a long time waiting on a human, and the ledger row is how PatchPilot knows the PR is its own and what the last verified state was. Deleting that row makes the tool treat the PR as foreign, or worse, open a duplicate.
- It can delete the most recent verdict for a dependency in a repository. We use the latest verdict to decide whether to skip a package that failed verification recently. If the only record of the failure ages out, the tool will try the same upgrade again and fail the same way, and again after that.
- One big delete in SQLite is a long write transaction. Readers on the reporting side may be fine under write-ahead logging, but the writer that records live runs has to wait, and a live run waiting on the ledger is a bad place to stall.
- Reclaiming the space is a separate step. Deleting rows does not shrink the file. The free pages get reused, which is fine for steady state, but it does not help the backup size on its own.

So the revised version has these rules.

1. Only terminal runs are eligible. A run is terminal when it has a final verdict and no open pull request attached. Anything still in progress, queued, or tied to an open PR is skipped regardless of age.
2. Keep the latest run per repository and dependency pair, whatever its age. This preserves the skip-recent-failure behavior and the duplicate-PR protection. It costs a small, bounded number of rows, since it scales with the number of pairs and not with time.
3. Use two ages instead of one. Past the shorter age, delete the child detail rows (per-test results, step timings) but keep the parent row and verdict. Past the longer age, delete the parent too. Both ages are config values with defaults that match what the team usually asks for. I am not writing them here so that nobody copies a stale number out of this note.
4. Delete in batches. Select a bounded set of eligible parent identifiers, delete their children and then the parents in one short transaction, commit, pause briefly, repeat until nothing eligible remains or the time budget for the night runs out. The batch size and the pause are config too.
5. Stop at a time budget. If the backlog is large the first few nights, the job does a slice and quits. It picks up again the next night. That is fine. It should log how much is left so someone can see it converging.
6. Do the space reclaim separately and less often. See the section below.

Order matters inside a batch. Children first, then the parent, in the same transaction, so a crash in the middle leaves either the old state or the new state and never an orphan. I do not want to rely only on foreign key cascades, because I am not sure the connection used by the job has them switched on. Check that, and if it is not on, either turn it on for this connection or delete explicitly. Explicit is safer and costs nothing.

On selecting eligible rows: the query needs an index that supports the age check combined with the terminal flag. I believe there is already an index on the timestamp used for ordering in the dashboard, but I have not confirmed whether it covers the status column. Look at the query plan before shipping. If the plan scans the whole table every night, the job itself becomes part of the problem.

## Safety, scheduling and where it runs

Who runs it. Two options were on the table. One is a scheduled GitHub Actions workflow that pulls the database from the volume, runs the delete, and puts it back. I dislike this, because copying a live SQLite file around invites losing writes made in between. The other is a scheduled task inside the long-running service container, using the same connection settings as the rest of the service, so it takes the same locks and sees the same state. I am leaning to the second. The workflow can still do a read-only check afterwards, such as reporting row counts before and after, but it should not be the thing that mutates the store.

Only one instance at a time. If the service is ever run with more than one replica, two nightly jobs would fight. Use a simple lease row in the ledger itself, or a lock the service already has for the sweep, and skip the night if the lease is held and fresh. A stale lease from a crash should expire on its own after a generous time. Do not make it depend on someone clearing it by hand.

Do not run during the sweep. The scheduled upgrade sweep is the busiest write period. The delete should be placed well away from it and should also check whether a sweep is active before starting and bail out if so. The batch pause gives live writers room even outside that window, but avoiding overlap altogether is cheaper than reasoning about it.

Dry run first. Before the real thing is enabled, the job needs a mode that does everything except the delete: it selects the eligible set batch by batch and logs counts per category (how many parents, how many child rows, how many skipped because of an open PR, how many protected as latest per pair). Run that for a few nights in a real environment and compare the counts against what a person would expect. If the protected-latest count looks wrong, the whole design is off. I would rather find that in the log than in missing history.

Backup before the first real run. Take a copy of the file through the SQLite backup mechanism, not a plain file copy, and keep it for a while. After the first few real nights, drop that requirement.

What to log. One structured line per batch and one summary per night: counts removed by category, counts skipped by reason, time spent, whether the budget ran out, and rows remaining. No row contents. Repository names are fine, since they are already in other logs, but there is no reason to repeat verdict text.

What the dashboard and the reports expect. Some reports compute success rates over a trailing period. If the detail rows are gone after the shorter age but the parent verdicts remain, success rates still work for the longer period. Per-test flakiness reports will only reach as far back as the detail window. That is a real trade and the team needs to know it. Mention it in the release notes for whichever change turns this on. I should also check whether any report joins parents to children with an inner join, since a parent whose children were pruned would then silently vanish from the result. That would look like a drop in activity and cause confusion. Switch those to outer joins, or add a flag on the parent saying its detail was pruned.

That last point suggests a small schema addition: a nullable marker on the parent row recording that detail has been pruned and when. It makes the reports honest and it makes the job idempotent, because the detail pass can skip parents already marked. Without it the job would re-examine the same parents every night. A migration for the marker needs to be written in the same style as the existing ledger migrations, and must be safe to run against a file that is in use.

## Space reclaim, open questions and next steps

Space reclaim. Deleting rows leaves free pages inside the file. For steady state this is fine, since new rows reuse them. But the first big cleanup will leave a file much larger than its content, and the backup and restore times will not improve until the file is compacted. Options:

- Incremental vacuum, if the database was created with the auto-vacuum mode that supports it. I do not remember whether it was. If it was not, switching modes requires a full rebuild once, which is a downtime event.
- A full vacuum run occasionally, say after a large backlog has been cleared, during a maintenance window with the service paused. It needs enough free disk for a second copy of the file, so check the volume has room first.
- Do nothing and rely on reuse. Acceptable if the only goal is to stop growth, not to shrink the file.

My recommendation: do nothing about reclaim in the first change. Ship the delete, watch the free page count over a couple of weeks, then decide. If the file is still large relative to content after the backlog is gone, schedule one compaction in a maintenance window and consider incremental vacuum for later. Keeping reclaim out of the first change keeps the first change small and easy to revert.

Open questions, still unresolved:

- Is the foreign key enforcement actually on for the connection the service uses? Needs a look at how connections are opened. If it differs between the service and the CLI tools, that is a bug worth its own note.
- Does anything outside the ledger reference run identifiers? The PR body templates might include a run identifier in a footer, and a link back from the PR to a pruned run would land on a not-found page. Probably acceptable for old runs, but decide what the not-found page says.
- Should abandoned runs, the ones that never reached a verdict because the process died, be eligible earlier than normal ones? They carry little information. A separate shorter age for them is simple to add, but one more knob is one more thing to explain. Leaning to treating them as terminal after a grace period and then using the normal ages.
- How should repositories that were removed from management be handled? Their rows will age out by the normal rule, except that the latest-per-pair protection keeps one row per pair forever. For removed repositories that protection is pointless. A periodic sweep that removes the protected rows for unmanaged repositories would close the gap. Separate change, lower priority.
- Where does the retention config live, and can a platform engineer override it per repository? Per-repository override sounds nice, but it makes the nightly selection query more complex and harder to index. Start with one global setting for each age and revisit if someone actually asks.
- Timezone of the schedule. The job should run in the quiet part of the day for the people who own the repositories, and those people are not all in one place. Pick an off-peak hour for the host and document it. Do not try to be clever.

Tests to write when this is implemented. Unit-level tests against an in-memory SQLite database seeded with a mix of runs: terminal and non-terminal, with and without open PRs, old and new, latest-per-pair and not. Assert exactly which rows survive. Then a test that a batch interrupted halfway leaves no orphans. Then a test that the time budget stops the job and the next invocation continues. Then a test that two concurrent invocations do not both proceed. The targeted-test selection that PatchPilot itself does for its own repository should pick these up from the changed files, but double check that the selection actually maps the delete module to the ledger tests, because a change that touches deletion and runs no ledger tests would be silly.

Next steps, in order:

1. Confirm the connection settings and the auto-vacuum mode, and write down what was found.
2. Add the pruned-detail marker migration, with the report queries adjusted for it.
3. Implement the eligibility query and check its plan against the existing indexes; add an index if the plan scans.
4. Implement the batch delete with the time budget, the lease and the sweep-active check.
5. Dry-run mode and structured logging, then run dry for a few nights.
6. Take a safe backup, enable the real delete with conservative settings, watch the counts.
7. After a couple of weeks, look at the free page count and decide about compaction.

The thing to hold onto if this note is all that gets read: never delete a run that is still in progress or has an open PR, always keep the latest per repository and dependency pair, prune detail before parents, delete in small committed batches, and keep reclaim out of the first change.
