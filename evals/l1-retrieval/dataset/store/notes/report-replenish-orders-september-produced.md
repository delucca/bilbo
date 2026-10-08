---
id: 01KDZ9PJBFN9W8B54ZH3E73ZAE
created: 2026-01-02T09:07-03:00
---

# replenish-orders-job September throughput and runtime

This note records the September volume and runtime of the replenish-orders-job in ShelfSense. In September the job produced 182,400 order lines per night, and a nightly run took 14 minutes. Use these two figures as the baseline when judging whether a later run looks slow or thin.

## What the job does

The replenish-orders-job takes the store-level stockout predictions and turns them into replenishment orders. It is written in Scala and runs on Apache Spark. Inputs and outputs live in Delta Lake tables, and Airflow schedules it as a nightly task. Merchandising analysts at the grocery chains read the resulting orders, usually downstream in Snowflake, so a late or short run shows up in their morning review.

## September numbers

- Order lines per night: 182,400
- Runtime per night: 14 minutes

These are the September figures for the replenish-orders-job as a whole, not for a single store or chain. The volume is the count of order lines written per night, not the count of orders. One order can hold many lines, so do not compare it with an order count from another report.

## Why the baseline matters

A run of 14 minutes leaves plenty of room in the nightly window. If the runtime grows while the line count stays near 182,400, suspect the Spark side: skew on a few large stores, small files in the Delta tables, or a changed shuffle size. If the line count falls well below 182,400 and the runtime falls with it, suspect the inputs. The usual cause is an upstream prediction table that arrived late or partial, and the job still succeeds on what it found.

## Checking a night against the baseline

Compare the night's output with the September figures before opening a ticket. A quick reading of the job's own summary is enough. The sample below is only the September baseline written out, not output from a real run.

```text
replenish-orders-job
  order lines per night: 182,400
  runtime: 14 minutes
```

If the new numbers sit close to this, the night is normal. A large gap in either direction is worth a look at the Airflow task history and the Delta table versions the job read.

## Caveats

- The figures are from September only. Seasonal peaks, promotions and holidays will move the volume, so a rise at those times is not by itself a fault.
- The note does not say how many stores or chains were in scope that month. If the store list changed, the baseline shifts with it.
- Runtime depends on cluster size and on load from other jobs sharing it. A slower night on a busy cluster is not proof of a code problem.
- No per-stage timing was recorded here. If you need it, take it from the Spark UI for a fresh run.

## Open items

- Record the October volume and runtime next to the September figures so the trend is visible.
- Note the cluster configuration used for the 14 minutes run, since the figure means little without it.
- Decide on an alert threshold for line count and for runtime, based on this baseline and a few more months of data.
