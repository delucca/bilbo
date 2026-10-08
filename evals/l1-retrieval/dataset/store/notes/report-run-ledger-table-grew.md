---
id: 01KR799W84Y5H96ZS1PGAXFB4Q
created: 2026-05-09T18:11-03:00
---

# run-ledger growth report

After eight months of operation the run-ledger table reached 2.4 million rows and 1.1 GB on disk. That is the headline number and the reason this note exists. The run-ledger is the SQLite-backed record PatchPilot keeps of every upgrade attempt: which repository, which dependency, which target version, which tests were selected, what happened, and how long it took. Nobody planned for it to be the biggest thing in the data directory, but it is, and it is now large enough that several things around it have started to feel slow. This note records what the growth looks like, what we think is driving it, what it costs us today, and what we would do about it. It is written from observation of the running system, not from a formal benchmark, so treat the cost statements as directional.

The short version: the table is not broken, nothing has failed because of its size, but reads that used to be instant are now noticeable, backups are heavier than they need to be, and the Docker image for the local dev setup is awkward to seed. Most of the rows are old, finished, and never read again. A retention policy plus a summary table would remove most of the weight without losing anything the product actually uses.

## What the run-ledger holds

Each row is one step in the life of an upgrade run, not one run. A run for a single dependency bump in a single repository produces a handful of rows as it moves through states: discovered, branch prepared, pull request opened, tests selected, tests started, tests finished, and then a terminal state such as merged, closed, abandoned or failed. Some runs produce more rows because they retry. This is the main reason the row count is so much larger than the number of pull requests ever opened. Anyone estimating growth from the count of pull requests will undershoot by a wide margin.

The columns fall into three groups. The first group is identity and routing: a run identifier, the repository, the package ecosystem, the dependency name, the from and to versions, and a timestamp. The second group is outcome: state, a short reason code, duration, and a link back to the pull request. The third group is the bulky one: a free-text detail field and a serialized blob holding the selected test targets and a trimmed copy of the test output. The third group is where most of the bytes live. The identity and outcome columns on their own would make for a modest table; the detail field and the blob are what push it past a gigabyte.

The detail field was meant to be short. In practice, failure messages from package managers and test runners are long, and nothing truncates them on the way in. A single noisy failure can write far more text than a hundred successful rows put together. Rows from the early months, before the output was trimmed, are especially heavy. Later rows are smaller, but there are more of them because more repositories were onboarded.

The table also carries a few indexes. One covers repository plus time, one covers dependency name plus target version, and one covers state. These make the common lookups fast, and they also take up real space. Part of the on-disk figure is index pages, not row data. When we talk about the size of the run-ledger we mean the whole thing, table and indexes together, as reported by the file size of the database.

## How the growth happened

Growth was not linear. The first months were quiet because only a few repositories were connected and the schedule was conservative. Once platform teams started pointing PatchPilot at larger groups of repositories, the volume of runs jumped, and it jumped again when the scheduled sweeps were made more frequent. Each time, the row count moved in a step rather than a slope. Since then the rate has been fairly steady, which means the next eight months will add more than the first eight did unless something changes.

Three behaviors multiply the row count more than anything else.

First, retries. When a test run fails for a reason that looks transient, such as a registry timeout or a flaky runner, PatchPilot tries again. Every attempt writes its own set of rows. A repository with an unreliable test suite can generate many times the normal number of rows for the same dependency bump.

Second, re-evaluation. When a new version of a dependency appears while an older upgrade pull request is still open, the old run is not simply left alone. It is re-checked, and each re-check writes rows. On busy ecosystems where new patch releases arrive constantly, a single stale pull request can be re-evaluated again and again.

Third, no-op sweeps. A scheduled sweep that finds nothing to upgrade still writes a row per repository so that we can tell the sweep happened. These rows are tiny, but they are numerous, and they never stop. They are a large share of the row count and a small share of the bytes.

None of this is a bug. Each behavior has a reason. The problem is only that the ledger was designed as an append-only history and we never decided how long to keep that history.

## What it costs today

The most visible cost is on the read side. The dashboard view that lists recent runs for a repository is still fine, because it uses the repository and time index and only touches a small slice. The views that aggregate across repositories, such as success rate by ecosystem or time spent in tests per week, have become slow enough that people have noticed. They scan far more of the table than they need to because the question being asked is about trends, and the table is storing events.

The second cost is write contention. SQLite allows only one writer at a time, and the ledger is written from several places: the orchestrator, the test runner step, and the reporting step. With a small table, writes finish quickly and nobody waits. With a larger table and larger blobs, each write holds the lock a bit longer, and under a burst of concurrent runs we now see occasional waits on the lock. They resolve on their own, but they show up in logs as delays and they make the system feel less predictable than it should.

The third cost is operational. Backups and snapshots copy the whole file. A database over a gigabyte is not enormous by general standards, but it is large for something that is supposed to be a small embedded store, and it changes constantly, so incremental approaches get little benefit. Restoring a copy for debugging takes long enough that people avoid doing it, which is its own kind of cost, because it means problems get investigated from logs instead of from the data.

