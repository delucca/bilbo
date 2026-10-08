---
id: 01M1XZANBKSQGTVH6HPB1XRSP1
created: 2026-09-07T10:01-03:00
---

# Gotchas when changing latency_rollup_mv

These are the things that bite when someone edits latency_rollup_mv. Most of them come from how ClickHouse materialized views behave, plus how the regression summaries downstream depend on the rollup staying stable. Read this before touching the definition, the source table, or the target table. It is general on purpose; check the live schema for specifics.

## What the component is for

latency_rollup_mv turns raw span data into per-service, per-operation latency aggregates. The deploy regression summaries read those aggregates and compare the window after a deploy with a window before it. SREs look at the result in Grafana, so a wrong rollup does not crash anything. It produces plausible but wrong charts, and people act on them. That is the main risk: silent wrongness.

Treat the view as part of a pipeline: OpenTelemetry exporters, the collector, the raw spans table, this view, the target table, then the summary queries and dashboards. A change in any link can break the others without an error.

## A materialized view is an insert trigger

The view only sees blocks inserted into its source table after it was created. It does not recompute history, and it does not react to updates, deletes, or mutations on the source. If you change the SELECT, old rows in the target stay as they were produced by the old logic. New rows follow the new logic. The target ends up with two regimes mixed in one table, and nobody can tell from the data alone where the line is.

Write down, in the change description, the point at which the new logic starts. Without it, a regression summary spanning that point will compare unlike things.

## Changing the SELECT does not backfill

Altering or recreating the view does not fix existing data. If the change matters for past periods, you need a separate backfill into the target, done deliberately. Think about these before running one:

- Backfill inserts can be picked up by the view itself if it still reads the source, which double counts. Do the backfill into a staging table or with the view detached.
- Live inserts keep arriving during the backfill. Decide how the overlap is handled so neither a gap nor a duplicate appears at the boundary.
- A backfill over a wide range is heavy on the cluster. Do it in slices and watch merges and memory.

## Aggregate state and finalization

If the target stores partial aggregate states, then the column types, the aggregate function names, and their parameters are part of the on-disk format. Changing a quantile function, its parameters, or the argument type makes old and new states incompatible, or merges them into nonsense without complaint. Do not swap one quantile variant for another casually; they differ in accuracy, memory, and determinism.

Reads must use the matching merge form. A query that forgets the merge step, or uses the wrong one, returns numbers that look reasonable and are wrong. When you add an aggregate column, check every reader.

## Quantiles are not additive

Never average percentiles across services, buckets, or time windows. The rollup exists to avoid that, by keeping mergeable states. Any new summary query or dashboard panel that takes a finished percentile column and averages it will be misleading. If someone asks for a coarser time bucket, derive it by merging states, not by averaging results.

Also be careful with the difference between mean latency and tail latency. A regression can show up in the tail while the mean barely moves.

## Granularity and the time bucket

The time bucket expression defines what one row means. Changing its size, its alignment, or the timezone handling changes what every downstream comparison means. Alignment matters more than it looks: if buckets are cut differently than before, a deploy marker can land in the middle of a bucket and the before and after windows then share a bucket.

Prefer making the bucket a deliberate, documented property. If you need a different resolution, add a second view or a second target rather than reshaping this one.

## Which timestamp is used

Spans have a start time, an end time, and an ingestion time. The rollup should key on one of them consistently. Late arriving spans, clock skew between nodes, and batching in the collector all push spans into buckets that were already read. If you change which timestamp is used, or add a filter on ingestion lag, expect the numbers for recent windows to move. Think about how late data is handled: it lands in an old bucket as an extra small row, which is fine for mergeable states but surprising for anything that expects one row per key.

## Grouping keys and cardinality

Every column added to the GROUP BY multiplies the number of target rows by the number of distinct values. Attributes that come from instrumentation, such as route templates, user agents, or anything containing ids, can explode. A raw URL path as a key is the classic mistake. Check that the values are bounded and normalized before adding a key.

