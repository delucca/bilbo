---
id: 01JXFN69SR7T53MWQXHFPJHHE3
created: 2025-06-11T11:09-03:00
sources:
  - "code: src/main/scala/shelfsense/ingest/PosIngest.scala"
---

# sales-ingest-pipeline design

The sales-ingest-pipeline reads POS files from `s3://shelfsense-raw/pos/` with Structured Streaming and the `availableNow` trigger into a bronze Delta table. It is a batch-style job built on the streaming API. Each run picks up whatever files have landed since the last run, processes them, and stops. Nothing stays running between runs. This note records how the component is shaped and why, so the next person does not have to rebuild the reasoning from the code.

The component is the front door for sales data in ShelfSense. Stockout prediction and replenishment orders for merchandising analysts at grocery chains all depend on what this pipeline lands. If it is late, wrong or silently incomplete, the forecasts downstream are wrong in ways that are hard to see. Most of the design choices below follow from that.

## What it does

The job has one input and one output.

- Input: raw POS files that stores and chain systems drop under `s3://shelfsense-raw/pos/`. The files are written by systems we do not control, so we treat them as untrusted in shape. Layout, column order and encoding can drift without notice.
- Output: a bronze Delta table that holds the rows as they arrived, plus a small amount of metadata about where each row came from.

The job is written in Scala on Apache Spark. It uses Structured Streaming with the `availableNow` trigger. That choice gives two things we wanted without running a cluster all day:

1. File discovery and progress tracking come from the streaming checkpoint. We do not keep our own list of processed files, and we do not scan the whole prefix on every run to work out what is new.
2. The run has a clear end. Airflow can schedule it like any other task, and a finished run means all data available at start has been consumed.

The stream is not meant to be continuous. If someone changes the trigger to a processing-time or continuous mode, the scheduling, cost and failure assumptions in the rest of this note stop holding. Treat `availableNow` as part of the contract.

A minimal sketch of the read and write, using only the values that matter here:

```scala
spark.readStream
  .format("cloudFiles")
  .load("s3://shelfsense-raw/pos/")
  .writeStream
  .format("delta")
  .trigger(Trigger.AvailableNow())
  .toTable("bronze_pos")
```

This is a sketch, not the production code. The real job also sets the schema handling, the checkpoint location, the options for the file format and the metadata columns. The table name above is a placeholder for the sketch. Read the job source for the actual names before relying on any of them.

## Bronze layer rules

Bronze is a faithful copy of the source, not a cleaned table. The rules we try to keep:

- Keep the raw values. Do not convert, round, trim or reinterpret fields in this job beyond what is needed to store them in typed columns. If a value cannot be parsed, it must not vanish.
- Add provenance. Each row should carry the source file it came from and the time it was ingested, so a bad row can be traced back to a file and a run.
- Never drop rows silently. Rows that fail parsing go to a quarantine path or a rescued-data column, and a count of them is logged by every run.
- Append only. The job does not update or delete bronze rows. Corrections happen in later layers.
- Keep the schema permissive at the edge. New columns from upstream should not crash the job. They are captured and surfaced for a human to look at, and promoted into the typed schema deliberately.

The reason for the append-only rule is replay. If silver or gold logic changes, we want to rebuild from bronze and get the same answer for the same inputs. That only works if bronze is not edited in place. Delta time travel gives some extra safety here, but we should not lean on it as a backup, since retention and vacuum settings can remove old versions.

Duplicates are expected at this layer. POS systems resend files, retailers re-export days, and a file can appear under a new name with the same content. Bronze keeps them. Deduplication is the job of the next layer, which needs the business key of a sale line rather than the file identity. If we ever dedupe in bronze, we lose the ability to see how often upstream resends, and that number is useful when talking to a chain about data quality.

## Scheduling and orchestration

Airflow owns the schedule. One DAG task launches the Spark job, waits for it to finish and reports the outcome. The job is idempotent at the run level: starting it again after a failure picks up from the checkpoint and does not double-write committed batches.

Things worth knowing when touching the DAG:

- A run with no new files is a success. It should finish quickly and write nothing. Do not make Airflow treat an empty run as a failure, or the DAG will page people on quiet days.
- Downstream tasks should depend on this task finishing, not on a clock time. Files arrive late, and a clock-based dependency will read half a day.
- Overlapping runs are not safe. Two jobs sharing one checkpoint will conflict. The DAG should allow only one active run of this task at a time, and a manual run from the command line needs the scheduled one to be paused first.
- Retries are fine for transient failures such as network errors to object storage or a lost executor. They are not fine for a schema or data failure, where a retry just repeats the same error. Fail those fast and tell a human.
- Backfills are done by pointing a separate run at a separate checkpoint and a separate target, then merging deliberately. Do not reset the production checkpoint to reprocess old files. That is easy to do and hard to undo.