The fourth cost is the container. The Docker image used for local development and for integration checks in GitHub Actions can be seeded with a copy of real data. With the ledger at its current size, seeding is slow and the resulting layer is heavy. People have started using empty databases in CI instead, which keeps the pipeline fast but means the checks no longer exercise realistic data volumes. That is a quiet loss of coverage.

The fifth cost is cognitive. When someone opens the database to figure out why a particular upgrade failed, the sheer number of rows makes it hard to see the story of one run. Ad hoc queries need to be careful about indexes, or they crawl.

## What actually reads old rows

Before cutting anything, it matters what depends on old data. We looked through the code that queries the run-ledger and sorted readers by how far back they look.

Most readers look back a short distance. The scheduler checks recent history to avoid opening a duplicate pull request for the same dependency and version, and to back off on repositories that keep failing. The test selection step looks at recent outcomes to decide whether a previously flaky target should be run first. The dashboard lists recent runs. All of these care about weeks, not months.

A smaller set of readers look back further. The reporting job that produces periodic summaries for platform teams aggregates over longer windows. Those summaries only need counts, durations and outcome categories, not the detail text or the test output blob. A rollup would serve them equally well.

A few uses look back at everything, and these are the ones to be careful with. One is the audit-style question of whether a given version was ever proposed for a given repository. Another is the occasional investigation into why a dependency was pinned or skipped. These need identity and outcome columns, not the bulky columns, and they are rare enough that a slower query is acceptable.

The conclusion is that the bulky columns are almost never needed after a short window, the identity and outcome columns are useful for a long time, and event-level rows are rarely needed for long-range reporting if a rollup exists. That shape suggests tiered retention rather than a single cutoff.

## Options considered

The first option is to do nothing and add disk. It works for a long while, since the amount of data is not frightening in absolute terms. But it leaves the write contention, backup weight, container seeding and slow aggregate views in place, and the table keeps growing at a rate that is now higher than when we started. It defers the work rather than avoiding it.

The second option is a plain age cutoff: delete rows older than some threshold on a schedule. It is simple and would shrink the file after a vacuum. Its downside is that it throws away the long-lived identity and outcome information the audit-style questions rely on, and it makes long-range reports silently change when rows disappear. It is acceptable only if paired with a rollup.

The third option is to trim the bulky columns but keep the rows. After a short window, null out the detail text and the test output blob, leaving the identity and outcome columns intact. This preserves the history that matters and removes most of the bytes. It does not reduce the row count, so index size and scan cost for aggregates are mostly unchanged, but it addresses the file size and backup weight directly. A vacuum is needed afterward to actually return space, and that has its own cost, discussed below.

The fourth option is to move the bulky data out of the main table into a side table keyed by run identifier, and expire the side table aggressively. This is the cleanest long-term shape, because the hot table stays narrow and scans touch fewer pages. It requires a migration and a change in every writer and reader of the bulky columns, so it is the most work.

The fifth option is to add a rollup table that stores per-repository, per-day or per-week aggregates, populated by a periodic job, and point the long-range views at it. This fixes the slow dashboards without touching the ledger itself, and it makes later deletion of old event rows safe because nothing important depends on them.

The sixth option is to move the ledger off SQLite into a server database. That would solve write contention for good, but it changes the deployment story for a tool that is intentionally easy to run, and nothing here suggests the single-file store has hit a hard limit. It is noted for completeness and is not recommended at this stage.

## Recommended plan

The recommendation is to combine the third, fifth and, later, the fourth options, in that order of effort. Start with what is cheap and reversible, and only then change the schema.

Step one is to add the rollup table and move the long-range reporting views onto it. This comes first because it removes the dependency of reports on raw events, which is what makes any deletion safe. The rollup job should be idempotent, so that re-running it for a period produces the same result, and it should record which periods it has covered, so that a gap is visible rather than silent.

Step two is to trim bulky columns on a schedule. Pick a window comfortably longer than anything the scheduler or test selection looks back over, and clear the detail text and output blob for rows older than that. Do it in small batches, committing between batches, so the writer lock is never held for long. Doing it as one giant statement would be worse than the problem it solves, since it would block every other writer for the duration.

Step three is to cap what goes in. Truncate the detail field and the test output at write time to a sensible maximum, and keep the tail of the output rather than the head, since failures usually explain themselves at the end. This stops the problem from reappearing at the same rate. It is also the cheapest change and could safely go first if there is a reason to hurry.

Step four, once the above is in place and has been observed for a while, is to decide whether to delete old event rows entirely beyond a long horizon, keeping only the rollup. If the audit-style questions need per-version history, keep a slim table with one row per repository, dependency and target version, holding the first and last time it was proposed and its final outcome. That is small and answers the question without the event stream.

Step five is the side-table split, only if the earlier steps do not bring the hot table to a comfortable size. It is the most invasive and should be justified by measurements taken after the earlier work.

