---
id: 01KTYBK01YX7JKFBCQSY12NPSS
created: 2026-06-12T13:45-03:00
---

# shelf-metrics-lib: next steps

Working plan for shelf-metrics-lib, the shared Scala library that the Spark jobs in ShelfSense use to compute store-level shelf metrics. Nothing here is a fixed decision. It is the order I would pick things up in, written fast, to be corrected as we learn more.

## Goal

Make shelf-metrics-lib the one place where metric definitions live, so the stockout model, the replenishment order generation and the analyst-facing reports all agree on what a metric means. Right now the risk is drift: the same idea computed slightly differently in several jobs. The plan below is about removing that drift and making the library easier to change safely.

## Current state, in general terms

The library holds metric calculations over Spark DataFrames, reading from and writing to Delta Lake tables. Airflow schedules the jobs that call it, and some outputs end up in Snowflake for the analysts. Coverage of edge cases is uneven. Some metrics have good tests, others have almost none. Documentation is thin and mostly lives in people's heads.

## Inventory the metrics

First step: list every metric the library exposes and every place that computes a similar value outside it. For each one, write down the meaning, the inputs, the grain (store, item, day, and so on) and who consumes it. A plain table in the repo docs is enough. Do not start refactoring before this exists.

## Find duplicated logic

Compare the inventory against the jobs. Flag the metrics that are re-implemented in job code or in Snowflake views. Rank them by how many consumers they have and by how likely they are to disagree. The top of that list gets moved into the library first.

## Settle definitions with analysts

Some definitions are ambiguous, for example what counts as out of stock when a store is closed, or how to treat partial-day shelf gaps. Those need an answer from the merchandising analysts, not from us. Collect the open questions in one list, get answers, and record each answer next to the metric. Keep this note free of the answers until they are confirmed.

## API shape

Look at the public surface of the library. Aim for small, composable functions over DataFrames and a few typed config objects, not long parameter lists. Mark what is public and what is internal. Anything internal can change freely; anything public needs a deprecation path.

## Input contracts

Each metric should state what columns and types it expects, and fail early with a clear message when the input does not match. Add a light validation layer at the entry points. Avoid checking every row; check the schema and a few cheap invariants.

## Null and missing data handling

Decide one consistent policy for missing inventory snapshots, late-arriving data and null sales. Write it down once and apply it everywhere in the library. This is where most disagreements between jobs probably come from, so give it real time.

## Testing

Build a test suite around small local Spark sessions with hand-made fixtures. Cover the normal case, empty input, a single store, duplicated rows, and the missing-data policy. Add a few property-style checks where a metric has an obvious invariant, such as values never going negative.

## Golden datasets

Keep a small set of reference inputs with expected outputs, reviewed by an analyst. Changes to a metric must show up as a diff against these. This makes definition changes visible instead of silent.

## Performance review

Profile the heavier metrics on realistic data. Look for wide shuffles, repeated scans of the same Delta table and window functions that could be simplified. Fix only what shows up in profiling, not what looks suspicious.

## Delta Lake interaction

Check how the library reads tables: partition pruning, time travel use and handling of schema changes. Keep table access out of the metric functions where possible, so the calculations take DataFrames and the jobs own the I/O. That also helps testing.

## Snowflake alignment

Review where Snowflake-side logic overlaps with the library. Either have Snowflake consume the library output, or document clearly which side owns each metric. Avoid a third copy of any definition.

## Airflow integration

Look at how DAGs call the library and how failures surface. Make sure a metric failure produces a message an on-call person can act on. Check that retries do not double-write outputs.

## Versioning and release

Set up a simple, predictable way to publish the library and to let jobs upgrade one at a time. Keep a short changelog that says in plain words what changed in a metric's meaning, not only in its code.

## Migration of jobs

After the library is trusted, move jobs over in order of risk, lowest first. For each job, compare old and new outputs side by side for a while before removing the old code. Keep the comparison itself as a small reusable helper.

A sketch of the comparison helper's shape, using only names from this project:

```scala
// compare shelf-metrics-lib output with a job's legacy output
val diff = legacyDf.exceptAll(libDf)
```

## Documentation

Write short docs per metric: meaning, inputs, grain, known limits. Add one worked example per area. Put it in the repo so it moves with the code.

## Risks and open questions

- Analysts may disagree on definitions, which would stall the cleanup.
- Moving a metric can change numbers that people already trust, so communicate before releasing.
- Test fixtures can go stale if nobody owns them.
- Unclear who owns the Snowflake side.

## Order of work

Inventory first, then definitions and the missing-data policy, then tests and golden datasets, then API cleanup, then performance and migration. Documentation runs alongside all of it. Revisit this order after the inventory, since it may change what is urgent.
