---
id: 01KG9XTHTJHQ6C1EWST2HW68EB
created: 2026-01-31T08:43-03:00
sources:
  - "doc: Backfill run log"
---

# influx-rollups backfill timing and what it means for planning

Backfilling 18 months of data took influx-rollups 6 hours 40 minutes on an 8 vCPU InfluxDB 2.7 node. This note records that measurement, what it covered, what we think drives the number, and how to plan around it. It is a timing report, not a design. The numbers are from a single run, so treat them as an order-of-magnitude guide and not a promise.

## What was measured

The run rebuilt every rollup that influx-rollups maintains, starting from raw telemetry and going back 18 months. The raw data is the household solar production and battery readings that GridHaven ingests through MQTT and Azure IoT Hub and writes into InfluxDB. The rollups are the downsampled series that the Julia forecasting code and the Svelte dashboards read, so they never have to scan raw points.

The host was a single node with 8 vCPU running InfluxDB 2.7. Wall-clock time from the first task starting to the last rollup bucket being written was 6 hours 40 minutes. That is end to end for the backfill, including the final verification pass, not only the query time.

```text
influx-rollups backfill
  range:  18 months
  node:   8 vCPU, InfluxDB 2.7
  total:  6 hours 40 minutes
```

The run was done on a quiet node. Live ingest was still going, but no other heavy jobs shared the machine. If another job shares the node, expect the total to grow.

## Why it takes this long

Most of the time goes into reading raw points back out of storage, not into computing the aggregates. A rollup over a long range has to touch every raw point once per rollup definition, and the raw series for a home with several inverters and a battery is dense. Doing this for many installations over 18 months is a lot of sequential scanning.

A few things shaped the result:

- The backfill walks the history in time windows rather than in one query. Smaller windows keep memory flat and make a failure cheap to retry, but they add overhead per window.
- InfluxDB 2.7 runs queries for a task on the cores it has, but a single task does not scale linearly with core count. Having 8 vCPU helped mostly because several windows could be in flight together.
- Writes of rollup points compete with the live ingest path for the same storage. We kept the backfill write rate below what would hurt ingest, which costs some speed.
- Older months are colder on disk, so the first read of them is slower than the recent ones.

We did not profile each factor separately. The list above is our reading of the behaviour, not a measured breakdown.

## What to expect when you rerun it

Use 6 hours 40 minutes as the baseline for a full 18 month backfill on a node like the one above. Scale it roughly in proportion to the range: a shorter backfill should take proportionally less, though there is a fixed startup and verification cost that does not shrink. A bigger fleet of installations makes it longer in about the same proportion.

A weaker node will be slower, and a stronger one will not necessarily be faster. Past a certain point storage throughput, not CPU, is the limit. If you try a bigger machine and see no gain, that is the likely reason.

Do not schedule it inside a window where the node is also serving forecast jobs at full load. The Julia forecasting runs read from the rollups, and they slow down if the backfill is hammering the same disks.

## Operational notes

A few practical points from the run:

- Start the backfill from the oldest end and move forward. Rollups for recent data are the ones people look at, but filling them last means dashboards show gaps for a long time. If you need recent data first, run a short recent backfill ahead of the long one and accept some repeated work.
- The backfill can be stopped and resumed by window. Because rollup writes are idempotent for a given bucket, rerunning a window overwrites the same points and does not duplicate them. That makes retries safe.
- Watch the node for disk and memory pressure during the run. If the node starts to swap or the ingest lag grows, pause the backfill and resume later instead of letting it finish slowly.
- Keep the retention settings of the raw bucket in mind. A backfill can only rebuild what the raw data still holds. If raw retention is shorter than the range you ask for, the older buckets will be empty or partial.
- Check a sample of installations after the run. Compare a rollup total against a direct sum over the raw data for the same period. We did this for a handful of homes and the figures agreed.

## Risks and open questions

This is one measurement. We have not repeated it, so we do not know the run-to-run spread. We also have not tried a version where the backfill is split across several nodes, or where it is split by installation instead of by time.

Things we have not answered:

- Whether a larger window size would cut the total meaningfully without raising memory too much.
- Whether the cost is dominated by the oldest data, which would argue for archiving it differently.
- How much the number changes on a later InfluxDB version. The measurement is specific to InfluxDB 2.7, and a storage engine change could move it either way.
- Whether time-of-use tariff changes force a rebuild of some rollups. If a tariff definition changes how we bucket data, we may need to rerun part of the backfill, and it would be useful to know which rollups are affected so we do not redo all of them.

Until those are answered, plan for the full time and make sure someone owns watching the run.

## Where this matters

The backfill comes up in a few situations. A new installer onboards with a large history to import. A rollup definition changes and old buckets must be recomputed. Or a bug in the aggregation is found and the output needs to be regenerated. In each case the question people ask is how long the data will be unavailable or wrong, and the answer is the figure above, adjusted for range and load.

For installers and their customers the visible effect is on the forecast and charging schedule. While rollups are being rebuilt, the scheduler should keep using the last good rollups and not read half-filled ones. Make sure the cutover to the rebuilt data happens only after the verification pass finishes. If the schedule runs on partial data it can pick a bad charging plan against the tariff, which is worse than using slightly stale data.

If you change influx-rollups in a way that forces a full rebuild, say so in the change description and give the expected duration, so whoever runs it can plan the maintenance window. Update this note if a later run gives a different time, and record the node size and InfluxDB version next to the new figure so the comparison stays honest.