No-op sweep rows deserve separate treatment. They carry almost no information beyond the fact that a sweep ran. Replacing them with a single heartbeat record per sweep, or a per-repository last-checked timestamp that is updated in place, would remove a large share of the row count with no loss. That change is independent of the rest and can be done at any point.

## Risks and things to watch

Reclaiming space is not free. Deleting rows or nulling columns in SQLite does not shrink the file by itself; freed pages stay in the file for reuse. A full vacuum rewrites the entire database, needs spare disk space about equal to the database size, and holds a lock while it runs. In a deployment where PatchPilot runs continuously, that needs a maintenance window or a different approach, such as enabling incremental auto-vacuum from the start in new databases and running incremental passes after each trim. Changing the auto-vacuum mode on an existing database requires a full vacuum once, so plan for that single expensive operation.

Batch deletion and trimming can still interact badly with the write-ahead log if a long reader holds a snapshot open. If the log grows without bound during a trim, check for long-lived readers, which are often a forgotten dashboard query or a debugging session left open. Keep an eye on the log file size while the first trim runs.

Indexes matter for the trim itself. If the job selects old rows by timestamp and there is no index that serves that predicate alone, each batch will scan more than expected. The repository plus time index may not help when the job is not filtering by repository. Check the query plan before running the first batch on real data, and add an index on time only if the plan shows a scan.

The rollup must agree with the raw data while both exist. Before any raw rows are deleted, compare rollup totals to counts taken directly from the ledger for a handful of past periods and for a handful of repositories. Differences usually come from retries or re-evaluations being counted inconsistently, and it is better to settle what counts as one unit of work now than to discover the disagreement after the evidence is gone.

Backups need a policy that matches the new retention. If old data is deleted from the live database, older backups will still contain it. That is fine, and arguably good, but someone should know how long backups are kept so that the effective retention is understood and not accidental.

Finally, the container and CI seeding story should be revisited once the file is smaller. A trimmed copy of real data would let integration checks exercise realistic volumes again without the slow seeding that pushed people to empty databases. It would be worth producing a seed that is representative rather than complete, for example recent history for a modest set of repositories.

## How to check progress

The simplest signals are the ones we already have. Count rows and note the file size on disk, both before and after each step, and write the numbers down. The starting point is 2.4 million rows and 1.1 GB. A successful trim of bulky columns should show a large drop in file size after reclaiming space, with little change in row count. A successful no-op sweep change should show the opposite, a large drop in row count with a smaller drop in bytes. Seeing both patterns confirms that the two causes of growth were as separate as we believe.

For read performance, pick a few representative queries before starting: the recent runs listing for a busy repository, the cross-repository success rate, and the weekly test time view. Time them against a copy of the current database, then again after each step. The aggregates should improve most once they read from the rollup. The recent runs listing should not change, and if it gets slower, something went wrong with an index.

For write contention, watch for lock wait messages in the logs during bursts. A reduction there is the best evidence that smaller rows and shorter transactions are helping. If waits persist after the table is much smaller, the cause is likely the number of writers and the length of their transactions rather than the table size, and the fix would be to batch writes from the runner and reporter steps.

For correctness, keep a small set of known runs and confirm their identity and outcome columns are unchanged after each trim. The scheduler's duplicate-avoidance behavior is the thing most likely to regress quietly if recent history is damaged, so run a sweep against a copy of the trimmed data and confirm it does not open pull requests that already exist.

## Open questions

How long a window of full detail is actually useful for debugging is not settled. The scheduler and test selection need recent history, but humans investigating a failure sometimes want the original output weeks later. A moderately generous window is cheap compared to the cost of losing the evidence, so lean long at first and shorten it once the real pattern of lookups is known.

Whether retries should be written as separate rows at all is worth a second look. Collapsing them into a single row with an attempt counter would cut row volume on flaky repositories sharply, at the cost of losing per-attempt timing. If per-attempt timing is only used for debugging, the counter plus the last attempt's detail may be enough.

Whether the rollup should be keyed by repository or by a coarser grouping depends on how many repositories platform teams end up connecting. Per-repository rollups are the most flexible, but with a large number of repositories and fine time buckets the rollup itself could become sizeable. Start with weekly buckets and revisit.

Whether to keep SQLite for the ledger over the long term is not an urgent question, but it should be asked again if write contention remains after the table is trimmed and the writers are batched. The single-file store has been a good fit so far, and the growth seen here is the kind that retention fixes, not the kind that demands a different engine.

## Summary of where things stand

The run-ledger reached 2.4 million rows and 1.1 GB after eight months. The weight is mostly in two bulky columns and in event rows that are rarely read after a short window. Retries, re-evaluations and no-op sweeps drive the row count. The cost shows up as slow aggregate views, occasional write waits, heavy backups and awkward container seeding. The plan is to build a rollup first, then trim bulky columns in small batches, cap new writes, thin out no-op sweep rows, and only then consider deleting old events or splitting the table. Reclaiming space needs a deliberate vacuum strategy, and every step should be measured against the starting numbers recorded here.
