---
id: 01KNKBHQ26ED7PMRR8T65EG5YT
created: 2026-04-07T03:54-03:00
---

# latency_rollup_mv backfill plan

This note replaces the earlier note about "latency rollup backfill last", and the new value is 90 days: backfill latency_rollup_mv for the last 90 days, still run in daily chunks. The earlier plan of 30 days is dropped.

## Scope

The backfill fills latency_rollup_mv in ClickHouse with history from the raw trace data that OpenTelemetry already collected. The goal is that the post-deploy latency regression summaries have a long enough baseline to compare against. Grafana panels that read latency_rollup_mv should show the full window once this is done.

## Window

The window is the last 90 days, counted back from the day the backfill starts. Daily chunks stay as the unit of work. One chunk covers one calendar day, so a failure only costs that day.

## Why the longer window

A short baseline made some regressions look normal, because slow periods sat inside the comparison range. SREs need to see more deploy cycles in the history. The longer range gives that without changing how the chunks run.

## Chunking

Each chunk is one day of source rows inserted into the target of latency_rollup_mv. Run chunks in order, oldest first, and wait for each to finish before the next starts. Keep the insert size modest so live ingestion is not starved.

```sql
-- one daily chunk, repeated for each day in the 90 days
INSERT INTO latency_rollup_mv
SELECT ... WHERE day = <chunk day>
```

## Running it

Run the loop from a job on Kubernetes, not from a laptop. The job should log the day it is on, so a restart can resume from the last finished day. Java services keep writing during the backfill, so the chunks must not touch the current day until the end.

## Checks after each chunk

- Row count for the day is non-zero for days that had traffic.
- No duplicate rows for the day in latency_rollup_mv.
- Percentile values look sane against a Grafana panel for the same day.

## Re-running a day

If a chunk fails or looks wrong, drop that day's partition data from the target and run the chunk again. Do not re-run a day without clearing it first, or the rollup double counts.

## Open items

- Confirm the ClickHouse disk has room for the extra history.
- Pick a quiet time window for the job.
- Tell the SREs when the 90 days are complete so they can rely on the baseline.

## Status

Not started. Nothing has been backfilled yet under this plan.
