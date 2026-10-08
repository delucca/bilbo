---
id: 01KF5VT19NYCFWFZSTH0HFPT9N
created: 2026-01-17T08:35-03:00
sources:
  - "code: dags/shelfsense_nightly.py"
---

# shelfsense-dags reference

This is a quick reference for shelfsense-dags, the Airflow side of ShelfSense. It covers where the nightly pipeline lives, how to start it by hand, and what to look at when it misbehaves. It is written from memory of how the project is laid out, so check details against the repo before relying on them.

## The nightly DAG file and how to run it

The nightly DAG file is `dags/shelfsense_nightly.py`. A run of that DAG is started by hand with `airflow dags trigger shelfsense_nightly`. The DAG id is shelfsense_nightly, the same as the file stem, and the trigger command takes that id, not the file path.

```
airflow dags trigger shelfsense_nightly
```

Use the trigger command for reruns after a fix, for backfilling a missed night, and for checking a change on a dev Airflow before it goes out. The scheduler also starts the DAG on its normal nightly schedule, so a manual trigger is an extra run on top of that.

## What the project does

ShelfSense predicts store-level stockouts and generates replenishment orders. Merchandising analysts at grocery chains read the output. shelfsense-dags does not hold the prediction logic itself. It orders the work: ingest, feature build, scoring, order generation, and publishing.

## Where the pieces run

The heavy work is Apache Spark jobs written in Scala. Tables are stored in Delta Lake. Snowflake is the serving side, where analysts and downstream tools read the results. Airflow only schedules and watches these steps. Keep real data processing out of the DAG file.

## Task layout in the nightly DAG

The DAG is a chain with a few fan-outs. Early tasks load the day's sales and inventory snapshots. Middle tasks build features and score each store and item. Late tasks write replenishment orders and push tables to Snowflake. Names in the Airflow UI follow the stage they belong to, so scan the graph view left to right to find where a run stopped.

## Ordering and dependencies

Scoring must not start until feature tables for the night are complete. Order generation must not start until scoring is complete. Publishing to Snowflake comes last. If you add a task, put it into this chain explicitly rather than leaving it floating, since a floating task can run before its inputs exist.

## Retries and failures

Spark steps get a small number of retries with a delay between them. Most transient failures are cluster or network blips and pass on retry. If a task fails after its retries, clear that task and its downstream tasks in the UI rather than rerunning the whole DAG, unless the inputs changed.

## Rerunning and backfills

For a missed night, trigger the DAG with the same command as above and make sure the run uses the right logical date. Spark jobs should be idempotent on Delta tables, so a rerun for the same date overwrites that date's partition instead of appending. If you see duplicated rows after a rerun, suspect a job that appends and not the DAG.

## Snowflake publishing

The final step loads results into Snowflake. Failures here are usually credentials, warehouse availability, or a schema change on the target table. Fix the cause, then clear only the publish task. Do not rerun scoring just to republish.

## Making changes safely

Edit the DAG file, import it locally to catch syntax and cycle errors, and try it on a dev Airflow before merging. Keep top-level code in the DAG file cheap, because the scheduler parses it often. Avoid network calls at import time.

## Common gotchas

A manual trigger does not replace the scheduled run, so two runs can overlap if you trigger near schedule time. Check that nothing else is writing the same Delta partitions. Another trap is renaming the DAG id: history and run state attach to the id, so a rename looks like a brand new DAG.

## Open items

Worth writing down later: the exact schedule and timezone, the retry counts per task, and who owns on-call for the nightly run. None of these are recorded here yet.
