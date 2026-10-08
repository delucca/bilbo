---
id: 01K2FS0NEW7W19V0HXCY2EXF42
created: 2025-08-12T15:34-03:00
---

# sales-ingest-pipeline: deleting the pos_ingest checkpoint doubles bronze rows

If the checkpoint directory `/checkpoints/pos_ingest` is deleted, the sales-ingest-pipeline forgets which POS files it has already processed. On the next run it re-reads every POS file from the start, and the bronze row count doubles. Nothing fails. The job goes green, Airflow is happy, and the only symptom is that every number downstream is too big. That quiet success is the trap. Treat that directory as state, not as scratch space, and do not clean it up as a side effect of anything else.

This note is for whoever is about to touch checkpoints, move storage, rebuild an environment, or debug a strange jump in sales volumes. Read it before deleting anything under the checkpoint root.

## What happens

The sales-ingest-pipeline is a Spark job written in Scala that picks up POS files as they land and appends rows to a bronze Delta table. The record of which files it has seen lives in the checkpoint at `/checkpoints/pos_ingest`. When that directory is present, the job resumes where it left off and only reads new files. When it is gone, the job starts as if it were the first run ever: it lists the whole landing area and reads all of it again.

The bronze table is append only. It does not dedupe on write. So the second read of each file adds a second copy of its rows next to the first copy. For every file that was already ingested, there are now two copies. The row count of the bronze table therefore doubles, assuming the landing area still holds the same files that were ingested before.

If the landing area holds more than what was ingested, for example because older files were kept, the growth can be different from exactly double. Doubling is the case to expect when the landing area and the table agree.

## Why the checkpoint matters

Structured Streaming style file ingestion keeps two kinds of progress in its checkpoint: which source files have been committed, and which batches have been written to the sink. Together they give the job its restart behavior. Without them there is nothing to compare the landing area against.

The Delta table itself has its own transaction log, and it is tempting to think that log protects against repeats. It does not do so here. The log records what was written, but the job does not consult it to decide what to read. Only the checkpoint decides that. So a healthy table with a missing checkpoint is a normal looking table that is about to be filled with repeats.

## How to recognize it

The first sign is usually on the analyst side. Daily sales for stores look about twice as large as they should. Stockout predictions get strangely confident, because the model sees high velocity and thinks shelves are emptying fast. Replenishment orders come out inflated.

On the engineering side, check these things:

- The run that followed the deletion took far longer than a normal run, because it was reading the whole backlog rather than the newest files.
- The bronze row count jumped by about as much as the table already held.
- Duplicate rows exist with identical business keys and identical source file names, but different write batches.
- The checkpoint directory has fresh creation times everywhere, which means it was rebuilt rather than resumed.

If you see the last one, someone or something removed it. Find out who or what before fixing the data, otherwise it will happen again.

## Why the count doubles and not something else

People ask whether the job might skip, merge, or error. It does none of these. The source reader has no memory, so every file looks new. The sink accepts appends without checking keys. The result is a plain second copy.

A second deletion before cleanup makes it worse: a third copy, and so on. Each time the checkpoint disappears and the job runs, the table gains another full copy of everything in the landing area. So the damage is repeatable and cumulative. Fix the cause before you fix the data.

## Impact on downstream

Everything built from bronze inherits the repeats. Silver tables that aggregate by store and day will carry doubled units and doubled revenue unless they dedupe by key. Some silver steps do dedupe and some do not, so the same incident can look fine in one table and wrong in another. Do not assume that a clean looking silver table means bronze is clean.

The forecasting and replenishment steps use these aggregates. Inflated demand means the model believes products sell faster than they do. That pushes orders up for the merchandising analysts, who then see large suggested quantities and may act on them. This is the part with real cost: ordered stock that nobody needed. If a doubling is found, tell the analysts that recent suggested orders may be inflated and should be reviewed.

## What not to do

Do not delete `/checkpoints/pos_ingest` to fix a stuck run, to reset a schema, or to free space. It feels like a harmless reset because the job starts again cleanly. It starts cleanly and then reads everything.

Do not point the job at a fresh checkpoint path as a shortcut. A new path is the same as a deleted one. The job has no memory in the new place.

Do not copy the checkpoint from another environment. Its recorded file lists belong to that environment's landing area and will not match here.

Do not edit files inside the checkpoint by hand. The contents are internal and a partial edit can leave the job in a state that is harder to reason about than a clean deletion.

## Safe handling of the checkpoint directory

Treat the directory like a database. If storage is being moved, move it together with the table and keep the same logical path, or take a copy first and verify the copy is complete before cutting over. If a storage cleanup script exists, make sure it excludes the checkpoint root. Retention rules that expire old objects should not apply to it either, since expiring parts of a checkpoint can be as bad as removing all of it.

Access should be limited. Few people and few jobs need write permission on this path. If a maintenance role can delete under the checkpoint root, that is a risk worth closing.

Back it up on a regular schedule if your storage allows it, and keep the backup in line with the table version it matches. A backup that is far older than the table is of limited use, because it will tell the job to re-read files that were already ingested after the backup was taken.