Removing a key has the opposite effect: rows collapse, and consumers that filter on that key stop working. Also, the order of the sorting key in the target determines how rows merge. Changing it later means a new table and a migration.

## Target table engine and merging

The target engine decides what happens to rows with the same key. With an aggregating engine, merging happens in the background and eventually, so a read right after insert can see several rows per key. Readers must aggregate; they cannot assume a single row. With a summing engine, only certain columns combine, and the others keep an arbitrary value. Do not change the engine, the sorting key, or the partitioning without planning a table swap.

Partitioning affects retention and drop speed. A partition scheme that does not line up with the TTL makes cleanup expensive. Keep TTL on the target in step with what the regression summaries need to look back at; shortening it silently shrinks the baseline window.

## Source table schema drift

The view's SELECT is bound to the source columns at creation time and by name afterward. Renaming or dropping a source column, or changing its type, can break inserts into the source if the view can no longer evaluate. That means a rollup problem can block trace ingestion. Test schema changes on the source against the view first, and know how to detach the view quickly in an incident so ingestion continues.

Adding a column to the source does not add it to the view. That is usually what you want, but remember it when someone says the new attribute is not showing up in the rollup.

## Nulls, defaults, and status handling

Decide how failed spans, spans without a parent, and spans with a missing duration are treated. Including errors in the latency distribution skews it, since failures are often very fast or very slow. Excluding them hides regressions that show up as timeouts. Whatever the current choice is, a change to it is a change in meaning and should be treated like a change of bucket size. Check also that a zero or negative duration from clock problems is filtered the same way as before.

Unit handling is another trap: durations in the source may be stored in nanoseconds while dashboards display milliseconds. Keep the conversion in one place.

## Sampling

If the collectors sample traces, the rollup sees only sampled spans. Counts then understate real traffic, and the sampling policy may bias latency, for example when slow traces are kept preferentially. A change in the sampling policy upstream moves the rollup output without any change to the view. When a regression summary looks odd right after a collector change, look at sampling first. Do not add weighting to the view unless sampling rates are recorded alongside each span and trusted.

## Deployment on Kubernetes

The view definition lives in a migration, not in a pod. Know which job or init step applies it, and whether it is safe to run twice. Rolling deploys can leave old and new application versions running together, and both write spans with possibly different attribute names. The view has to cope with both during the overlap.

On a cluster with replicas and shards, the view must exist on every node that receives inserts, and the target must be consistent with the distributed layer in front of it. Creating it on only some nodes gives a partial rollup that depends on which node took the insert. Use the cluster-aware form of DDL and verify on each node afterward.

## Testing a change

Do not test only by checking that the query parses. Before shipping:

- Run the new SELECT by hand over a slice of real source data and compare with the existing target for the same slice.
- Compare counts first, then sums, then a few percentiles. Counts mismatching means the filter or keys differ.
- Try an empty window, a window with one span, and a window with late data.
- Run the regression summary against both versions for a deploy where the answer is known, and see that it still flags it.

Keep a copy of the old definition so you can go back. Reverting the view does not undo rows already written by the new one.

## Dashboards and consumers

Grafana panels and the summary job reference columns and sometimes the bucket size directly. Search for every consumer before renaming anything. A renamed column gives an empty panel, which an on-call engineer may read as no traffic. Alert rules built on the rollup are worse, since they may stop firing without notice. After any change, open the main panels and confirm they still show data for the recent past.

## Monitoring the view itself

There is no built-in signal that the view has fallen behind or stopped. Watch for the target stopping growth, the gap between the newest source row and the newest target row, insert errors on the source, and part counts on the target getting high. A detached or failing view looks like a quiet period. Make sure someone owns these checks.

## Before merging

Short checklist: is the meaning of a row unchanged, or is the change point written down; are aggregate states compatible; is cardinality bounded; do all readers still merge correctly; is the backfill plan explicit; is the rollout consistent on all nodes; is there a way back. If any answer is unknown, stop and find out. The view is cheap to change and expensive to get wrong, because the damage is in data that nobody revisits until a deploy looks bad.