The cadence is a business decision more than a technical one. Analysts need fresh enough sales to catch a store running out of an item before it happens. The pipeline is cheap enough per run that the schedule is set by how fresh the downstream forecasts need to be, and by how often upstream actually delivers, rather than by cluster cost.

## Failure modes and recovery

The failure cases we have thought about, and what the design does with each.

**Malformed or surprising files.** A file with the wrong columns, a truncated body or an unexpected encoding. The job should isolate the bad records, keep the good ones, record the file name and the reason, and keep going. A single bad file must not block a whole day of sales for every store. If the share of bad records in a run goes over a threshold that the team agreed, the run fails loudly instead of passing quietly with holes in the data.

**Late and out-of-order files.** Files for an earlier business date can show up after files for a later one. Ingest time and sale time are different things, and bronze stores both. Anything downstream that assumes files arrive in business-date order will be wrong sometimes. The ingest job does not try to order them.

**Missing files.** The hardest case, because nothing errors. If a store never sends its file, the pipeline sees nothing and succeeds. Detecting this needs an expectation of which stores should report, which does not live in this job. It belongs in a freshness check on the bronze table, per store or per chain, run after ingest. Without that check, a missing store looks identical to a store that sold nothing, and the stockout model will read it as zero demand.

**Checkpoint trouble.** If the checkpoint is lost or corrupted, the next run would treat the whole prefix as new and re-ingest everything. Because bronze keeps duplicates, this would not corrupt data, but it would inflate volume and cost, and downstream dedupe would have to absorb it. Keep the checkpoint on durable storage, do not clean it up as scratch space, and treat deleting it as a deliberate operation.

**Partial writes.** Delta commits are atomic per batch, so a failed run leaves the table at the last good commit. Readers never see half a batch. This is one of the main reasons for Delta here rather than plain files.

**Schema drift.** Upstream adds, renames or retypes a column. Added columns are captured. Renames and type changes are the problem: they can look like a new column plus a column that went all null. The run log should say when a known column stopped appearing, because that is how a rename shows up in practice.

**Small files.** Frequent short runs produce many small files in the Delta table. Compaction should run on its own schedule, separate from ingest, so that ingest stays fast and compaction cost is visible on its own.

## Downstream contract and open questions

The bronze table is read by the next layer, which cleans, types and dedupes sales lines, and which in turn feeds the data that reaches Snowflake for analyst-facing work. This job does not write to Snowflake directly. Keeping that boundary means an ingest problem can be fixed and replayed inside the lake before any analyst sees it.

What downstream can count on from this component:

- Every row that arrived and could be stored is in bronze, with its source file and ingest time.
- Rows that could not be parsed are accounted for somewhere, with a count, and not just missing.
- Once a run succeeds, everything that was available at its start has been consumed.
- The table only grows by appending.

What downstream must not count on:

- Uniqueness of sales lines.
- Ordering by business date.
- Completeness per store. That needs the separate freshness check.
- A stable column set. New columns can appear.

Open questions that nobody has settled yet:

1. Where the freshness check for missing stores should live, and who owns the list of stores that are expected to report. It probably needs input from the merchandising side, since they know which stores are closed or on holiday schedules.
2. Whether quarantined records should be reprocessed automatically once a parser fix ships, or whether a human should trigger the reprocess. Automatic is convenient but can surprise people.
3. How long raw files stay in the source prefix. The checkpoint tracks files by path, so moving or deleting them has consequences for replay and for any later audit. This should be agreed with whoever manages the bucket before any lifecycle rule is turned on.
4. Whether one job should cover all chains or whether large chains deserve their own run so a bad feed from one cannot slow the others. One job is simpler today. Splitting is cheap to do later if the checkpoint layout allows for it, so avoid design choices that make splitting hard.
5. How schema promotion from captured extra columns into the typed schema is reviewed and who signs off.

If you change this component, check these before merging: the trigger is still `availableNow`, the source is still the same prefix, `s3://shelfsense-raw/pos/`, bronze is still append only, and the run still reports counts for rows read, rows written and rows quarantined. Those three counts are what let a person tell on a bad morning whether the problem is upstream, in the parser or in the pipeline itself.
