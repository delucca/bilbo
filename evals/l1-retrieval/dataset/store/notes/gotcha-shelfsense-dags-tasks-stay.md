---
id: 01KF5XCM37E403EPVS2G8B3X77
created: 2026-01-17T09:02-03:00
---

# shelfsense-dags: tasks stuck in queued when spark_pool is full

If tasks in shelfsense-dags sit in the `queued` state and never start, look at the Airflow pool `spark_pool` first. When every slot of `spark_pool` is held by stuck runs, no new task that needs the pool can get a slot, so it stays `queued` forever. Nothing fails and nothing times out, which is why it is easy to miss.

## Symptom

A DAG run in shelfsense-dags looks alive but makes no progress. Downstream tasks show `queued` in the UI, with no start time and no logs, because the worker never picked them up. Other DAGs that do not use the pool keep running normally, so the scheduler itself looks healthy. For analysts this shows up as stale stockout predictions and replenishment orders that were not generated on time.

## Why it happens

Spark-related tasks in shelfsense-dags are assigned to `spark_pool` to cap how many Spark jobs hit the cluster at once. A slot is only released when the task ends. If a run hangs (a Spark job that never finishes, a lost executor, a sensor waiting on data that never lands), its task keeps the slot. Enough hung runs fill the whole pool. After that, each new scheduled run adds more `queued` tasks behind them, and the backlog grows on its own.

## How to confirm

Open the pool view in the Airflow UI and compare running slots with the pool size. If running plus queued slots fill the pool and the running tasks are very old, that is the cause. Then check the oldest running tasks in shelfsense-dags and see whether the matching Spark application is still doing work or is just sitting there.

## Fix

Clear the stuck runs: mark the hung task instances failed, or kill the Spark application behind them, so the slots free up. The queued tasks then start by themselves, usually in order. Do not just raise the pool size to push through. That hides the hung runs and lets them pile up against the cluster.

## Prevention

- Set an execution timeout on Spark tasks in shelfsense-dags, so a hung run fails and gives its slot back.
- Alert on pool occupancy staying full for a long time, and on tasks in `queued` for much longer than normal.
- Avoid long-lived sensors that hold a `spark_pool` slot while they wait. Use deferrable or reschedule mode instead.
- After any cluster outage, check the pool for runs that were orphaned and still hold slots.
