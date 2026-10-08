---
id: 01KG4TNHX7PHDGSDPDQ76451T0
created: 2026-01-29T09:11-03:00
---

# store-features-delta OPTIMIZE result and small-file cleanup

Running `OPTIMIZE` on store-features-delta cut the file count from 41,000 to 2,300. This note records what that means for ShelfSense, why the table had so many files, and what to watch so the count does not creep back up. It is a report, not a plan. Details that I did not measure are kept general on purpose.

## What happened

store-features-delta is the Delta Lake table that holds the per-store, per-product features the stockout model reads. Over time it had piled up a very large number of small files. After `OPTIMIZE` was run on it, the file count went from 41,000 to 2,300. That is a large drop, roughly a factor of eighteen, and it came from compaction alone. No data was removed and no schema was changed.

The table is still the same logical table. Readers see the same rows. What changed is how many physical files Spark has to list, open and plan around.

## Why there were so many files

The table is fed by frequent incremental writes from Spark jobs that Airflow schedules. Each run writes a batch of new or updated store feature rows, and each batch is spread over many shuffle partitions. Many of those partitions hold very little data, so each write leaves a pile of tiny files.

Two things made it worse. First, the table is partitioned in a way that splits data across many directories, so a single write touches many partitions and creates at least one file in each. Second, late-arriving corrections from stores rewrite small slices of older partitions, which adds still more small files there. Nothing in the pipeline compacted them, so they accumulated until the table was slow.

## Symptoms before the cleanup

The people who use ShelfSense are merchandising analysts, and they noticed the symptoms indirectly. Replenishment order generation took longer than it should have, and the feature-building stage of the daily run was the slowest part. In Spark, the job spent a lot of time on planning and on listing files before it did any real work. Task counts were high and each task did very little.

On the Delta side, the transaction log had grown long with many small add actions. Reading the table state took longer as a result. None of this caused wrong output, only slow output, which is why it went unnoticed for a while.

## What OPTIMIZE did

`OPTIMIZE` rewrites many small files into fewer larger ones within each partition. Delta marks the old files as removed in the log and adds the new compacted files. The old files stay on storage until they are vacuumed, so the physical storage footprint does not shrink right away even though the live file count did.

The result, 41,000 down to 2,300, is the live file count in the current table version. Anyone checking storage listings directly may still see the old files until cleanup of removed files has run, and should not read that as the compaction having failed.

## Effect on reads

Reads of store-features-delta now plan faster because there are far fewer files to list and far fewer tasks to schedule. Scans of a single store or a small date range benefit most, because they used to open many tiny files to find a small amount of data. Larger scans benefit too, mainly through fewer tasks and less overhead per task.

I did not record an exact before and after runtime, so I am not quoting one here. If a number is needed, measure the feature-building stage of the daily run before and after the next compaction and note it in a new report.

## Effect on writes

Writes are not much faster. They still produce small files per batch, so the table will drift back toward the old state unless something compacts it regularly. Compaction itself is a heavy job: it reads and rewrites a lot of data, so it competes with the regular pipeline for cluster resources if scheduled at the wrong time.

Concurrent writers also matter. Delta uses optimistic concurrency, and a compaction that overlaps with a write to the same partitions can conflict. Compaction should run when the feature-writing jobs are idle, or be limited to partitions that are not being written.

## Interaction with VACUUM and time travel

After `OPTIMIZE`, the replaced files are only logically removed. Running vacuum deletes them physically once they are older than the retention period. Until then, time travel to earlier versions still works, since the old files exist. After vacuum, versions older than the retention window can no longer be read.

Do not shorten the retention period just to reclaim space quickly. Model backtests and debugging of past replenishment orders sometimes need an earlier version of the features, and a short retention would break that. Keep the default unless someone has decided otherwise and written it down.

## Downstream consumers

The main consumer is the Spark job that builds model inputs for stockout prediction. Some feature aggregates are also exported to Snowflake for analyst-facing views. The export reads from store-features-delta, so it gets the same benefit of fewer files. Compaction does not change content, so the Snowflake side should see no difference in values, only a faster load.

Nothing downstream needed a code change. If a consumer pins to a specific table version, check that the version still exists after vacuum.

## Keeping the count down

One compaction is not a fix. These are the options, with my recommendation first.

- Schedule a regular compaction as its own Airflow task, after the feature writes finish and before the stockout model reads. This is the recommended route, because it is simple and visible.
- Reduce the small files at write time by coalescing or repartitioning before the write, so each batch writes fewer, larger files. This helps but does not remove the need for compaction because late corrections still create small files.
- Turn on Delta's auto-optimize style table properties for optimized writes and auto compaction. This is convenient, but it adds write latency and is less predictable, so it should be tried on a copy first.

Whichever is chosen, track the live file count over time so growth is noticed early.

## Open questions

The partitioning of store-features-delta is probably part of the root cause, and it has not been revisited. A coarser layout would give fewer partitions and fewer small files, but it would change read patterns for per-store queries, so it needs a test against real workloads before anyone changes it.

It is also unclear how fast the count grows between compactions. The next step is to record the live file count right after each compaction and again just before the next one, so the growth rate is known and the schedule can be set from data rather than guessed.

## Summary of the state now

store-features-delta was compacted with `OPTIMIZE`, taking it from 41,000 files to 2,300. Reads are faster, content is unchanged, and old files remain until vacuumed. Nothing yet prevents the file count from growing again, so a scheduled compaction in Airflow is the main follow-up.
