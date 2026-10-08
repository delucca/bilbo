---
id: 01KPVNYWVP7KREZQ4WTS86RPDZ
created: 2026-04-22T19:46-03:00
---

# store-features-delta: OPTIMIZE collides with the feature job append

Do not run OPTIMIZE on store-features-delta while the feature job is appending to it. When the two overlap, the writer fails with `io.delta.exceptions.ConcurrentAppendException`. The append is the thing that dies, not the compaction, so the symptom shows up as a failed feature run and not as a failed maintenance task. Easy to misread at first.

## Symptom

The feature job's write step fails partway or at commit time. The Spark driver log shows `io.delta.exceptions.ConcurrentAppendException`. The Airflow task for the feature job goes red and retries. Downstream replenishment order generation then waits on features that never landed, or worse, runs against yesterday's features if someone forces it through.

The error can look random. If OPTIMIZE and the append do not overlap in time, nothing fails. So a run can pass for days and then fail on the one night maintenance runs long.

## Why it happens

OPTIMIZE rewrites many small files in store-features-delta into fewer large ones. It commits a transaction that removes the old files and adds the compacted ones. The feature job commits an append to the same table. Delta Lake checks the two commits for conflicts when the later one tries to commit. Depending on which side commits first and which files and partitions each one touched, the check decides they conflict and aborts the later writer.

In our case the feature job is usually the later committer, because OPTIMIZE reads a snapshot, does a long rewrite, and the append lands inside that window. The append gets the exception. Partition layout matters: if the append writes into partitions that OPTIMIZE is also compacting, the conflict is much more likely. Appends to partitions OPTIMIZE is not touching are less likely to collide, but do not rely on that. We have not mapped exactly where the line is.

## What to do instead

Keep the two apart in the schedule. The simplest rule is that compaction of store-features-delta must not start until the feature job has finished, and must finish before the next feature run begins. Put this in Airflow as a dependency, not as a guess about timing.

- Make the maintenance task depend on the feature job's completion for the same logical date.
- Make the next feature run wait for the maintenance task, or use a pool or a shared lock so only one of them holds the table at a time.
- If maintenance is a separate DAG, use a sensor or an explicit cross-DAG dependency. Clock-based offsets drift and will fail again.
- Limit OPTIMIZE to partitions the feature job is not writing, if you must run it near the job. Treat this as a fallback, since it is easy to get wrong.

## If it already failed

The failed append did not commit, so the table is not corrupted. No data from that attempt is in store-features-delta. Check that OPTIMIZE has finished, then rerun the feature job. A plain rerun should succeed once nothing else is rewriting the table.

Do not add a blind retry loop inside the Spark job as the main fix. A retry can work if OPTIMIZE finishes quickly, but a long compaction will outlast the retries and you get the same failure after wasted compute. A modest retry is fine as a backstop, not as the design.

Before rerunning, look at what else touches the table. Ad hoc notebooks, backfills, and Snowflake export jobs that read it are not a concern for this error, since readers do not conflict with appends. Another writer or another maintenance command is.

## Checking for it

When a feature run fails, search the driver log for the exception name first. If it is there, look at the Delta table history for store-features-delta and see whether an OPTIMIZE operation has a timestamp overlapping the failed run. That confirms the cause in a minute and saves a long hunt through Spark executor logs.

Things that look similar but are different:

- A failure about schema mismatch on write is a feature code change, not this.
- A failure in the Snowflake load step after the Delta write succeeded is a different problem; the Delta table was fine.
- Executor out-of-memory during the write is resource sizing, not a conflict.

## Open points

- Nobody has confirmed how long OPTIMIZE takes at full table size on a bad night. Worth measuring so the Airflow gap is sized from data.
- It is unclear whether other maintenance commands on the same table, such as vacuum, interact with the feature job the same way. Assume they need the same separation until someone checks.
- If the table is repartitioned, revisit the partition-based fallback above, since it depends on layout.

## Short version

OPTIMIZE and the feature job's append on store-features-delta must never overlap. If they do, the writer fails with `io.delta.exceptions.ConcurrentAppendException`. Fix it with an Airflow dependency, then rerun the feature job after maintenance finishes.
