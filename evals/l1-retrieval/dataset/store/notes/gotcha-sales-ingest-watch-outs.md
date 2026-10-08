---
id: 01KHREDQTMJTYEHTGABSZQAWM9
created: 2026-02-18T10:18-03:00
---

# sales-ingest-pipeline: things to watch when changing it

Notes for anyone touching sales-ingest-pipeline. Nothing here is a spec; it is the list of places where changes tend to go wrong. Downstream stockout predictions and replenishment orders depend on this component being boringly correct, so a small slip here shows up later as bad orders for analysts.

## Why it is sensitive

sales-ingest-pipeline sits at the front of everything. Forecasts, stockout scores and order suggestions all read what it writes. Errors rarely crash anything. They produce plausible but wrong numbers, and nobody notices until a store is empty or overstocked.

## Late and out-of-order data

Store sales arrive late, twice, or out of order. Any change to ordering, watermarks or dedup logic has to be tested against delayed files and replays. Do not assume the latest file holds the latest sales.

## Idempotency

A rerun of a day or a task must give the same table state. Appends without a merge key double count sales. Check that a retry from Airflow, a manual backfill and a normal run all end in the same result.

## Merge keys and duplicates

Changing the key used for merges or dedup changes what counts as the same sale. Returns, voids and corrections look like duplicates but are not. Think about them before tightening a key.

## Schema changes

Source feeds from chains do not change in sync. Adding, renaming or retyping a column can break the Spark job, the Delta table, or the Snowflake load, each in a different way. Look at all three before merging.

## Delta Lake schema evolution

Automatic schema evolution is convenient and also hides mistakes. A misspelled column becomes a new column quietly. Be deliberate about when evolution is allowed.

## Partitioning

Partition layout was chosen for how downstream reads happen. Changing it can make reads slow or small files pile up. Compaction and vacuum behaviour also depend on it, so check retention before and after.

## Time zones and business dates

Stores sit in different time zones, and a sale near midnight belongs to a business day, not a UTC day. Mixing the two shifts sales between days and breaks daily stockout signals. Daylight saving days are a classic trap.

## Units and product identifiers

Quantities come as units, weights or cases depending on the chain. Product identifiers get remapped over time. A change to normalization can silently reshuffle history, so compare before and after on a sample of stores.

## Nulls and defaults

Do not fill missing quantities with zero to make a job pass. Zero sales and no data mean different things to the model: one says the item sold nothing, the other says we do not know. Keep that distinction.

## Spark performance

Skew on big stores or popular items is normal. A new join or aggregation can turn one task into the whole job. Watch shuffle size and broadcast assumptions when adding joins, and test with a realistic data shape rather than a tiny sample.

## Scala code and types

Case class changes ripple into encoders and into any job that reads the same tables. Prefer compile-time breakage over runtime surprises, and avoid loosening types to get something through.

## Airflow scheduling and dependencies

Downstream DAGs assume this one finishes by a certain point. Adding retries, new tasks or longer runtimes can push that out. Check sensors, dependencies and catchup settings when changing the schedule or task layout.

## Snowflake load

The load into Snowflake is its own step with its own failure modes: type mapping, truncation, permissions and partial loads. A job that succeeds in Spark can still leave Snowflake half updated. Verify row counts match between the two sides.

## Backfills

Backfilling over history can overwrite data analysts already rely on, and can trigger recomputation downstream. Tell the downstream owners first, and prefer writing to a side table and comparing before swapping.

## Testing and validation

Unit tests on small frames miss most of the above. Compare outputs against the old version on real recent data, and check totals per store and per day. Keep data quality checks in place when refactoring; do not remove one because it looks redundant.

## Rollback

Delta history makes rollback possible, but only inside the retention window and only if nobody vacuumed. Know how you would undo a change before you ship it, and remember that downstream tables derived from bad data need fixing too.
