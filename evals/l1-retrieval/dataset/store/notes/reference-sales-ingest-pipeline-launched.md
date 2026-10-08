---
id: 01KMFXWVPTVXZSKSGEGQSDQCS0
created: 2026-03-24T09:42-03:00
sources:
  - "code: src/main/scala/shelfsense/ingest/PosIngest.scala"
---

# sales-ingest-pipeline reference (tillstream)

This is a working reference for the sales-ingest-pipeline, the part of ShelfSense that pulls point-of-sale data from grocery chains and lands it in Delta Lake so the stockout models and the replenishment order generator can read it. Internally the team calls it `tillstream`. If you see that word in a ticket, a chat thread, a dashboard name or an old design doc, it is the same thing as `sales-ingest-pipeline`. This note uses the long name from here on, and only says tillstream when quoting something that used it.

The pipeline is a Scala Spark application. It is launched with `spark-submit --class shelfsense.ingest.PosIngest` from the assembly jar. Airflow schedules it, Delta Lake holds what it writes, and Snowflake gets a downstream copy for the analysts. The rest of this note covers how it is put together, how to run it, what goes wrong, and where to look first. It is written from memory of how the thing behaves, so treat specifics that are not spelled out here as things to check in the repo.

## What it is for

Merchandising analysts at grocery chains use ShelfSense to see which stores are about to run out of which products, and to get suggested replenishment orders. Every one of those predictions depends on knowing what actually sold. The sales-ingest-pipeline is the only path by which sales transactions get into the system. If it is late, the forecasts are stale. If it is wrong, the forecasts are wrong in ways that are hard to spot, because a missing sale looks like a quiet day and a duplicated sale looks like a rush.

So the job has two priorities. First, completeness: every till transaction that a chain sent us should end up in the lake exactly once. Second, timeliness: the data should be there before the morning modelling run starts. When these two conflict, completeness wins, and the downstream jobs wait or run on the previous day with a flag set.

## How it is launched

The entry point is the main class of the ingest module, started with `spark-submit --class shelfsense.ingest.PosIngest` pointed at the assembly jar. The jar is a fat jar built with the Scala build tool, and it bundles the project code plus the libraries that the cluster does not already provide. Spark itself and the Delta Lake package are expected from the cluster or passed as packages at submit time, so check what the cluster image provides before assuming the jar is self-contained.

Arguments and configuration come from two places: command-line arguments for the run-specific things (which chain, which business date or window, whether this is a replay), and a configuration file plus environment for the stable things (storage locations, table names, credentials references). Do not put secrets on the command line. The Airflow task passes only references and the job resolves them at start.

When running by hand for debugging, use the same submit command Airflow uses, copied from the task's rendered template in the Airflow UI. That way you get the same memory settings, the same packages and the same configuration. Running with different settings is the commonest reason that a hand run works and the scheduled one does not, or the reverse.

## Where it sits in the system

Upstream are the chains. Each sends till data in its own way: some drop files into a landing area, some push through a message feed, and a couple still rely on an end-of-day export. The pipeline does not try to hide these differences at the edge. Each chain has an adapter that reads the raw form and turns it into a common transaction shape. After that the code path is shared.

Downstream are three consumers. The feature jobs read the cleaned Delta tables to build rolling sales features. The stockout models read those features. The replenishment order generator reads model output and current inventory. A fourth consumer, the Snowflake share, is for analysts who want to query sales directly. None of these call the pipeline; they read what it wrote. That is deliberate: the pipeline can be rerun or replayed without anyone downstream needing to know, as long as the tables stay consistent.

Airflow ties it together. A DAG per chain group runs the ingest task, then a validation task, then the Snowflake sync, then it triggers or unblocks the feature jobs.

## Input shapes and adapters

Raw till data comes in as line items grouped into baskets, with a store identifier, a till identifier, a timestamp in the store's local time, a product code as the chain knows it, a quantity, a price, and flags for voids, returns and promotions. Chains differ on almost every one of those: the encoding of timestamps, whether the product code is a barcode or an internal code, whether a return is a negative quantity or a separate record type, and whether voided lines are removed or kept with a flag.

Each adapter handles one chain's quirks and nothing else. If you find chain-specific logic outside an adapter, it probably crept in as a hotfix and should be moved. When a new chain is onboarded, the work is mostly writing an adapter and a mapping for product and store identifiers, plus a sample file for tests. The common code should not need changes. If it does, stop and think about whether the common shape is missing a concept.

