---
id: 01KMH61TKWAPN4AYY6CJBN9VZV
created: 2026-03-24T21:24-03:00
---

# influx-rollups backfill time with parallel windows

This note replaces the earlier note about "influx rollups backfilling months". The new value: after enabling parallel windows, the same backfill of influx-rollups took 2 hours 15 minutes, and that figure replaces the earlier backfill time. Anything in the old note that gives a longer duration should be treated as out of date.

The rest of this note is context so a reader who has only this page can tell what was measured, what changed, and what to be careful about. It is written quickly and records what is known, not a full design.

## What was measured

The measurement is the wall-clock time of a backfill of influx-rollups over the same span of history that the earlier note used. The point of keeping the span the same was to make the comparison fair: the only intended difference between the old run and the new one is that parallel windows were switched on. The new total is 2 hours 15 minutes.

The earlier figure is not repeated here on purpose. It was superseded, and carrying it forward in two places is how stale numbers survive. If someone needs the old value for a historical comparison, they should look at the old note's history, not rely on memory.

The backfill in question is the one that rebuilds the downsampled series that GridHaven reads when it forecasts household solar output and when it plans battery charging against time-of-use tariffs. It reads raw points that came in over MQTT and were stored in InfluxDB, and it writes the aggregated results back into InfluxDB as rollups. The Julia forecasting code and the Svelte dashboards consume those rollups rather than the raw points.

## What changed: parallel windows

Before, the backfill walked through the time range one window after another. Each window had to finish before the next one started. That made the run time roughly the sum of every window's time, and the machine spent a lot of it waiting on InfluxDB queries and writes rather than computing.

With parallel windows enabled, several time windows are processed at once. The windows do not overlap in time, so each rollup bucket is still produced by exactly one window. That matters because a bucket split across two workers would be written twice or written partially. The window boundaries are aligned to the rollup bucket size for this reason.

The speedup is not linear in the number of workers. InfluxDB becomes the shared resource, and past some point extra workers only add contention on reads and on write batching. The observed total of 2 hours 15 minutes is what we got on our setup, not a guarantee for other installs or other data volumes.

## Conditions that affect the number

A backfill time is only comparable if the conditions match. These are the things that moved the number in the past or are likely to:

- The amount of raw data in the span. Installers with many sites or high-frequency meters produce more points per day, so the same calendar span costs more.
- Whether live ingestion from MQTT and Azure IoT Hub was running during the backfill. Live writes compete with backfill writes, and a busy evening ingest window slows things down.
- How warm the InfluxDB caches were. A first run after a restart is slower than a repeat run.
- The size of the write batches and the retry behaviour when InfluxDB pushes back.
- Other jobs on the same host, such as forecast training in Julia.

The 2 hours 15 minutes figure was taken as a single run of the same backfill as before. It is a data point, not an average. If the number is going to be used for planning, repeat the run at least once under similar load before treating it as stable.

## Correctness checks after a parallel backfill

Faster is only useful if the output is the same. After switching to parallel windows, the things worth checking are these.

First, bucket counts. For a sample of sites, the number of rollup points per day should match what a sequential run produces. Gaps at window boundaries are the most likely failure, so look at the buckets right at the edges between windows.

Second, values at the edges. A bucket that straddles a boundary can get a wrong mean or sum if a window cut it short. Aligning windows to bucket size is meant to prevent that, but it is cheap to spot-check a few boundary buckets by hand.

Third, duplicates. InfluxDB overwrites a point that has the same measurement, tag set and timestamp, so a double write usually shows up as a changed value rather than an extra row. If a re-run changes values it should not change, suspect overlapping windows.

Fourth, the downstream consumers. The forecasts and the charge schedules should not shift in any visible way because of a backfill that only rebuilds history. If a schedule changes after a backfill, compare the rollups before and after for the affected site before blaming the scheduler.

## Operational notes

Run the backfill when live ingest is quiet if you can. It is safe to run during live traffic, but it will be slower and it will add load that installers' dashboards can feel. Dashboards read the same rollups, so a half-finished backfill can show a partly rebuilt history. For long spans, prefer to tell installers before starting.

If a run fails partway, it can be restarted. Because every bucket is produced by exactly one window and writes overwrite by timestamp, re-running a window is idempotent. Restart from the failed window instead of from the beginning, and keep an eye on the window that was in flight when it failed, since it may have been partly written.

Watch memory on the host. Parallel windows hold several result sets at once. If the host starts swapping, the backfill will get slower than the sequential version, which would make the new number meaningless. A run that is much slower than 2 hours 15 minutes for the same span is a sign that something is wrong with the host or with InfluxDB, not a sign that parallel windows stopped working.

## Open questions and next steps

Things we have not settled:

- Whether the number of parallel windows should be tuned per install instead of using one default. The right value probably depends on how much headroom the InfluxDB instance has.
- Whether a backfill should pause itself when live ingest lag grows, so that it never hurts forecasting freshness.
- Whether to record the backfill time automatically at the end of each run, so the next note does not need a person to time it.

Until those are answered, treat 2 hours 15 minutes as the current reference time for the backfill of influx-rollups with parallel windows enabled, and update this note, not the old one, when it changes again.
