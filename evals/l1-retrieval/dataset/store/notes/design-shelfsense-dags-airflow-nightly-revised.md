---
id: 01KZMS2C400EDQW4F96SVZ8ZHT
created: 2026-08-09T23:47-03:00
---

# shelfsense-dags nightly schedule

The `shelfsense_nightly` DAG in shelfsense-dags now runs on cron `0 4 * * *`, replacing the earlier schedule, so that late POS files land before ingest starts. This note replaces the earlier note about "shelfsense dags airflow nightly"; the new value is `0 4 * * *`.

## Summary of the change

The schedule of `shelfsense_nightly` moved to `0 4 * * *`, which means 04:00 every day. The old schedule fired too early. POS files from some stores arrived after ingest had already started, so those stores were missing from the night's data. With the new cron, the late files are in place before ingest begins.

## Why the old schedule failed

Some grocery chains send their POS exports late. Ingest started on the old schedule and read whatever had landed by then. Late files were then picked up only the next night, a full day behind. That made stockout predictions for those stores stale, and the replenishment orders built from them were off.

## What the DAG does

`shelfsense_nightly` is the main nightly pipeline in shelfsense-dags. It ingests POS data, runs the Spark jobs written in Scala, writes results to Delta Lake, and loads the output into Snowflake for the merchandising analysts. Order generation depends on the earlier steps finishing.

## The new value

```python
# shelfsense-dags: shelfsense_nightly
schedule = "0 4 * * *"
```

The DAG id is unchanged. Only the cron expression changed.

## Reading the cron

Five fields: minute, hour, day of month, month, day of week. Here minute is 0, hour is 4, and the rest are wildcards. It runs daily with no day-of-week restriction.

## Timezone

Airflow evaluates the cron in the timezone configured for the DAG or the deployment. Check which one applies before assuming 04:00 means local store time. This note does not record that setting.

## Downstream timing

Moving the start later moves the finish later. Analysts expect orders in the morning, so watch that the full run still completes in time. If runtimes grow, the fix is in the Spark jobs, not a return to the earlier hour.

## Late files still possible

The new time reduces the problem but does not remove it. A file that lands after 04:00 is still missed by that night's run. If it keeps happening, add a file-arrival check before ingest rather than moving the cron again.

## Catchup and backfill

Changing the schedule can confuse the Airflow run history. Confirm that catchup behaviour is what you want, so the change does not trigger a burst of extra runs. Backfill missed days by hand when needed.

## Interaction with other DAGs

Anything that waits on `shelfsense_nightly` by time rather than by dependency will now start too early. Check such DAGs in shelfsense-dags and switch them to a real dependency.

## Monitoring

Watch the first few runs after the change. Look at when ingest starts compared to when the last POS file arrives, and at total run time.

## Rollback

To roll back, restore the earlier cron in the DAG definition and redeploy. The earlier value is not recorded here; check version history. Rolling back brings the late-file problem back.

## Open questions

- Does the 04:00 start leave enough margin for the slowest chains?
- Should ingest wait for files instead of relying on the clock?

## Status

Done in shelfsense-dags. The earlier note on the nightly DAG schedule is superseded by this one.