Time zones are the biggest source of trouble at this layer. The adapter must convert to a single canonical time zone for storage and also keep the store's local business date, because the analysts and the models both think in local trading days. A trading day that runs past midnight is normal for some stores.

## Common transaction shape and cleaning

After the adapter, records go through the shared cleaning stage. It does the following, in this order: parse and validate required fields, drop or quarantine records that fail validation, resolve store and product identifiers against the reference tables, apply the void and return rules so that net quantity is correct, compute the local business date, and add lineage columns that say which source file or feed offset and which run produced the row.

Quarantined records are not discarded. They go to a separate quarantine table with a reason code, so an analyst or an engineer can see what was rejected and why. A rising quarantine rate for one chain is usually the first sign that the chain changed its format without telling us. Check that table before anything else when volumes look low.

Unresolved product codes are the other common rejection. New products appear in stores before the reference data catches up. The pipeline keeps those rows, marked unresolved, rather than throwing them away, because the sale is real and the mapping will usually arrive later. A separate step re-resolves them once reference data is updated.

## Writing to Delta Lake

The cleaned data is written to Delta tables, partitioned by business date and, for the larger chains, by a chain or region key. The partitioning was chosen so that the typical read, which is a window of recent days for a set of stores, touches few files. Do not change the partitioning casually; the feature jobs depend on it for performance, and a change means rewriting history.

Writes are done as merges keyed on a natural transaction key built from chain, store, till, transaction number and line number. That is what makes reruns safe: a replay of the same input updates rows that already exist rather than adding copies. If you ever see duplicated sales, the first thing to check is whether the key is being built differently for that chain, for example because a till number was reused or a line number is missing in the raw data.

Small files are a recurring cost. Intraday feeds create many little writes, so there is a compaction job that runs on a schedule and the pipeline itself avoids producing tiny files where it can. If reads get slow with no change in data volume, look at file counts before looking at code.

## Scheduling in Airflow

Each chain group has a DAG. The ingest task is a Spark submit step that runs the command described above with parameters for the chain and the window. Sensors wait for the landing data or the feed watermark before starting, so a late chain does not cause a failed run, only a delayed one. After a configurable wait, the sensor gives up and the DAG raises an alert so that someone can contact the chain.

Retries are set conservatively. Because writes are merges, a retry is safe, but a retry of a job that failed from memory pressure will usually fail the same way. Look at why it failed before clearing the task again.

Backfills go through the same DAGs with an explicit window. Do not run backfills for many days across many chains at once in the shared cluster during the working morning; they compete with the models for resources. Stagger them and prefer overnight.

## Validation and data quality checks

After ingest, a validation task compares what arrived against expectations. It checks row counts against the previous same weekday for each chain, checks that every active store reported something, looks at the share of records quarantined, and looks for implausible values such as negative net sales for a whole store or a quantity far outside what the product normally sells.

Failures are of two sorts. Hard failures block the downstream tasks: a missing chain, or a quarantine rate that suggests a format change. Soft failures only raise a warning and annotate the data: a few stores silent, which can be real closures. The distinction matters because the silent-store case is common and we do not want it to stop a whole run.

When a store is silent, the downstream models need to know whether the store was closed or the data is missing. That is handled by a store calendar in the reference data, and the validation step consults it. Keep that calendar current; holidays and temporary closures are where false alarms come from.

## Snowflake sync

A later task copies selected tables to Snowflake for analyst use. It is a one-way copy. Nobody writes back, and nothing in the pipeline reads from Snowflake. The copy is incremental by business date, and it re-copies recent dates each time to pick up late-arriving corrections.

Because of that re-copy, Snowflake tables can briefly differ from the Delta tables for the last few days. If an analyst reports a number that does not match, check whether they are looking at a recent date and whether the sync has run since the last ingest. Late corrections, returns processed days after the sale and chains re-sending files are the usual causes of changes to recent days.

Schema changes need care: adding a column in Delta does not automatically add it in Snowflake. The sync task has to be updated, and the downstream views that analysts use may need to be updated as well.

## Replays and backfills

A replay means running the pipeline over data it has already seen, either because the source was corrected or because our logic changed. The merge-based writes make this safe, with one caveat: rows that existed before but should no longer exist, such as lines that the chain has since voided by removing them from the file, are not removed by a plain merge. If the correction removes rows, the replay needs to be run in a mode that deletes the target window first or reconciles it. Know which kind of correction you have before replaying.