## Recovery when it already happened

First stop the schedule so no further runs add more copies. Pause the Airflow DAG for the sales-ingest-pipeline rather than killing tasks one at a time.

Next, decide between two repair routes.

The first route is to restore the table. Delta Lake keeps table history, so the table can be rolled back to the version just before the repeated read, as long as the history has not been vacuumed away. This is the cleanest path when nothing legitimate was written after the bad run. Check the history and find the version boundary.

The second route is to dedupe in place. Use the business key together with the source file identity to find rows that exist twice, keep one of each, and remove the others. This is needed when legitimate new data arrived after the bad run and a rollback would lose it.

Whichever route, restore or rebuild the checkpoint so it matches the table, then resume. A job restarted with no checkpoint will repeat the whole problem.

## Verifying the bronze table after recovery

Compare counts against an independent source. The landing area file listing and the row counts per file are the best reference, since they do not depend on the bronze table. Per file, the rows in bronze should match the rows in the file, once.

Look for any key that shows up more than once in bronze. After repair there should be none, or only those that were duplicated at the source before this incident. Keep note of known source level duplicates so they are not mistaken for leftovers.

Check the silver and gold layers too, and re-run any that were computed from the bad bronze data. Compare store level totals before and after. If the totals fall back to the level they had before the incident, the repair worked.

Do a final run with a small new batch to confirm the job reads only the new files and adds only their rows.

## Airflow considerations

Airflow schedules the sales-ingest-pipeline, and Airflow does not know about the checkpoint. A task that succeeds tells you the process exited cleanly, nothing about whether the data is right. Because of that, a deleted checkpoint produces no alert on its own.

Consider adding a cheap guard task before the ingest step that checks the checkpoint directory exists and is not empty, and fails the DAG if it is missing. A failing guard is far better than a green run that doubles the table. Also consider a sanity task after ingest that compares the number of rows added against a plausible range for the volume of new files, and fails loudly on a large outlier.

Backfills and clears in Airflow do not reset the checkpoint. Clearing a task reruns the job, and the job consults the checkpoint as usual. That is fine, and it is a reason not to delete the checkpoint when you only want to rerun something.

## Snowflake side

Data is also loaded into Snowflake for analysts. If the doubled bronze rows were propagated, Snowflake tables now hold them as well, and fixing Delta alone does not fix what analysts see. Plan the Snowflake correction together with the Delta correction.

Depending on how the load is built, either a reload of the affected range or a delete of the repeated rows is needed. Reloading from corrected Delta tables is simpler to reason about than patching in place. Check any dashboards or scheduled queries that cache results, since they may keep showing inflated figures after the data is fixed.

Tell the analysts which stores and which range of dates were affected, so they can judge whether any decision they made used the wrong numbers.

## Delta Lake notes

Delta history and vacuum interact with recovery. Rolling back with table history only works while the old files still exist. A vacuum with a short retention removes them, and then the rollback route is closed and only dedupe is left. After an incident, hold off on vacuum for this table until the repair is complete.

Because the sink is append only, adding a merge on a business key would make the pipeline tolerant of repeated reads. That is a design change with a cost in write time and complexity, and it should be considered carefully. It would make a lost checkpoint less harmful, but the right fix is still to not lose the checkpoint.

Optimize and compaction jobs rewrite files but do not change the logical rows, so they neither hide nor fix repeats.

## Spark config notes

The checkpoint location is set in the job configuration, and the path it uses is `/checkpoints/pos_ingest`. If a config change alters that location even slightly, for example a trailing difference, an environment prefix, or a renamed folder, the effect is the same as deleting it. The job will find nothing and start over.

So review any pull request that touches the checkpoint setting with the same care as a schema change. Keep the value in one place, avoid computing it from values that can vary between environments, and do not let it default to something derived from the job name or the run.

Also be careful with upgrades. A major change in how the job reads files could make old checkpoint contents unusable. Test an upgrade against a copy of the checkpoint and a copy of the table first.

## Prevention

A short list of habits that would have prevented this:

- Mark the checkpoint root as protected in storage policy and exclude it from cleanup and retention rules.
- Add a guard task in Airflow that fails when the checkpoint is missing.
- Add a post ingest volume check that fails on implausible growth.
- Keep a documented procedure for environment rebuilds that says to preserve or restore the checkpoint along with the table.
- Review config changes that move or rename the checkpoint location.
- Keep a recent backup of the checkpoint and know which table version it matches.

None of these are complex. The failure is only dangerous because nothing complains when it happens.

## Open questions

Some things are not settled and worth deciding with the team.

Should the bronze write become idempotent, so a repeated read stops mattering? It would trade write cost for safety. Which key would be reliable enough for that, given that POS sources sometimes have their own quirks?

Should the checkpoint be backed up automatically, and where should the backups live so that they survive the same event that removes the original?

Who owns the checkpoint root, and who is allowed to delete from it? Right now that is not written down, and it should be.

Until those are answered, the rule is simple: never delete `/checkpoints/pos_ingest` unless you also intend to rebuild the bronze table from scratch and have planned for it.
