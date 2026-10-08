---
id: 01KV27PYMHVDTXY5ZF81HKR7AQ
created: 2026-06-14T01:54-03:00
---

# replenish-orders-job runtime after repartitioning

This note replaces the earlier note about "replenish orders september produced"; the new value for the replenish-orders-job runtime is 9 minutes, measured after repartitioning by `store_id`. The earlier measurement is out of date and should not be quoted.

## What changed

The replenish-orders-job now repartitions its input by `store_id` before the stage that builds the replenishment orders. Before this, the data reached the order-building stage with whatever partitioning the upstream read left it with. A few tasks ended up with most of a chain's stores and the rest sat idle, so the stage waited on the slowest tasks.

With the data split by `store_id`, each task handles a bounded set of stores and the work is spread more evenly. The runtime fell to 9 minutes. That figure is the current number for replenish-orders-job and replaces the earlier one.

## Why store_id

Everything the job computes is per store: the stockout prediction is read per store, and the orders are generated per store. Grouping rows by `store_id` up front means later per-store aggregations and joins do not need another shuffle to bring a store's rows together. That is the main reason the change helps, beyond just balancing task sizes.

It is also the natural key for the output. Analysts look at orders store by store, so the written Delta Lake files line up with how the data is read afterwards.

## How it was measured

The 9 minutes is a wall-clock time for a full run of replenish-orders-job as scheduled by Airflow, not a single Spark stage. It covers reading the inputs, the repartition, building the orders and writing the result. It was taken on one run, so treat it as a good single data point and not an average. If the number matters for a decision, check a few more Airflow runs first.

## Caveats

- Skew by `store_id` is still possible. A very large store could still make one task slower than the others. I have not seen this hurt the runtime so far.
- The number of partitions was not tuned separately. Most of the gain seems to come from the key, not the count.
- The job reads from Delta Lake and the downstream consumers load results into Snowflake. I did not check whether the load side changed in any way.
- Input volume varies from day to day, so the runtime will move a bit even with no code change.

## Things to do next

- Compare a few more scheduled runs against the 9 minutes and see how much they vary.
- Look at the Spark UI task times for the order-building stage to confirm skew is gone, not just reduced.
- If a single large store dominates, consider salting or splitting that store's work.
- Update any dashboards or docs that still show the earlier runtime.

## Related

Input options for sales data are compared in [[sales-ingest-options-survey]]. That note is about getting data in; this one is only about the runtime of replenish-orders-job once the data is there.