For logic changes that affect past data, replay the affected chains and dates, then rerun the feature jobs for those dates, then let the models pick up the change on their next run. Tell the analysts if historic numbers will move; they notice, and they would rather hear it from us.

The lineage columns help here. They show which run wrote a row, so you can tell whether a replay has actually touched the rows you expected.

## Performance and resource notes

The pipeline is mostly shuffle-bound at the merge step and at identifier resolution. Reference tables for stores and products are small enough to broadcast, and the code relies on that. If a reference table grows past what broadcasting handles comfortably, the join strategy needs rethinking; the symptom is executors running out of memory during resolution, not a clean error.

Skew is the other issue. A few large stores or a few very popular products can concentrate data in some partitions. Adaptive query execution helps, and it should stay on, but for the biggest chains the job also salts the merge keys in one place. Do not remove that without testing on a heavy day, such as the day before a holiday.

Cluster sizing is set per DAG. Raising executor memory is the quick fix and often hides a real problem. Before doing it, look at the Spark UI for skew and for spill.

## Failure modes seen so far

Several failures have come up often enough to list. A chain changes its file layout: the quarantine rate jumps, validation hard-fails, and the fix is an adapter change. A chain re-sends a full day: the merge handles it, but the run takes longer and the sync copies more. A time zone change at daylight saving: records land in the wrong business date for an hour, and the fix is in the adapter's local time handling. A reference table is stale: unresolved products pile up, and the fix is in the reference data update, not in the pipeline.

Also seen: the assembly jar on the cluster is not the one that was just built, because the deploy step failed silently or the task points at an older path. When behaviour does not match the code you are reading, confirm which jar version actually ran by looking at the task log and the application's reported version.

## Debugging checklist

Start with the Airflow task log to see which stage failed and with what message. Then open the Spark application page and look at the failed stage: is it a read, the cleaning, the identifier resolution, or the merge? Next check the quarantine table for the chain and date. Then compare row counts for the chain with the same weekday the week before. Finally, check the lineage columns to confirm the run you think wrote the data really did.

If the problem is only in Snowflake, check Delta first. If Delta is right, the issue is in the sync. If Delta is wrong, the issue is upstream of it, and the sync is innocent.

When asking for help, include the chain, the business date, the run identifier and whether it was a scheduled run or a manual one. Those four things save a round of questions.

## Testing

Unit tests cover the adapters and the cleaning rules with small sample inputs per chain. There are also integration-style tests that run the whole job against a local Spark session and a temporary Delta location. They are slower, so they run in the main build but are easy to skip locally while iterating. Run them before changing the merge logic or the key construction, because mistakes there produce duplicates or lost rows that unit tests on single functions do not catch.

When a production issue is found, add a sample reproducing it to the relevant adapter's tests. Real chain data cannot be copied into the repo, so build a minimal synthetic file with the same quirk. Keep the sample small and comment on what quirk it demonstrates.

## Conventions

Code is Scala, formatted by the project's formatter, and it keeps adapters, cleaning, and writing in separate packages. Prefer pure functions on datasets for the cleaning rules, so they can be tested without the full job. Configuration is read once at start and passed down; do not read environment settings deep in the code. Log at stage boundaries with the chain and window, since those are the first things anyone asks.

Naming: use `sales-ingest-pipeline` in docs, runbooks and alerts. The codename `tillstream` survives in a few older places, and when you find one, it is fine to leave it, but do not introduce new uses. If you rename something that carries the old name, check that Airflow, dashboards and alert routing are updated together, or alerts will silently go to nowhere.

## Open questions and things to watch

Some things are not settled. Whether to move the feed-based chains to continuous streaming instead of micro-batches is still debated; the gain is fresher data, the cost is more operational surface and more small files. Whether to keep the quarantine rules chain-specific or push more of them into shared validation is also open. And the handling of deleted rows in replays is still a manual decision, which is a weakness.

Keep an eye on onboarding of new chains, since each one tends to reveal a quirk the common shape does not cover. Keep an eye on reference data freshness, since most apparent pipeline problems turn out to be reference problems. And keep this note current: when you learn how the pipeline really behaves, correct what is written here rather than adding a second note on the same subject.
