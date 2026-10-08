---
id: 01JRX2GC7C4YPAGB36FH28BAHT
created: 2025-04-15T12:53-03:00
---

# replenish-orders-job weekly recap

Most of the week went into replenish-orders-job: reading how it behaves end to end, fixing a few rough spots, and writing down what is still unclear. Nothing here is final. It is a snapshot so the next session does not start cold.

## Where things stand

replenish-orders-job still runs as a Spark batch in Scala, scheduled from Airflow, reading stockout predictions from Delta Lake and writing proposed orders out for analysts. The overall shape did not change this week. Work was mostly inside the order-building step and around its inputs.

## What got done

Cleaned up the step that turns predicted stockout risk into order quantities. The logic was spread over a few helpers that repeated the same rounding and pack-size handling. Pulled that into one place so the rules are readable in one pass.

Tightened how the job handles stores with missing or late prediction data. Before, a gap could quietly produce an empty order set for that store. Now it is logged clearly and the store is skipped on purpose.

## Problems seen

Some runs were slower than expected on the join between predictions and current inventory. It looks like skew on a few very busy stores and items, but this is a guess until someone checks the Spark UI on a real run.

Analysts raised that a few suggested orders looked odd for items with irregular case sizes. Not reproduced yet. It may come from the rounding path I just consolidated, or from bad reference data upstream.

## Data and Snowflake side

Order output still lands in Snowflake for the analysts' tools. Column naming between the Delta tables and the Snowflake tables is inconsistent in a couple of places. It works today because of manual mapping in the load step, which is fragile. Worth cleaning up, but not urgent.

## Airflow

The DAG for replenish-orders-job retries on failure, but the retry settings were chosen early and nobody has revisited them against how often the upstream prediction job finishes late. Need to look at whether the job should wait on that dependency more explicitly instead of retrying blindly.

## Testing

Added a few unit tests around the order-building helpers, using small in-memory frames. Still no good end-to-end test that runs against realistic data volumes. That gap is the main reason the slow-join question is hard to settle.

## Open questions

- Is the odd-order behavior from our code or from the input data?
- Is the skew real, and would salting or a broadcast help?
- Should the Airflow dependency on predictions be made stricter?
- Who owns the column naming cleanup between Delta and Snowflake?

## Next week

Reproduce the odd case-size orders first, since analysts feel that one directly. Then profile a real run to confirm or drop the skew idea. After that, review the DAG dependency and retry setup. Keep the consolidated rounding logic under watch until the first two items are settled.
