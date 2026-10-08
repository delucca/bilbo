---
id: 01K46N41YC6EY7YCW9C9VXFYDE
created: 2025-09-02T23:05-03:00
sources:
  - "doc: DAG Review Notes"
---

# Review of shelfsense-dags: catchup and backfill storms

This is a review note on shelfsense-dags, the Airflow component of ShelfSense that schedules the jobs behind store-level stockout prediction and replenishment order generation. The main conclusion: `catchup=False` must be set on every DAG, so that an outage does not trigger a backfill storm when the scheduler comes back.

## Naming

flowmaster was the previous name of shelfsense-dags. The component is called shelfsense-dags now. Old branches, dashboards, runbooks, chat threads and some log lines may still say `flowmaster`. They all mean shelfsense-dags. When searching for history, search for both names. In new notes and code comments, use `shelfsense-dags` only.

## Scope of the review

The review looked at how the DAGs in shelfsense-dags behave after the scheduler or the workers have been down for a while. It did not look at the Spark job logic itself, the Delta Lake table layouts, or the Snowflake models. It only looked at the scheduling layer: schedules, start dates, catchup settings, concurrency limits and retries.

## Main conclusion

`catchup=False` must be set on every DAG in shelfsense-dags. This is a rule, not a preference. Any DAG without it is a defect and should be fixed in review. New DAGs must not be merged without it.

## Why it matters

With catchup on, Airflow creates a run for every schedule interval missed between the last successful run and now. After an outage of any length, the scheduler then queues a large number of runs at once. That is the backfill storm.

In ShelfSense the storm is costly for a few reasons:

- Each run launches Spark work, so a pile of runs competes for the same cluster capacity.
- Runs write to Delta Lake tables, and many overlapping runs for different intervals can contend on the same tables.
- Snowflake loads and queries multiply, which adds warehouse load nobody planned for.
- Analysts need the current forecast, not a stale one for every missed interval.

## What a storm looks like

The first sign is a sudden jump in queued and running DAG runs right after the scheduler recovers. Task slots fill up. Fresh runs for the current interval wait behind old ones. Replenishment orders for today can be late because the queue is full of work for intervals that no longer matter.

## Why skipping missed intervals is fine here

Stockout prediction and replenishment are about the current state of the shelf. The jobs read the latest inventory and sales data, so one run after the outage covers what the missed runs would have covered. Running each missed interval separately adds load without adding value. Where a job really needs a past interval, it should be run by hand on purpose.

## Review checklist

When reviewing a DAG in shelfsense-dags, check these things:

- `catchup=False` is set explicitly on the DAG object, not left to the global default.
- The start date is fixed and sensible, not computed from the current time.
- Concurrency limits for runs and tasks are set, so a burst cannot take the whole cluster.
- Retries are bounded and have a delay.
- Tasks that write data are safe to run twice.

## Explicit setting, not defaults

Relying on a deployment-wide default is not enough. A default can be changed, or may differ between environments, and a DAG file read on its own would then tell you nothing. Setting `catchup=False` in each DAG makes the behavior visible in the code and the same everywhere.

## Manual backfills

When a past interval must be rebuilt, do it deliberately: pick the range, run it with limited parallelism, and watch cluster and warehouse load. Do not turn catchup back on for this. A manual backfill is a decision a person makes. A storm after an outage is an accident.

## Recovery after an outage

After an outage, the scheduler should produce one run for the current interval per DAG. If an analyst needs a missed interval, someone triggers it by hand. Check that the current run finished before declaring the system healthy.

## Open points

- Add an automated check that fails CI when a DAG does not set `catchup=False`.
- Look again at concurrency limits per DAG, since the review focused on catchup.
- Make sure old `flowmaster` references in docs and alerts are updated to `shelfsense-dags`.

## Follow-up

Treat the rule as settled. The remaining work is enforcement: a lint or test over the DAG folder, and a short mention in the contributor guide so new DAG authors see it before their first review.
