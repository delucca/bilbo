---
id: 01K1GVMVTJ6BS4KPV70RWBYD7D
created: 2025-07-31T15:24-03:00
---

# shelf-metrics-lib: service_level plan for 1.5.0

The shelf-metrics-lib will add a `service_level` metric in release 1.5.0. This note is the working plan for getting it in: what it is for, what has to change around it, and what is still open. Nothing here is built yet. Details that are not settled are written as general questions, not as values.

## Goal

Merchandising analysts using ShelfSense currently see stockout predictions and replenishment orders, but the library that computes shelf-level numbers has no direct measure of how often demand was actually met. `service_level` fills that gap. The metric ships in release 1.5.0 of shelf-metrics-lib and not before.

## Why this release

Analysts keep asking for a number they can compare across stores and categories without reading raw stockout counts. A single service measure also gives the replenishment side something to tune against. Putting it in 1.5.0 keeps it in one library release rather than scattered across jobs.

## Scope of the first version

Keep the first version small. One metric, computed per store and per product over a time window the caller chooses. It reads the same inputs the other metrics in shelf-metrics-lib already read, so no new source tables should be needed. Anything that needs new inputs waits for a later release.

## Out of scope

No new dashboards in this release. No change to how replenishment orders are generated; the orders will not consume `service_level` yet. No backfill of historical Snowflake tables beyond what a normal rerun does.

## Definition work

The exact definition of `service_level` has to be written down before code starts. Open questions: how to treat days when a product was not ranged at a store, how to treat partial days, and whether the measure is based on units or on order lines. Decide with the merchandising analysts, then record the answer in this note or in a decision note, and link it here.

## Library changes

Add the metric alongside the existing ones in the Scala code of shelf-metrics-lib, following the same shape as the other Spark-based metrics: a function that takes a DataFrame and returns a DataFrame with the metric column. Keep naming consistent with the existing columns. Expose it through the same public entry point so callers do not need new imports beyond the version bump.

## Delta Lake considerations

Output tables stored in Delta Lake will get one extra column for `service_level`. Adding a nullable column is a schema evolution step; check whether the jobs that write those tables have schema merge enabled, and if not, plan the change explicitly instead of relying on it silently. Older rows will have nulls until they are recomputed.

## Airflow

The Airflow DAGs that call shelf-metrics-lib need the new version pinned. Do the bump in a separate change from the metric work so it can be rolled back alone. Check whether any DAG selects metric columns by an explicit list; those would need the new column added by hand.

## Snowflake

Downstream tables in Snowflake that mirror the Delta outputs need the new column before the first load that includes it, or the load will fail or drop data depending on how it is configured. Coordinate with whoever owns those tables. Views used by analysts should be updated after the column exists and has real data.

## Testing

Unit tests for the metric on small hand-built DataFrames: a store with no stockouts, a store with a long stockout, a product with no demand, and a product not ranged for part of the window. Add one integration run on a sample of real data and compare results by hand against what an analyst expects for a few known cases.

## Compatibility

Existing callers must keep working without code changes. The new column should only appear where the caller asks for it, or be added at the end of the output so positional readers are not broken. Note this in the release notes for 1.5.0.

## Rollout order

First finish the definition. Then the library change and tests. Then cut 1.5.0. Then prepare Delta and Snowflake schemas. Last, bump the pin in Airflow and let one DAG run before the rest.

## Risks

The definition may shift after analysts see first numbers, which would mean recomputing. Sparse data for slow-moving products may give noisy values. Schema changes in Snowflake may lag behind the library release.

## Open items

Who signs off on the definition. Who owns the Snowflake tables. Whether the replenishment logic should use `service_level` in a later release, and what that would need from this metric.
