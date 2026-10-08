---
id: 01K4AFXGVGJGN9860CVFP303TN
created: 2025-09-04T10:51-03:00
---

# shelfsense-dags: general orchestration direction

We settled on keeping shelfsense-dags thin. The DAGs schedule and order work; they do not hold business logic. Spark jobs written in Scala do the real computation, and Airflow only triggers them, waits, and reports. This is the direction, not a spec. Related: [[shelfsense-dags-nightly-file]].

## Why we chose this

Earlier DAGs had transformation code mixed into Python tasks. It was hard to test and hard to rerun. Moving logic into Spark jobs means one place to test and one place to fix. Analysts care that stockout predictions and replenishment orders show up reliably, so predictable reruns matter more than clever scheduling.

## Scope of a DAG

Each DAG covers one business flow: refresh inputs, score stockout risk, produce replenishment orders, publish. We avoid one giant DAG. Smaller DAGs fail in smaller ways and are easier to reason about at 3am.

## Task design

Tasks should be idempotent. Rerunning a task for the same logical period should give the same result, not duplicate rows. Delta Lake table writes lean on merge or partition overwrite for this, not blind appends.

## Data handoff

Tasks pass references to data, not data. Delta tables are the handoff point between steps. Airflow metadata stays small. Snowflake is the serving side for analysts, loaded at the end of a flow, not read from mid-pipeline unless there is no alternative.

## Scheduling

Prefer data-aware triggers over fixed clock guesses where we can. When a fixed schedule is unavoidable, add a sensor or a check that the upstream data actually arrived before starting heavy work.

## Failure handling

Retries are allowed for transient problems, kept modest. Anything that keeps failing should stop and alert a person rather than loop. Partial output must not reach analysts: publish only after validation passes.

## Backfills

Backfills use the same DAGs with a period parameter, not separate scripts. If a backfill needs special handling, that is a sign the DAG design is wrong and we fix the DAG.

## Config and environments

Environment differences live in configuration, not in forked DAG code. Secrets stay in the Airflow secrets backend, never in DAG files.

## Open items

Still undecided: how much validation belongs in Spark versus in a separate Airflow task, and how to surface late-data warnings to analysts. Revisit once the current flows have run for a while.
