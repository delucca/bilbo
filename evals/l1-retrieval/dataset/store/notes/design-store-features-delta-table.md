---
id: 01KJJ7PB1CHVJCNP8R2DJZRFNY
created: 2026-02-28T10:41-03:00
sources:
  - "code: src/main/scala/shelfsense/features/FeatureTable.scala"
---

# store-features-delta design

The Delta table `shelfsense.store_features` in store-features-delta is partitioned by `store_id` and Z-ordered by `sku_id`. That is the core layout choice, and everything below follows from it. Partition column: `store_id`. Z-order column: `sku_id`.

## Naming

store-features-delta was called `featvault` before. The component is called store-features-delta now. Old docs, DAG names, dashboards or Slack threads may still say `featvault`; treat it as the same thing. Use store-features-delta in anything new.

## What it holds

It is the per-store feature layer for ShelfSense. Stockout models and the replenishment order generator read from it. Features are computed per store and per SKU, so the table is wide in rows and narrow in purpose.

## Table layout

The table `shelfsense.store_features` is partitioned by `store_id`. One partition directory per store. Inside each partition, files are Z-ordered by `sku_id` so that SKU lookups skip most files.

## Why partition by store_id

Analysts and jobs almost always work on one store or a small set of stores at a time. Partition pruning on `store_id` cuts the scan to a few directories. Chains have a bounded number of stores, so partition count stays manageable and does not explode into tiny files.

## Why Z-order by sku_id

Within a store, queries filter or join on `sku_id`. Z-ordering clusters rows for the same SKU, which lets Delta data skipping drop files by min/max stats. Partitioning by SKU instead would create far too many partitions.

## Writes

Spark jobs written in Scala do the writes, scheduled by Airflow. Writes go to Delta Lake with merge or append depending on the feature group. Keep writes aligned with the store partitioning so a job touches only the stores it needs.

## Maintenance

Z-ordering is not automatic on every write. It runs as part of table optimization, scheduled after the main feature loads. If skipping looks poor, check that the last optimize finished before blaming the layout.

## Reads

Readers should always filter on `store_id` first, then on `sku_id` when they can. A full table scan works but is slow and defeats the layout.

## Downstream

Some consumers copy slices into Snowflake for analyst use. Those copies are derived; the Delta table is the source of truth.

## Orchestration

Airflow owns the schedule and ordering: feature load, then optimize, then downstream exports. A failed load should block the optimize and exports.

## Small files

Many small writes per store can leave small files in a partition. Optimize compacts them while it Z-orders. Avoid frequent tiny appends.

## Schema changes

Add columns rather than changing types. Delta handles added columns with schema evolution, but readers in Scala should be updated together with the write.

## Gotchas

Do not change the partition column casually. Repartitioning means rewriting the whole table. Do not search only for `featvault` when tracing history; search for both names.

## Open questions

Whether a very large store set would justify a different partition scheme is not settled. Nothing suggests it is needed now.

## Summary of layout

`shelfsense.store_features`: partitioned by `store_id`, Z-ordered by `sku_id`, owned by store-features-delta, formerly `featvault`.
