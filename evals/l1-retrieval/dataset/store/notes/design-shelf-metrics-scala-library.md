---
id: 01KDCES2JXJJYS77E54PZ2GZ6C
created: 2025-12-26T01:30-03:00
sources:
  - "code: build.sbt"
---

# shelf-metrics-lib design

shelf-metrics-lib is a Scala library that computes two store-level metrics for ShelfSense: `fill_rate` and `days_of_supply`. It is published as `com.shelfsense:shelf-metrics_2.12:1.4.0`, built for Scala 2.12. Anything in ShelfSense that needs those two numbers should take them from this library and not recompute them locally.

## Purpose

ShelfSense predicts store-level stockouts and generates replenishment orders for merchandising analysts at grocery chains. Both the stockout prediction and the replenishment logic need the same definition of how well a shelf was stocked and how long the stock will last. shelf-metrics-lib is the one place those definitions live, so the Spark jobs, the Airflow-scheduled batch runs and the Snowflake-side reporting all agree on the numbers analysts see.

## What it computes

- `fill_rate`: how much of the demand at a store and item level was met from shelf stock over a period. It is a ratio, so it stays comparable across stores of different sizes.
- `days_of_supply`: how many days current stock would cover at the recent rate of demand. Replenishment ordering uses it to decide when a store runs short.

Both are computed per store and per item, then can be rolled up by the caller. The library does not decide thresholds or order quantities; it only produces the metrics.

## How it is used

The library is a plain dependency of the Spark jobs, which read inventory and sales data from Delta Lake tables and apply the metric functions to DataFrames or Datasets. It has no scheduling logic of its own. Airflow runs the jobs that call it, and the results are loaded to Snowflake for analysts.

Because it is a Scala 2.12 artifact, any consumer has to run on a Spark build that uses Scala 2.12. Mixing in a different Scala binary version fails at link time, so check that first when a job cannot find the library's classes.

Dependency line for sbt:

```scala
libraryDependencies += "com.shelfsense" % "shelf-metrics_2.12" % "1.4.0"
```

## Things to keep in mind

- Keep the metric code free of I/O. It takes data in and returns data out, so it can be tested without a cluster.
- Division by zero needs care: items with no demand or no stock history should give a defined result and not a crash or a silent null spread across a rollup.
- A change to how `fill_rate` or `days_of_supply` is defined changes numbers analysts already know. Treat it as a version bump and tell the consumers of the library, not a quiet fix.
- Pin the version in each consuming job. Upgrading is a deliberate step, done job by job.

## Open points

- Document the exact handling of missing inventory snapshots in the library's own docs; this note only covers the shape of the design.
- Decide who owns the release process for new versions of the artifact.
