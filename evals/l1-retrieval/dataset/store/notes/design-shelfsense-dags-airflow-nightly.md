---
id: 01K18W5EVQRQ3611JRGKVXDHDJ
created: 2025-07-28T12:59-03:00
sources:
  - "doc: ShelfSense Orchestration Overview"
---

# shelfsense-dags nightly DAG design

The nightly pipeline in shelfsense-dags is a single Airflow DAG, `shelfsense_nightly`. It runs on cron `0 3 * * *`, so it starts once a day at 03:00 in the scheduler's timezone. Five tasks run in sequence: ingest, features, score, orders and export. No branching, no parallel fan-out. Each task starts only after the previous one succeeds. This note says why it is shaped that way and what each step is for. It is a design note, not a runbook. See [[shelfsense-dags-review-concluded]] for the review that looked at this layout.

## Why one linear DAG

The stages depend on each other's output. Features need the freshly ingested sales and inventory snapshots. Scoring needs the features. Orders need the stockout scores. Export needs the final order set. A linear chain matches that data flow, and it keeps the failure story simple: when a task fails, everything after it is skipped, and the analysts see yesterday's orders instead of a half-updated set.

We considered splitting the DAG per chain or per region so one slow chain would not hold up the others. We did not do it. The Spark jobs already partition by store, so parallelism lives inside the jobs and not in the DAG. Splitting would multiply DAG runs and make it harder to answer "did last night's run finish?"

## Task by task

### ingest

Pulls the day's point-of-sale, inventory and delivery data and lands it in Delta Lake tables. This is the only task that touches the source feeds. It should be idempotent for a given run date, so a rerun overwrites the same partition and does not append duplicates.

### features

A Spark job written in Scala that builds the per-store, per-item features from the Delta tables: recent sales velocity, days of cover, promo flags, delivery lateness and similar. It reads only what ingest wrote and writes a feature table keyed by run date.

### score

Applies the stockout model to the feature table and writes a probability of stockout per store and item. Model version is read from configuration, not hardcoded in the DAG, so changing a model does not mean changing the schedule or task graph.

### orders

Turns scores into replenishment order proposals. It applies case-pack rounding, minimum order rules and supplier constraints. This is where business rules live. Keep them out of the scoring step so analysts can reason about "what the model thinks" separately from "what we would order."

### export

Writes the final orders to Snowflake, where the merchandising analysts read them. Export is last on purpose: nothing reaches Snowflake unless every upstream step succeeded.

## Scheduling choice

The schedule leaves time for stores' end-of-day data to arrive before ingest begins and for the whole chain to finish before analysts start work. If upstream feeds start arriving later, the first thing to move is the cron, not the task order. Do not add sensors inside the tasks to wait for late data without discussing it, because a stuck sensor holds the whole chain.

## Retries and reruns

Tasks retry a small number of times with a delay, since most failures are transient cluster or network problems. For a manual rerun, clear the failed task and let the downstream tasks follow. Because each task writes by run date, rerunning a past date should reproduce that date's output. Check this assumption whenever a task is changed.

## Open points

- Alerting is per task failure only. There is no check that export produced a sensible number of orders.
- No data quality gate sits between ingest and features. A bad feed flows through until scoring looks odd.
- Backfills are done by hand by clearing runs. A dedicated backfill path has not been designed.
